//! Archive indexing, thumbnail caching and explicit exports. Game archives are read-only.
use crate::iwi::{self, Header};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{DynamicImage, ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const CACHE_VERSION: &str = "iwi-v1";
const MAX_IWI_BYTES: u64 = 128 * 1024 * 1024;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Source {
    pub archive: String,
    pub entry: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub hash: String,
    pub header: Header,
    pub sources: Vec<Source>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Index {
    pub entries: Vec<Entry>,
    pub warnings: Vec<String>,
    pub cache_path: String,
}

pub struct Catalog {
    pub root: PathBuf,
    pub cache: PathBuf,
    pub index: Index,
}

fn archive_names(root: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(root.join("main")).map_err(|e| format!("Cannot read main: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("iwd"))
        {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    if names.is_empty() {
        return Err("No IWD archives found in the game's main folder.".into());
    }
    Ok(names)
}

fn archive_key(root: &Path, names: &[String]) -> Result<String, String> {
    let mut stamp = format!("{CACHE_VERSION}|{}", root.display());
    for name in names {
        let metadata = fs::metadata(root.join("main").join(name)).map_err(|e| e.to_string())?;
        let modified = metadata
            .modified()
            .map_err(|e| e.to_string())?
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        stamp.push_str(&format!("|{name}:{}:{modified}", metadata.len()));
    }
    Ok(digest(stamp.as_bytes()))
}

fn write_atomic(path: &Path, bytes: &[u8], replace: bool) -> Result<(), String> {
    let parent = path.parent().ok_or("Destination has no parent folder.")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    if replace {
        file.persist(path).map_err(|e| e.to_string())?;
    } else {
        file.persist_noclobber(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl Catalog {
    pub fn new(root: PathBuf, cache: PathBuf) -> Self {
        let index = Index {
            entries: Vec::new(),
            warnings: Vec::new(),
            cache_path: cache.display().to_string(),
        };
        Self { root, cache, index }
    }

    pub fn scan(&mut self, force: bool) -> Result<Index, String> {
        fs::create_dir_all(self.cache.join("thumbnails")).map_err(|e| e.to_string())?;
        let names = archive_names(&self.root)?;
        let key = archive_key(&self.root, &names)?;
        let index_path = self.cache.join(format!("index-{key}.json"));
        if !force {
            if let Ok(bytes) = fs::read(&index_path) {
                if let Ok(mut index) = serde_json::from_slice::<Index>(&bytes) {
                    index.cache_path = self.cache.display().to_string();
                    self.index = index;
                    return Ok(self.index.clone());
                }
            }
        }
        let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
        let mut warnings = Vec::new();
        for name in names {
            let result = self.scan_archive(&name, &mut entries, &mut warnings);
            if let Err(error) = result {
                warnings.push(format!("{name}: {error}"));
            }
        }
        self.index = Index {
            entries: entries.into_values().collect(),
            warnings,
            cache_path: self.cache.display().to_string(),
        };
        // An incomplete scan is shown to the user, but never saved as a complete index.
        if self.index.warnings.is_empty() {
            let bytes = serde_json::to_vec(&self.index).map_err(|e| e.to_string())?;
            write_atomic(&index_path, &bytes, true)?;
        }
        Ok(self.index.clone())
    }

    fn scan_archive(
        &self,
        name: &str,
        entries: &mut BTreeMap<String, Entry>,
        warnings: &mut Vec<String>,
    ) -> Result<(), String> {
        let file = File::open(self.root.join("main").join(name)).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        for position in 0..archive.len() {
            let mut member = archive.by_index(position).map_err(|e| e.to_string())?;
            let entry_name = member.name().to_string();
            let lower = entry_name.to_lowercase();
            if !lower.starts_with("images/") || !lower.ends_with(".iwi") || member.is_dir() {
                continue;
            }
            let result = (|| {
                if member.size() > MAX_IWI_BYTES {
                    return Err("Image exceeds the supported size limit.".to_string());
                }
                let mut data = Vec::new();
                member
                    .by_ref()
                    .take(MAX_IWI_BYTES + 1)
                    .read_to_end(&mut data)
                    .map_err(|e| e.to_string())?;
                if data.len() as u64 > MAX_IWI_BYTES {
                    return Err("Image exceeds the supported size limit.".to_string());
                }
                let header = iwi::header(&data)?;
                let hash = digest(&data);
                let id = digest(format!("{lower}:{hash}").as_bytes());
                let source = Source {
                    archive: name.to_string(),
                    entry: entry_name.clone(),
                };
                if let Some(existing) = entries.get_mut(&id) {
                    existing.sources.push(source);
                } else {
                    entries.insert(
                        id.clone(),
                        Entry {
                            id,
                            name: entry_name.clone(),
                            hash,
                            header,
                            sources: vec![source],
                        },
                    );
                }
                Ok::<(), String>(())
            })();
            if let Err(error) = result {
                warnings.push(format!("{name}/{entry_name}: {error}"));
            }
        }
        Ok(())
    }

    pub fn entry(&self, id: &str) -> Result<&Entry, String> {
        self.index
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| "Image is no longer in the index. Rescan the archives.".into())
    }

    pub fn read(&self, id: &str) -> Result<Vec<u8>, String> {
        let entry = self.entry(id)?;
        let source = entry
            .sources
            .first()
            .ok_or("Image has no archive source.")?;
        // Cached metadata cannot redirect archive reads outside main.
        if Path::new(&source.archive).components().count() != 1
            || !source.archive.to_lowercase().ends_with(".iwd")
        {
            return Err("Invalid cached archive name. Clear the cache and rescan.".into());
        }
        let file =
            File::open(self.root.join("main").join(&source.archive)).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let member = archive.by_name(&source.entry).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        member
            .take(MAX_IWI_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_IWI_BYTES || digest(&bytes) != entry.hash {
            return Err("The archive image changed. Click Rescan before continuing.".into());
        }
        Ok(bytes)
    }

    pub fn thumbnail(&self, id: &str) -> Result<String, String> {
        let entry = self.entry(id)?;
        let path = self
            .cache
            .join("thumbnails")
            .join(format!("{}.png", entry.hash));
        if let Ok(bytes) = fs::read(&path) {
            if image::load_from_memory(&bytes).is_ok() {
                return Ok(format!("data:image/png;base64,{}", STANDARD.encode(bytes)));
            }
        }
        let levels = iwi::decode(&self.read(id)?)?;
        let level = &levels[0];
        // Opaque thumbnails retain useful normal-map data instead of treating it as transparency.
        let mut rgba = level.faces[0].clone();
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        let image = RgbaImage::from_raw(level.width, level.height, rgba)
            .ok_or("Invalid thumbnail dimensions.")?;
        let image = DynamicImage::ImageRgba8(image).thumbnail(160, 128);
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        let bytes = encoded.into_inner();
        write_atomic(&path, &bytes, true)?;
        Ok(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
    }

    pub fn pixels(&self, id: &str, face: usize) -> Result<Vec<u8>, String> {
        let mut levels = iwi::decode(&self.read(id)?)?;
        if face >= levels[0].faces.len() {
            return Err("Invalid cubemap face.".into());
        }
        Ok(std::mem::take(&mut levels[0].faces[face]))
    }

    pub fn export_paths(
        &self,
        id: &str,
        destination: &Path,
        kind: &str,
    ) -> Result<Vec<PathBuf>, String> {
        let entry = self.entry(id)?;
        match kind {
            "iwi" => Ok(vec![destination.with_extension("iwi")]),
            "png" if entry.header.faces == 1 => Ok(vec![destination.with_extension("png")]),
            "png" => {
                let stem = Path::new(&entry.name)
                    .file_stem()
                    .ok_or("Invalid image name.")?
                    .to_string_lossy();
                Ok(["px", "nx", "py", "ny", "pz", "nz"]
                    .iter()
                    .map(|face| destination.join(format!("{stem}_{face}.png")))
                    .collect())
            }
            _ => Err("Unknown export format.".into()),
        }
    }

    pub fn export(
        &self,
        id: &str,
        destination: &Path,
        kind: &str,
        replace: bool,
    ) -> Result<Vec<String>, String> {
        let paths = self.export_paths(id, destination, kind)?;
        if !replace && paths.iter().any(|path| path.exists()) {
            return Err("An export file already exists. Nothing was saved.".into());
        }
        let source = self.read(id)?;
        let mut files = Vec::new();
        if kind == "iwi" {
            files.push(source);
        } else {
            let levels = iwi::decode(&source)?;
            let level = &levels[0];
            for face in &level.faces {
                let image = RgbaImage::from_raw(level.width, level.height, face.clone())
                    .ok_or("Invalid image dimensions.")?;
                let mut encoded = Cursor::new(Vec::new());
                DynamicImage::ImageRgba8(image)
                    .write_to(&mut encoded, ImageFormat::Png)
                    .map_err(|e| e.to_string())?;
                files.push(encoded.into_inner());
            }
        }
        let mut saved = Vec::new();
        for (path, bytes) in paths.iter().zip(files.iter()) {
            if let Err(error) = write_atomic(path, bytes, replace) {
                return Err(format!(
                    "Export stopped: {error}. Files already saved: {}",
                    saved.join(", ")
                ));
            }
            saved.push(path.display().to_string());
        }
        Ok(saved)
    }

    pub fn clear_cache(&self) -> Result<(), String> {
        // Delete only this app's known cache files; never recursively follow a junction.
        let thumbnails = self.cache.join("thumbnails");
        for folder in [&self.cache, &thumbnails] {
            if folder.exists()
                && fs::symlink_metadata(folder)
                    .map_err(|e| e.to_string())?
                    .file_type()
                    .is_symlink()
            {
                return Err("Refusing to clear a redirected cache folder.".into());
            }
            if !folder.is_dir() {
                continue;
            }
            for item in fs::read_dir(folder).map_err(|e| e.to_string())? {
                let item = item.map_err(|e| e.to_string())?;
                let name = item.file_name().to_string_lossy().into_owned();
                if item.file_type().map_err(|e| e.to_string())?.is_file()
                    && (name.ends_with(".png")
                        || name.starts_with("index-") && name.ends_with(".json"))
                {
                    fs::remove_file(item.path()).map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    }
}
