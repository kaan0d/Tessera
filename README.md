# Tessera

Right-click any file in Windows, pick a format, done. A free, open-source file converter for images, audio, video and PDF. Tauri 2 + Rust, one small exe, no account, no ads, no telemetry, no admin rights.

Converter apps that cost money, run on subscriptions or bury the exit under upsells do the same job: decode a file, encode it again. This does it from the Explorer menu, on your machine, and your files never leave it. MIT licensed, so there is nothing to buy and nothing to unlock.

## Features

- **Explorer menu:** cascading "Tessera" menu per file type and per folder, with one icon per kind. Lives in your user registry only.
- **Formats:** PNG, JPG, WEBP, GIF, BMP, TIFF, ICO, AVIF, PDF; WAV, FLAC, MP3, OGG, AAC; MP4, MKV, AVI, WEBM, MOV. Also reads SVG, HEIC, RAW (embedded preview) and PDF pages, and turns animated GIFs into MP4 or WebM.
- **Presets:** shrink for web, WhatsApp, email under 10 MB, Discord under 10 MB, strip location data, make a PDF, WebP or an icon. Make your own.
- **Edit while converting:** resize, strip metadata, fix orientation, compress to a target file size, cut audio and video, audio bitrate, mono, GIF fps and width. Save any custom settings as a menu preset.
- **Batch:** merge images into one PDF or audio files into one, convert a whole folder, drop files on the window and choose.
- **Automation:** watch folders with rules, a full CLI for scripts, a Send To entry.
- **Safe by default:** the original stays, output gets a new name, nothing is overwritten unless you turn it on. History with undo, cancel any running job.
- **Pure Rust where possible:** images, audio and PDF need no extra tools. Video, MP3, OGG, AAC and HEIC use ffmpeg.
- **English and Turkish**, light and dark.

## Setup

Windows 10 or 11 (needs Microsoft Edge WebView2, which both already include).

**Download:** get `tessera.exe` from the [Releases page](https://github.com/kaan0d/Tessera/releases) and double-click it. The exe is its own installer, so there is nothing else to download. It is not code-signed yet, so SmartScreen may ask you to confirm once (More info, then Run anyway).

**Build from source** (Rust stable):

```
cd src-tauri
cargo build --release
target\release\tessera.exe
```

The release exe opens its own installer: pick a folder (default `%LOCALAPPDATA%\Tessera`), optionally keep settings and history in a `data` folder next to the app instead of `%APPDATA%`, registers the menu, a Start menu shortcut and an uninstall entry, downloads video support (ffmpeg) with a live percentage, and can add the folder to your PATH so `tessera-cli` works in any terminal (off by default). The window follows Windows: Turkish on a Turkish system, English otherwise. Settings > Apps > Tessera > Uninstall opens the same window and removes the menu, shortcuts, entry, files and (optionally) settings and history. No admin rights, no separate setup file.

```
tessera.exe --install D:\Apps --portable --add-to-path   silent install (adds \Tessera to the folder)
tessera.exe --uninstall --quiet            remove without a window (--keep-data keeps settings)
```

Both silent commands exit 0 on success and 1 on failure. Video support (ffmpeg) is downloaded by `--install` unless it is already there; add `--defer-ffmpeg` to skip it.

## Usage

```
tessera-cli convert photo.heic --to jpg
tessera-cli convert scan.png --to webp --max-edge 1600 --strip
tessera-cli convert clip.mov --to mp4 --max-size 8MB
tessera-cli convert talk.mp4 --to mp3 --start 1:30 --end 4:00 --mono
tessera-cli preset whatsapp beach.jpg --out D:\Sent
tessera-cli merge pdf page1.png page2.png page3.png
tessera-cli list-presets
tessera-cli formats png
```

Exit codes: 0 ok, 1 a file failed (or every file was skipped, nothing converted), 2 usage error. `tessera-cli merge pdf a.png b.png` joins images into one PDF (`--max-size` keeps it small), `merge audio ... --to mp3` joins audio.

In scripts use `tessera-cli.exe` (installed next to `tessera.exe`): the same tool flagged as a console program, so PowerShell and cmd wait for it and see the exit code. It takes folders and `*.png` patterns, `--recursive`, `--dry-run`, `--json` and `--quiet`.

## ffmpeg and your data

Video and a few audio and image formats need ffmpeg. Tessera downloads one pinned LGPL build (about 170 MB, SHA-256 checked) during setup, with a live percentage. Untick it there, or choose Later in the app, and nothing is fetched until you press Download now under Advanced. Nothing else is downloaded, and the update check is off until you turn it on.

Files never leave your computer. On disk, in `%APPDATA%\Tessera` (or the `data` folder next to the app): `settings.json`, `history.json` (your last 200 jobs with their file paths, switch it off under History) and `tessera.log` (failed jobs with file names, 512 KB at most). Uninstalling can delete all of it.

## What it changes on your PC

Per user, no admin rights. Files: the install folder (default `%LOCALAPPDATA%\Tessera`) and `%APPDATA%\Tessera` (or `data` next to the app). Registry, all under `HKCU`: the Explorer menu keys per file type, the uninstall entry, optionally `Run` (Start with Windows) and the user `Path`. Uninstalling removes all of it; settings and history go too if you tick that box. Converted files are never touched.

## Contributing

Issues and pull requests are welcome. `cd src-tauri && cargo test && cargo clippy --all-targets` should stay clean. Report security problems privately through GitHub's "Report a vulnerability" on the repository.

## Limits

- Windows 11 shows classic menus under "Show more options".
- The window needs Microsoft Edge WebView2, which Windows 11 and current Windows 10 already include.
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
