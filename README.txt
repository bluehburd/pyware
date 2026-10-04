BOARD PAD - Tauri project (Windows)

ONE-TIME SETUP
 1. Install Node.js (https://nodejs.org)
 2. Install Rust (https://rustup.rs) - accept the defaults
 3. Install "Microsoft C++ Build Tools" with the "Desktop development with C++" workload
    (https://visualstudio.microsoft.com/visual-cpp-build-tools/)
    WebView2 is already included in Windows 11.

BUILD THE APP (in this folder, in a terminal)
    npm install
    npm run build

RESULT
    Installer : src-tauri\target\release\bundle\nsis\Board Pad_1.0.0_x64-setup.exe
    Plain exe : src-tauri\target\release\board-pad.exe   (double-click to run, no install)

TRY IT WITHOUT BUILDING A RELEASE
    npm run dev

EDITING THE APP
    The whole app is src\index.html - edit it and rebuild.
    To use your own icon:  npm run tauri icon path\to\icon.png
