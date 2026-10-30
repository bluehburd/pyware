# Board Pad (native)

Rust + egui rewrite. Stage 1: tabs, infinite board (pan, zoom), notes with per-word rich text, autosave.

Build locally: `cargo build --release` -> `target/release/board-pad.exe`
Build on GitHub: push this folder to a repo, open Actions -> "Build native exe" -> Run workflow, then download the artifact.

Board data is saved to `%APPDATA%\BoardPad\board.json`.
