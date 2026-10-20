# WaW Image Viewer

**An unofficial community app. Not affiliated with Treyarch, Activision or UGX.**

Browse World at War's IWI images, find a filename by eye, and save original IWIs or PNG copies. Made by [phil81334](https://github.com/phil81334). Built with JavaScript, Rust, Vite and Tauri.

![WaW Image Viewer](images/waw-image-viewer.png)

## Download and run

1. Have World at War installed. Modtools are not required.
2. Open [Releases](https://github.com/phil81334/waw-image-viewer/releases/latest). Under **Assets**, download **waw-image-viewer.exe**. The source-code ZIP is for development.
3. Put the EXE in the folder containing `CoDWaW.exe`, then double-click it.

You do not need Node.js, Rust or VS Code to use the EXE. Windows needs [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/#download-section), usually already installed; if missing, install its **Evergreen Bootstrapper**.

## Browse images

Images load from the IWD archives in `main`. Search filenames or filter by archive, dimensions, shape, image type, format, alpha availability or duplicate versions. Click a thumbnail to inspect it, wheel to zoom, or drag to pan. **Copy name** copies the filename without `images/` or `.iwi`.

Thumbnails show opaque RGB. The preview offers transparency, individual R/G/B/A channels and six cubemap faces. These are stored images, not rendered materials. Alpha can contain data rather than transparency. DXT1 supports optional one-bit alpha, so it appears under **Alpha: Available**.

Identical copies of a filename are grouped; different versions remain selectable. Save an original IWI or export PNG copies. PNG exports preserve the original channels regardless of the preview mode; cubemaps export six PNGs. Existing exports require confirmation before replacement. Game archives are never changed.

Thumbnails load as you browse and are cached in AppData along with the image index. **Cache…** shows the folder and lets you clear it. **Rescan** reads the archives again. The first scan takes longer.

## Run from source

**Windows setup — do this once.** Skip anything already installed.

| Install | What to choose |
| --- | --- |
| [VS Code](https://code.visualstudio.com/) | The Windows installer. This is where you open the project. |
| [Node.js](https://nodejs.org/en/download) | **LTS**, **Windows**, **Installer (.msi)**. Keep the default options; npm is included. |
| [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) | In its installer, select **Desktop development with C++**. Keep the selected MSVC tools and Windows SDK. |
| [Rust](https://rust-lang.org/tools/install/) | Download and run **rustup-init.exe (x64)**. Choose the default installation. |

Close and reopen VS Code after installing these. [Tauri's Windows guide](https://v2.tauri.app/start/prerequisites/#windows) has more help. Vite and Tauri are installed by the project; no separate installation or Python is needed.

**Open and run the project:**

1. On this GitHub page, choose **Code → Download ZIP**, then extract it. You can clone it instead if you already use Git.
2. In VS Code, choose **File → Open Folder**. Open the extracted folder containing `package.json`.
3. In VS Code's Explorer, create a folder named `.local`, then a file inside it named `game-path.txt`.
4. Open your WaW game folder in Windows File Explorer—the one containing `CoDWaW.exe`. Click the address bar, copy the full folder address, paste it into `game-path.txt` and save. Use your actual folder, on whichever drive you installed WaW. One line, no quotes, no EXE filename.
5. Choose **Terminal → New Terminal**, then run these commands one at a time:

   ```powershell
   npm install
   npm run dev
   ```

`npm install` downloads the project's dependencies. `npm run dev` opens the desktop app. The first run downloads and compiles Rust dependencies, so give it time. Keep the terminal open; press **Ctrl+C** there to stop development.

Next time, run `npm run dev`, or click the play button beside **dev** in VS Code's **NPM Scripts** panel. If that panel is hidden, right-click the Explorer panel headings and enable **NPM Scripts**. Update `game-path.txt` if you move WaW. The packaged EXE always uses its own folder instead.

| Command | What it does |
| --- | --- |
| `npm run dev` | Opens the desktop app for development. |
| `npm run dev:web` | Shows the UI in a browser. Cannot read game images or export files. |
| `npm run build` | Creates `release/waw-image-viewer.exe`. |
| `npm run build:web` | Builds the UI files only. No EXE. |

To make your own EXE, run `npm run build`, then copy `release/waw-image-viewer.exe` beside `CoDWaW.exe`.

[Credits](ATTRIBUTION.md) · [GPL-3.0 licence](LICENSE)
