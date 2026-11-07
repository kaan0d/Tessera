# Third-party licenses

Tessera is MIT-licensed (see LICENSE). It is written in Rust and runs two separate programs
it downloads: FFmpeg (during setup, or when the app starts unless you chose Later) and LibreOffice
(only when you ask for document support).

## FFmpeg (downloaded at runtime, not bundled)

FFmpeg is licensed under the GNU Lesser General Public License, version 2.1 or later
(LGPL-2.1+). Tessera does not contain or link FFmpeg. During setup (or at the next start, unless
you chose Later in the app) it downloads this
unmodified build and runs `ffmpeg.exe` as a separate process:

- Build: BtbN FFmpeg-Builds, release `autobuild-2026-10-20-13-08`, asset `ffmpeg-n8.1.3-18-g46d899d7a0-win64-lgpl-8.1.zip` (an LGPL build: no GPL components such as x264 or x265)
- Download: first from a copy of the same file on Tessera's own release page (https://github.com/kaan0d/Tessera/releases/tag/ffmpeg-8.1.3), then from https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-20-13-08/ffmpeg-n8.1.3-18-g46d899d7a0-win64-lgpl-8.1.zip
- Release page: https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-10-20-13-08
- Build scripts: https://github.com/BtbN/FFmpeg-Builds
- FFmpeg source of this build (n8.1.3, commit `46d899d7a0`): https://github.com/FFmpeg/FFmpeg/commit/46d899d7a0 and https://ffmpeg.org/download.html
- License text: https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html
- The archive carries its own license files for the libraries inside the build (libwebp, libvpx, opus, vorbis, LAME, OpenH264, ...).

The download is verified against a fixed SHA-256 before it is used. To use your own FFmpeg
instead (for example a newer or custom LGPL build), replace `ffmpeg.exe` in
`data\ffmpeg\` inside the install folder (`%LOCALAPPDATA%\Tessera` by default) with yours. Tessera uses whatever runs there and answers
`ffmpeg -version`; if it stops working, Tessera downloads the pinned build again.

## LibreOffice (downloaded on request, not bundled)

LibreOffice is licensed under the Mozilla Public License 2.0 (MPL-2.0); some of its parts carry
other free licenses, listed in its own `LICENSE.html` and `CREDITS.fodt`. Tessera does not contain
or link LibreOffice. Only when you press Download under Settings > Advanced > Office documents
does it download this unmodified official installer from The Document Foundation, check its
SHA-256, and unpack it into `data\libreoffice\` inside the install folder. Help pages, interface
translations and spelling dictionaries are left out; the license files stay. Tessera runs
`soffice.exe --headless` as separate processes (up to three at once), only to convert files, each
with its own profile in `data\office-profile\` (`office-profile-1`, `-2`; in `%TEMP%` when the
install path is very long). If LibreOffice is already installed on the PC, Tessera uses that
instead. When a newer build is pinned in a Tessera update, Settings offers to replace the copy.

- Build: LibreOffice 26.2.6 (build 26.2.6.3), `LibreOffice_26.2.6_Win_x86-64.msi`, SHA-256 `f9877032fd908beb9c0ddf06df4af5c2e85f419c42e14876c4cce5aae5fb2660`
- Download: https://downloadarchive.documentfoundation.org/libreoffice/old/26.2.6.3/win/x86_64/
- Source code: https://www.libreoffice.org/download/source-code/ and https://git.libreoffice.org/core
- License text: https://www.mozilla.org/en-US/MPL/2.0/ and https://www.libreoffice.org/about-us/licenses/

## Tesseract OCR (downloaded on request, not bundled)

Tesseract is licensed under the Apache License 2.0. Tessera does not contain or link it. Only
when you press Download under Settings > OCR does it download a trimmed copy of the official
Windows build (the installer from the Tesseract project, SHA-256
`bee9e3434bd94fd65387d9be28cd467a41f61b1275383b55b0f59a1331270ae4`), repackaged as
`tesseract-5.5.3-win64-tessera.zip` on Tessera's own release page (SHA-256
`2ff4a233891bd35d728a6728de0144d602db3cbc209953321545978a74111770`), checks it and unpacks it
into `data\tesseract\` inside the install folder. Training tools, documentation and the Java viewer are left out. Tessera
runs `tesseract.exe` as a separate process, only to read text from pictures.

The zip's `NOTICE.txt` lists every file. Its libraries are under permissive licenses (BSD,
MIT, zlib, Apache-2.0, curl, libpng, libtiff, IJG) with these exceptions, all separate DLLs used
unmodified:

- libiconv, libintl (gettext): LGPL-2.1-or-later. libunistring, libidn2: LGPL-3.0-or-later or GPL-2.0-or-later, used under the LGPL. They can be replaced by other builds of the same libraries.
- libgcc_s and libstdc++: GPL-3.0-or-later with the GCC Runtime Library Exception, which allows programs linked to them under any license.
- `libjbig-0.dll` is **not** the original: JBIG-KIT is GPL-licensed, so the zip carries a small MIT-licensed stand-in (`tools/libjbig-stub/` in this repository) that exports the same function names and reports an error from each. A JBIG-compressed TIFF is therefore refused; every other format is unaffected.

Language files are `tessdata_fast` from https://github.com/tesseract-ocr/tessdata_fast (Apache-2.0),
pinned to commit `87416418657359cb625c412a48b6e1d6d41c29bd`, one SHA-256 per file, downloaded one
language at a time.

- Source code: https://github.com/tesseract-ocr/tesseract and https://github.com/tesseract-ocr/tessdata_fast
- License text: https://www.apache.org/licenses/LICENSE-2.0 (also `LICENSE-tesseract.txt` in the zip)

## Rust and its libraries

Tessera is written in Rust and includes the Rust standard library. Rust is dual-licensed under
the MIT license or the Apache License 2.0, at your option.

- License texts: https://github.com/rust-lang/rust/blob/master/LICENSE-MIT and https://github.com/rust-lang/rust/blob/master/LICENSE-APACHE
- Other notices: https://github.com/rust-lang/rust/blob/master/COPYRIGHT

The exe is built from open-source Rust libraries (crates.io) under permissive licenses (MIT,
Apache-2.0, BSD, Zlib, ISC, MPL-2.0, Unicode-3.0, CC0-1.0, CDLA-Permissive-2.0); none is GPL. Among them: Tauri (app shell), image, resvg,
hayro (PDF rendering), pdf-writer, lopdf (PDF editing: merge, split, rotate, compress), symphonia,
hound, flacenc, zip, ureq and winreg. Each crate's license is in its source on crates.io.
