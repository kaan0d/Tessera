# Tessera

Right-click any file in Windows, pick a format, done. A free, open-source file converter for images, audio, video and PDF. Tauri 2 + Rust, one small exe, no account, no ads, no telemetry, no admin rights.

Converter apps that cost money, run on subscriptions or bury the exit under upsells do the same job: decode a file, encode it again. This does it from the Explorer menu, on your machine, and your files never leave it. MIT licensed, so there is nothing to buy and nothing to unlock.

## Features

- **Explorer menu:** cascading "Tessera" menu per file type and per folder, with one icon per kind. Lives in your user registry only.
- **Formats:** PNG, JPG, WEBP, GIF, BMP, TIFF, ICO, AVIF, PDF; WAV, FLAC, MP3, OGG, AAC; MP4, MKV, AVI, WEBM, MOV. Also reads SVG, HEIC, RAW (embedded preview) and PDF pages.
- **Presets:** shrink for web, WhatsApp, email under 10 MB, Discord under 8 MB, strip location data, make a PDF. Make your own.
- **Edit while converting:** resize, strip metadata, fix orientation, compress to a target file size, cut audio and video, audio bitrate, mono, GIF fps and width. Save any custom settings as a menu preset.
- **Batch:** merge images into one PDF or audio files into one, convert a whole folder, drop files on the window and choose.
- **Automation:** watch folders with rules, a full CLI for scripts, a Send To entry.
- **Safe by default:** the original stays, output gets a new name, nothing is overwritten unless you turn it on. History with undo, cancel any running job.
- **Pure Rust where possible:** images, audio and PDF need no extra tools. Video, MP3, OGG, AAC and HEIC use ffmpeg.
- **English and Turkish**, light and dark.

## Setup

Windows 10 or 11. Build from source (Rust stable):

```
cd src-tauri
cargo build --release
target\release\tessera.exe
```

The menu registers on first run. `tessera.exe --remove-menu` takes it out again.

Per-user installer (no admin, installs to `%LOCALAPPDATA%\Tessera`, uninstall removes the menu, autostart entry and app data):

```
npm install
npx tauri build --bundles nsis
```

Output: `src-tauri	argeteleaseundle
sis\Tessera_<version>_x64-setup.exe`.

## Usage

```
tessera convert photo.heic --to jpg
tessera convert scan.png --to webp --max-edge 1600 --strip
tessera convert clip.mov --to mp4 --max-size 8MB
tessera preset whatsapp beach.jpg
tessera list-presets
tessera formats png
```

Exit codes: 0 ok, 1 a file failed, 2 usage error.

## ffmpeg

Video and a few audio and image formats need ffmpeg. Tessera downloads one pinned LGPL build (about 170 MB, SHA-256 checked) on first run and tells you first. Nothing else is downloaded, and the update check is off until you turn it on.

## Limits

- Windows 11 shows classic menus under "Show more options".
- The installer is not code signed, so SmartScreen may warn on first run.
- A full-screen exclusive game hides the progress panel.
- RAW files use the embedded JPEG preview, not a full demosaic.

## Layout

```
src/         UI (vanilla JS, no framework)
src-tauri/   Rust: convert engine, queue, shell menu, watch, CLI
```

## License

MIT. Third-party notices are in `THIRD-PARTY-LICENSES.md`.
