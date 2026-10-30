# Board Pad (native)

Rust + egui rewrite. Stage 1: tabs, infinite board (pan, zoom), notes with per-word rich text, autosave.

Build locally: `cargo build --release` -> `target/release/board-pad.exe`
Build on GitHub: push this folder to a repo, open Actions -> "Build native exe" -> Run workflow, then download the artifact.

Board data is saved to `%APPDATA%\BoardPad\board.json`.

## Stage 2 (3.0.0-alpha.2)
- Images: Ctrl+V a screenshot/image, or drop files on the window. Stored as PNG in `%APPDATA%\BoardPad\img`, max 1600px. Corner resizes, keeps aspect.
- Pins: Pin tool, click to drop. Pins stick to the note/image under them and follow it.
- Strings: select exactly two pins (Ctrl+click), right-click, "Tie with string". "Cut strings" on a pin removes its strings.
- Multi-select: Ctrl/Shift+click, or drag a box. Dragging moves the whole selection. Ctrl+A, Delete, Esc.
- Tools: Select / Pin / Hand. Pan with Hand tool, Space, middle-drag, or scroll.
