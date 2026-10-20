import "./style.css";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { open, save, confirm } from "@tauri-apps/plugin-dialog";

const element = (id) => document.getElementById(id);
const desktop = isTauri();
const formats = {
  1: "RGBA bitmap",
  2: "RGB bitmap",
  3: "Luminance + alpha",
  4: "Luminance",
  5: "Alpha",
  6: "Wavelet RGBA",
  7: "Wavelet RGB",
  8: "Wavelet luminance + alpha",
  9: "Wavelet luminance",
  10: "Wavelet alpha",
  11: "DXT1",
  12: "DXT3",
  13: "DXT5",
};
const filters = [
  "search",
  "archive",
  "dimensions",
  "shape",
  "type",
  "format",
  "alpha",
  "duplicates",
];
let entries = [];
let groups = new Map();
let visible = [];
let selected = null;
let selectedGroup = [];
let scanBusy = false;
let previewGeneration = 0;
let previewPixels = null;
let zoom = 1;
let fitMode = true;
let animationFrame = 0;

// Only visible cards enter this queue. Memory and simultaneous IPC requests stay bounded.
const thumbnails = new Map();
const thumbnailErrors = new Map();
const inFlight = new Set();
let thumbnailQueue = [];
let cacheGeneration = 0;

function message(text, error = false) {
  element("message").textContent = text;
  element("message").className = error ? "error" : "success";
}

function option(select, value, label) {
  const item = document.createElement("option");
  item.value = value;
  item.textContent = label;
  select.append(item);
}

function fillSelect(id, values, label = (value) => value) {
  const select = element(id);
  const previous = select.value;
  select.replaceChildren(select.options[0]);
  for (const value of values) {
    option(select, value, label(value));
  }
  select.value = previous;
  if (select.selectedIndex < 0) {
    select.selectedIndex = 0;
  }
}

function matches(entry) {
  const info = entry.header;
  const archive = element("archive").value;
  const dimensions = element("dimensions").value;
  const shape = element("shape").value;
  const type = element("type").value;
  const format = element("format").value;
  const alpha = element("alpha").value;
  if (archive && !entry.sources.some((source) => source.archive === archive)) {
    return false;
  }
  if (dimensions && dimensions !== `${info.width}×${info.height}`) {
    return false;
  }
  if (format && Number(format) !== info.format) {
    return false;
  }
  if (
    (type === "cube" && info.faces !== 6) ||
    (type === "2d" && info.faces !== 1)
  ) {
    return false;
  }
  if ((alpha === "yes" && !info.alpha) || (alpha === "no" && info.alpha)) {
    return false;
  }
  if (shape === "square" && info.width !== info.height) {
    return false;
  }
  if (shape === "wide" && info.width <= info.height) {
    return false;
  }
  if (shape === "tall" && info.width >= info.height) {
    return false;
  }
  return true;
}

function applyFilters() {
  const query = element("search").value.trim().toLowerCase();
  const duplicates = element("duplicates").value;
  visible = [];
  for (const [name, versions] of groups) {
    if (!name.includes(query)) {
      continue;
    }
    if (duplicates === "different" && versions.length < 2) {
      continue;
    }
    if (
      duplicates === "copies" &&
      versions.reduce((count, item) => count + item.sources.length, 0) < 2
    ) {
      continue;
    }
    const matching = versions.filter(matches);
    if (matching.length > 0) {
      visible.push({ name, versions: matching });
    }
  }
  visible.sort((a, b) => a.name.localeCompare(b.name));
  element("count").textContent =
    `${visible.length.toLocaleString()} of ${groups.size.toLocaleString()} filenames`;
  element("empty-grid").textContent = "No images match these filters.";
  element("empty-grid").hidden = visible.length > 0;
  element("grid-scroll").scrollTop = 0;
  renderGrid();
}

function scheduleGrid() {
  if (animationFrame) {
    return;
  }
  animationFrame = requestAnimationFrame(() => {
    animationFrame = 0;
    renderGrid();
  });
}

function renderGrid() {
  const scroll = element("grid-scroll");
  const grid = element("grid");
  const columns = Math.max(1, Math.floor(scroll.clientWidth / 166));
  const width = scroll.clientWidth / columns;
  const rowHeight = 190;
  const firstRow = Math.max(0, Math.floor(scroll.scrollTop / rowHeight) - 1);
  const lastRow =
    Math.ceil((scroll.scrollTop + scroll.clientHeight) / rowHeight) + 1;
  grid.style.height = `${Math.ceil(visible.length / columns) * rowHeight}px`;
  const cards = document.createDocumentFragment();
  thumbnailQueue = [];
  for (
    let index = firstRow * columns;
    index < Math.min(visible.length, lastRow * columns);
    index += 1
  ) {
    const group = visible[index];
    const entry = group.versions[0];
    const card = document.createElement("button");
    card.className = "image-card";
    card.classList.toggle(
      "selected",
      selected?.name.toLowerCase() === group.name,
    );
    card.style.left = `${(index % columns) * width + 5}px`;
    card.style.top = `${Math.floor(index / columns) * rowHeight + 5}px`;
    card.style.width = `${width - 10}px`;
    card.title = entry.name;
    card.dataset.id = entry.id;
    card.setAttribute("aria-label", `Preview ${entry.name}`);
    const image = document.createElement("img");
    image.alt = "";
    if (thumbnails.has(entry.id)) {
      image.src = thumbnails.get(entry.id);
    } else if (!inFlight.has(entry.id) && !thumbnailErrors.has(entry.id)) {
      thumbnailQueue.push(entry.id);
    }
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = entry.name.replace(/^images\//i, "");
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = thumbnailErrors.has(entry.id)
      ? "Preview unavailable"
      : `${entry.header.width} × ${entry.header.height}${group.versions.length > 1 ? ` · ${group.versions.length} versions` : ""}`;
    card.append(image, name, badge);
    card.addEventListener("click", () => selectGroup(group));
    cards.append(card);
  }
  grid.replaceChildren(cards);
  pumpThumbnails();
}

function pumpThumbnails() {
  if (!desktop || scanBusy) {
    return;
  }
  while (inFlight.size < 3 && thumbnailQueue.length > 0) {
    const id = thumbnailQueue.shift();
    if (inFlight.has(id) || thumbnails.has(id) || thumbnailErrors.has(id)) {
      continue;
    }
    inFlight.add(id);
    const generation = cacheGeneration;
    invoke("thumbnail", { id })
      .then((url) => {
        if (generation !== cacheGeneration) {
          return;
        }
        thumbnails.set(id, url);
        if (thumbnails.size > 256) {
          thumbnails.delete(thumbnails.keys().next().value);
        }
        const image = element("grid").querySelector(`[data-id="${id}"] img`);
        if (image) {
          image.src = url;
        }
      })
      .catch((error) => {
        if (generation === cacheGeneration) {
          thumbnailErrors.set(id, String(error));
          const badge = element("grid").querySelector(
            `[data-id="${id}"] .badge`,
          );
          if (badge) {
            badge.textContent = "Preview unavailable";
          }
        }
      })
      .finally(() => {
        inFlight.delete(id);
        if (generation !== cacheGeneration) {
          scheduleGrid();
        }
        pumpThumbnails();
      });
  }
}

function selectGroup(group) {
  selectedGroup = group.versions;
  element("version").replaceChildren();
  for (const [index, entry] of selectedGroup.entries()) {
    option(
      element("version"),
      entry.id,
      `${index + 1}: ${entry.sources.map((source) => source.archive).join(", ")}`,
    );
  }
  element("version-label").hidden = selectedGroup.length < 2;
  selectEntry(selectedGroup[0]);
}

function selectEntry(entry) {
  selected = entry;
  element("empty-preview").hidden = true;
  element("selection").hidden = false;
  element("filename").textContent = entry.name.replace(/^images\//i, "");
  element("image-info").textContent =
    `${entry.header.width} × ${entry.header.height} · ${formats[entry.header.format]} · ${entry.header.faces === 6 ? "Cubemap" : "2D image"}`;
  element("sources").textContent = entry.sources
    .map((source) => `${source.archive}\n${source.entry}`)
    .join("\n\n");
  element("face").hidden = entry.header.faces !== 6;
  element("face").value = "0";
  fitMode = true;
  renderGrid();
  loadPreview();
}

async function loadPreview() {
  if (!selected || !desktop) {
    return;
  }
  const generation = ++previewGeneration;
  const entry = selected;
  previewPixels = null;
  element("canvas").hidden = true;
  element("preview-loading").hidden = false;
  element("preview-loading").textContent = "Loading…";
  try {
    const bytes = await invoke("pixels", {
      id: entry.id,
      face: Number(element("face").value),
    });
    if (generation !== previewGeneration) {
      return;
    }
    previewPixels = new Uint8ClampedArray(bytes);
    if (previewPixels.length !== entry.header.width * entry.header.height * 4) {
      throw new Error("The preview returned an unexpected pixel count.");
    }
    drawPreview();
    element("status").textContent = "Ready";
  } catch (error) {
    if (generation === previewGeneration) {
      element("preview-loading").textContent = "Preview unavailable.";
      message(String(error), true);
    }
  }
}

function drawPreview() {
  if (!previewPixels || !selected) {
    return;
  }
  const canvas = element("canvas");
  canvas.width = selected.header.width;
  canvas.height = selected.header.height;
  const pixels = new Uint8ClampedArray(previewPixels);
  const channel = element("channel").value;
  const channelIndex = { r: 0, g: 1, b: 2, a: 3 }[channel];
  for (let offset = 0; offset < pixels.length; offset += 4) {
    if (channelIndex !== undefined) {
      const value = pixels[offset + channelIndex];
      pixels[offset] = value;
      pixels[offset + 1] = value;
      pixels[offset + 2] = value;
    }
    if (channel !== "rgba") {
      pixels[offset + 3] = 255;
    }
  }
  canvas
    .getContext("2d")
    .putImageData(new ImageData(pixels, canvas.width, canvas.height), 0, 0);
  canvas.hidden = false;
  element("preview-loading").hidden = true;
  if (fitMode) {
    fitPreview();
  } else {
    applyZoom();
  }
}

function applyZoom() {
  if (!selected) {
    return;
  }
  element("canvas").style.width = `${selected.header.width * zoom}px`;
  element("canvas").style.height = `${selected.header.height * zoom}px`;
  element("zoom-value").textContent = `${Math.round(zoom * 100)}%`;
}

function fitPreview() {
  if (!selected) {
    return;
  }
  const area = element("preview-scroll");
  zoom = Math.min(
    1,
    Math.max(0.01, (area.clientWidth - 4) / selected.header.width),
    Math.max(0.01, (area.clientHeight - 4) / selected.header.height),
  );
  fitMode = true;
  applyZoom();
}

async function scan(force = false) {
  if (!desktop || scanBusy) {
    return;
  }
  scanBusy = true;
  element("rescan").disabled = true;
  element("clear-cache").disabled = true;
  element("status").textContent = "Reading archives…";
  message("Reading the IWD image index. The first scan takes longer.");
  try {
    const result = await invoke("scan", { force });
    entries = result.entries;
    groups = new Map();
    for (const entry of entries) {
      const name = entry.name.toLowerCase();
      if (!groups.has(name)) {
        groups.set(name, []);
      }
      groups.get(name).push(entry);
    }
    for (const versions of groups.values()) {
      versions.sort((a, b) =>
        a.sources[0].archive.localeCompare(b.sources[0].archive),
      );
    }
    fillSelect(
      "archive",
      [
        ...new Set(
          entries.flatMap((entry) =>
            entry.sources.map((source) => source.archive),
          ),
        ),
      ].sort(),
    );
    fillSelect(
      "dimensions",
      [
        ...new Set(
          entries.map(
            (entry) => `${entry.header.width}×${entry.header.height}`,
          ),
        ),
      ].sort((a, b) => parseInt(a, 10) - parseInt(b, 10) || a.localeCompare(b)),
    );
    fillSelect(
      "format",
      [...new Set(entries.map((entry) => entry.header.format))].sort(
        (a, b) => a - b,
      ),
      (value) => formats[value],
    );
    element("cache-path").value = result.cache_path;
    element("warnings").hidden = result.warnings.length === 0;
    element("warnings-title").textContent =
      `${result.warnings.length} indexing warnings`;
    element("warning-text").textContent = result.warnings.join("\n");
    selected = null;
    previewPixels = null;
    previewGeneration += 1;
    element("selection").hidden = true;
    element("empty-preview").hidden = false;
    cacheGeneration += 1;
    thumbnails.clear();
    thumbnailErrors.clear();
    applyFilters();
    message(
      `${entries.length.toLocaleString()} image versions indexed. Select a thumbnail to inspect it.`,
      result.warnings.length > 0,
    );
    element("status").textContent =
      result.warnings.length > 0 ? "Done with warnings" : "Ready";
  } catch (error) {
    message(String(error), true);
    element("status").textContent = "Could not read archives";
    element("empty-grid").textContent =
      "Could not read images. See the message below, then click Rescan to retry.";
  } finally {
    scanBusy = false;
    element("rescan").disabled = false;
    element("clear-cache").disabled = false;
    pumpThumbnails();
  }
}

async function exportImage(kind) {
  if (!desktop || !selected) {
    return;
  }
  const entry = selected;
  const filename = entry.name
    .split("/")
    .pop()
    .replace(/\.iwi$/i, "");
  try {
    let destination;
    if (kind === "png" && entry.header.faces === 6) {
      destination = await open({
        directory: true,
        multiple: false,
        title: "Save six cubemap PNG faces here",
      });
    } else {
      destination = await save({
        defaultPath: `${filename}.${kind}`,
        filters: [{ name: kind.toUpperCase(), extensions: [kind] }],
      });
    }
    if (!destination) {
      return;
    }
    const request = { id: entry.id, destination, kind };
    const conflicts = await invoke("export_conflicts", request);
    let replace = false;
    if (conflicts.length > 0) {
      replace = await confirm(
        `Replace these existing files?\n\n${conflicts.join("\n")}`,
        { title: "Replace exported files", kind: "warning" },
      );
      if (!replace) {
        return;
      }
    }
    element("status").textContent = "Saving…";
    const saved = await invoke("export_image", { ...request, replace });
    message(`Saved: ${saved.join(" · ")}`);
    element("status").textContent = "Done";
  } catch (error) {
    message(String(error), true);
    element("status").textContent = "Export failed";
  }
}

for (const id of filters) {
  element(id).addEventListener(
    id === "search" ? "input" : "change",
    applyFilters,
  );
}
element("reset-filters").addEventListener("click", () => {
  for (const id of filters) {
    element(id).value = "";
  }
  applyFilters();
});
element("grid-scroll").addEventListener("scroll", scheduleGrid);
new ResizeObserver(scheduleGrid).observe(element("grid-scroll"));
new ResizeObserver(() => {
  if (fitMode) {
    fitPreview();
  }
}).observe(element("preview-scroll"));
element("rescan").addEventListener("click", () => {
  scan(true);
});
element("version").addEventListener("change", () => {
  selectEntry(
    selectedGroup.find((entry) => entry.id === element("version").value),
  );
});
element("face").addEventListener("change", loadPreview);
element("channel").addEventListener("change", drawPreview);
element("fit").addEventListener("click", fitPreview);
element("actual").addEventListener("click", () => {
  fitMode = false;
  zoom = 1;
  applyZoom();
});
element("preview-scroll").addEventListener(
  "wheel",
  (event) => {
    if (!previewPixels) {
      return;
    }
    event.preventDefault();
    fitMode = false;
    zoom = Math.max(
      0.01,
      Math.min(16, zoom * (event.deltaY < 0 ? 1.2 : 1 / 1.2)),
    );
    applyZoom();
  },
  { passive: false },
);
let drag = null;
element("preview-scroll").addEventListener("pointerdown", (event) => {
  if (event.button !== 0) {
    return;
  }
  const area = element("preview-scroll");
  drag = {
    x: event.clientX,
    y: event.clientY,
    left: area.scrollLeft,
    top: area.scrollTop,
  };
  area.setPointerCapture(event.pointerId);
});
element("preview-scroll").addEventListener("pointermove", (event) => {
  if (drag) {
    element("preview-scroll").scrollLeft = drag.left - event.clientX + drag.x;
    element("preview-scroll").scrollTop = drag.top - event.clientY + drag.y;
  }
});
for (const event of ["pointerup", "pointercancel", "lostpointercapture"]) {
  element("preview-scroll").addEventListener(event, () => {
    drag = null;
  });
}
element("copy-name").addEventListener("click", async () => {
  if (!selected) {
    return;
  }
  try {
    await navigator.clipboard.writeText(
      selected.name.replace(/^images\//i, "").replace(/\.iwi$/i, ""),
    );
    message("Image name copied without the images/ prefix or .iwi extension.");
    element("status").textContent = "Done";
  } catch (error) {
    message(`Could not copy the name: ${error}`, true);
  }
});
element("export-iwi").addEventListener("click", () => {
  exportImage("iwi");
});
element("export-png").addEventListener("click", () => {
  exportImage("png");
});
element("about").addEventListener("click", () => {
  element("about-dialog").showModal();
  element("repo-url").focus();
  element("repo-url").select();
});
element("close-about").addEventListener("click", () => {
  element("about-dialog").close();
});
element("repo-url").addEventListener("click", (event) => {
  event.target.select();
});
element("cache-button").addEventListener("click", () => {
  element("cache-dialog").showModal();
});
element("close-cache").addEventListener("click", () => {
  element("cache-dialog").close();
});
element("open-cache").addEventListener("click", async () => {
  try {
    await invoke("open_cache");
  } catch (error) {
    message(String(error), true);
  }
});
element("clear-cache").addEventListener("click", async () => {
  if (!desktop || scanBusy) {
    return;
  }
  scanBusy = true;
  element("clear-cache").disabled = true;
  thumbnailQueue = [];
  cacheGeneration += 1;
  try {
    await invoke("clear_cache");
    thumbnails.clear();
    thumbnailErrors.clear();
    message("Cache cleared. Visible thumbnails will be rebuilt as needed.");
    element("status").textContent = "Done";
  } catch (error) {
    message(String(error), true);
  } finally {
    scanBusy = false;
    element("clear-cache").disabled = false;
    element("cache-dialog").close();
    renderGrid();
  }
});
element("startup-error").addEventListener("cancel", (event) => {
  event.preventDefault();
});
element("quit").addEventListener("click", () => {
  invoke("quit");
});

async function start() {
  element("environment").textContent = import.meta.env.DEV
    ? "Development"
    : "Production";
  try {
    if (desktop) {
      await invoke("startup");
    }
    element("app").hidden = false;
    element("footer").hidden = false;
    if (desktop) {
      await scan();
    } else {
      element("status").textContent = "UI preview";
      element("count").textContent = "No game connection";
      element("empty-grid").textContent =
        "Run npm run dev to browse your game images.";
      message(
        "Browser preview only. Image reading and exports require the desktop app.",
      );
      for (const id of ["rescan", "cache-button"]) {
        element(id).disabled = true;
      }
    }
  } catch (error) {
    element("startup-message").textContent = String(error);
    element("startup-error").showModal();
  }
}

start();
