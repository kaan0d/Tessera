# Third-party licenses

Tessera is MIT-licensed (see LICENSE). It is written in Rust and runs a separate FFmpeg
program that it downloads on first use.

## FFmpeg (downloaded at runtime, not bundled)

FFmpeg is licensed under the GNU Lesser General Public License, version 2.1 or later
(LGPL-2.1+). Tessera does not contain or link FFmpeg. On first use it downloads this
unmodified build and runs `ffmpeg.exe` as a separate process:

- Build: BtbN FFmpeg-Builds, release `autobuild-2026-10-20-13-08`, asset `ffmpeg-n8.1.3-18-g46d899d7a0-win64-lgpl-8.1.zip` (an LGPL build: no GPL components such as x264 or x265)
- Download: https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-20-13-08/ffmpeg-n8.1.3-18-g46d899d7a0-win64-lgpl-8.1.zip
- Release page: https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-10-20-13-08
- Build scripts: https://github.com/BtbN/FFmpeg-Builds
- FFmpeg source of this build (n8.1.3, commit `46d899d7a0`): https://github.com/FFmpeg/FFmpeg/commit/46d899d7a0 and https://ffmpeg.org/download.html
- License text: https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html
- The archive carries its own license files for the libraries inside the build (libwebp, libvpx, opus, vorbis, LAME, OpenH264, ...).

The download is verified against a fixed SHA-256 before it is used. To use your own FFmpeg
instead (for example a newer or custom LGPL build), replace `ffmpeg.exe` in
`data\ffmpeg\` inside the install folder (`%LOCALAPPDATA%\Tessera` by default) with yours. Tessera uses whatever runs there and answers
`ffmpeg -version`; if it stops working, Tessera downloads the pinned build again.

## Rust

Tessera is written in Rust and includes the Rust standard library. Rust is dual-licensed under
the MIT license or the Apache License 2.0, at your option.

- License texts: https://github.com/rust-lang/rust/blob/master/LICENSE-MIT and https://github.com/rust-lang/rust/blob/master/LICENSE-APACHE
- Other notices: https://github.com/rust-lang/rust/blob/master/COPYRIGHT
