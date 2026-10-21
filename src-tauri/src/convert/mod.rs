//! Conversion engine.
//!
//! Presets and the UI describe a job with [`Options`]: start from `Options::default()` (or
//! `Options::from(..)` in settings) and chain the builder methods, e.g.
//! `Options::default().resize(Resize::MaxEdge(1280)).max_bytes(200_000).strip_metadata(true)`.
//! Every value is clamped by [`Options::clamped`] before use, so out-of-range input from a
//! preset or settings file is safe. Then call [`convert`] (pure-Rust paths) or
//! [`ffmpeg::convert_with_ffmpeg`] when [`uses_ffmpeg`] says so.

mod audio;
pub mod combine;
mod error;
pub mod ffmpeg;
mod image;
pub mod output;
pub mod pdf;
mod raw;
pub mod security;
mod size;
mod svg;
mod transform;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use error::{ConvertError, PathIssue};
pub use output::{Conflict, OutDir, OutputParams};
pub use transform::{Fit, Resize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Png,
    Jpg,
    Webp,
    Bmp,
    Gif,
    Tiff,
    Ico,
    Avif,
    /// Source only (decoded through ffmpeg).
    Heic,
    /// Source only: rasterized SVG.
    Svg,
    /// Source only: dng cr2 cr3 nef arw orf rw2 raf (embedded preview, see `raw.rs`).
    Raw,
    /// Target for any image; source: page `Options::page`.
    Pdf,
    Wav,
    Flac,
    Mp3,
    Ogg,
    Aac,
    Mp4,
    Mkv,
    Avi,
    Webm,
    Mov,
}

/// Raster formats that work as source and target.
const IMAGES: [Format; 8] = [
    Format::Png,
    Format::Jpg,
    Format::Webp,
    Format::Bmp,
    Format::Gif,
    Format::Tiff,
    Format::Ico,
    Format::Avif,
];
/// Image sources that cannot be written.
const IMAGE_ONLY_SOURCES: [Format; 3] = [Format::Heic, Format::Svg, Format::Raw];
const AUDIO: [Format; 5] = [
    Format::Wav,
    Format::Flac,
    Format::Mp3,
    Format::Ogg,
    Format::Aac,
];
const VIDEO: [Format; 5] = [
    Format::Mp4,
    Format::Mkv,
    Format::Avi,
    Format::Webm,
    Format::Mov,
];
/// Targets for a video source: other containers, GIF, and audio extraction.
const FROM_VIDEO: [Format; 6] = [
    Format::Gif,
    Format::Mp3,
    Format::Ogg,
    Format::Aac,
    Format::Wav,
    Format::Flac,
];

impl Format {
    pub fn from_ext(ext: &str) -> Option<Format> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "png" => Format::Png,
            "jpg" | "jpeg" => Format::Jpg,
            "webp" => Format::Webp,
            "bmp" => Format::Bmp,
            "gif" => Format::Gif,
            "tif" | "tiff" => Format::Tiff,
            "ico" => Format::Ico,
            "avif" => Format::Avif,
            "heic" | "heif" => Format::Heic,
            "svg" => Format::Svg,
            "dng" | "cr2" | "cr3" | "nef" | "arw" | "orf" | "rw2" | "raf" => Format::Raw,
            "pdf" => Format::Pdf,
            "wav" => Format::Wav,
            "flac" => Format::Flac,
            "mp3" => Format::Mp3,
            "ogg" | "oga" => Format::Ogg,
            "aac" | "m4a" => Format::Aac,
            "mp4" => Format::Mp4,
            "mkv" => Format::Mkv,
            "avi" => Format::Avi,
            "webm" => Format::Webm,
            "mov" => Format::Mov,
            _ => return None,
        })
    }

    /// Extension used for output files (`Raw` is never an output; "dng" stands for the family).
    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpg => "jpg",
            Format::Webp => "webp",
            Format::Bmp => "bmp",
            Format::Gif => "gif",
            Format::Tiff => "tiff",
            Format::Ico => "ico",
            Format::Avif => "avif",
            Format::Heic => "heic",
            Format::Svg => "svg",
            Format::Raw => "dng",
            Format::Pdf => "pdf",
            Format::Wav => "wav",
            Format::Flac => "flac",
            Format::Mp3 => "mp3",
            Format::Ogg => "ogg",
            Format::Aac => "aac",
            Format::Mp4 => "mp4",
            Format::Mkv => "mkv",
            Format::Avi => "avi",
            Format::Webm => "webm",
            Format::Mov => "mov",
        }
    }

    /// Pictures: raster formats plus SVG, HEIC and RAW sources.
    pub fn is_image(self) -> bool {
        IMAGES.contains(&self) || IMAGE_ONLY_SOURCES.contains(&self)
    }

    pub fn is_video(self) -> bool {
        VIDEO.contains(&self)
    }

    /// "image" | "audio" | "video" | "document"
    pub fn kind(self) -> &'static str {
        if self.is_image() {
            "image"
        } else if self.is_video() {
            "video"
        } else if self == Format::Pdf {
            "document"
        } else {
            "audio"
        }
    }

    /// Every output format (what the settings grid lists). Source-only formats are not here.
    pub const ALL: [Format; 19] = [
        Format::Png,
        Format::Jpg,
        Format::Webp,
        Format::Bmp,
        Format::Gif,
        Format::Tiff,
        Format::Ico,
        Format::Avif,
        Format::Pdf,
        Format::Wav,
        Format::Flac,
        Format::Mp3,
        Format::Ogg,
        Format::Aac,
        Format::Mp4,
        Format::Mkv,
        Format::Avi,
        Format::Webm,
        Format::Mov,
    ];
}

/// Encoder quality knobs; `clamped` pins each to its valid range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quality {
    /// 1..=100 (JPEG; also the AVIF quality and the JPEG quality inside PDFs)
    pub jpeg: u8,
    /// lame VBR `-q:a`, 0 (best) ..= 9
    pub mp3_q: u8,
    /// vorbis `-q:a`, 0 ..= 10
    pub ogg_q: u8,
    /// H.264 bitrate, 1..=30 Mbit/s
    pub video_mbps: u8,
    /// 5..=30
    pub gif_fps: u8,
    /// 120..=1280 px
    pub gif_width: u16,
}

impl Default for Quality {
    fn default() -> Self {
        Quality {
            jpeg: 85,
            mp3_q: 2,
            ogg_q: 5,
            video_mbps: 6,
            gif_fps: 12,
            gif_width: 480,
        }
    }
}

impl Quality {
    pub fn clamped(self) -> Self {
        Quality {
            jpeg: self.jpeg.clamp(1, 100),
            mp3_q: self.mp3_q.min(9),
            ogg_q: self.ogg_q.min(10),
            video_mbps: self.video_mbps.clamp(1, 30),
            gif_fps: self.gif_fps.clamp(5, 30),
            gif_width: self.gif_width.clamp(120, 1280),
        }
    }
}

/// Everything a job can ask for. `Default` changes nothing: no transform, orientation
/// baked into pixels, metadata never copied.
#[derive(Debug, Clone)]
pub struct Options {
    pub quality: Quality,
    pub output: OutputParams,
    /// Size rule for image and PDF/SVG outputs, and a scale filter for video targets.
    pub resize: Option<Resize>,
    /// Output carries no EXIF/GPS/XMP/comments. The image encoders never write metadata,
    /// so this matters for ICC in JPEG-in-PDF and for ffmpeg outputs (container tags).
    pub strip_metadata: bool,
    /// Apply the EXIF orientation to the pixels before the (EXIF-free) output is written,
    /// so photos do not come out sideways. On by default; turning it off is for tests.
    pub preserve_orientation: bool,
    /// Compress to at most this many bytes: JPEG/AVIF quality search, then downscaling;
    /// bitrate from the duration for video, MP3, OGG, AAC. WAV, FLAC and GIF cannot trade
    /// quality for size, so they fail with `TargetSize` when over the limit.
    /// Fails with `ConvertError::TargetSize` when it cannot be met.
    pub max_bytes: Option<u64>,
    /// PDF source: page to render, 1-based.
    pub page: u32,
    /// Set from another thread to stop a running job: decode/encode loops, every image of a
    /// batch and every PDF page check it and return `ConvertError::Cancelled`.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            quality: Quality::default(),
            output: OutputParams::default(),
            resize: None,
            strip_metadata: false,
            preserve_orientation: true,
            max_bytes: None,
            page: 1,
            cancel: None,
        }
    }
}

impl Options {
    pub fn resize(mut self, r: Resize) -> Self {
        self.resize = Some(r);
        self
    }

    pub fn strip_metadata(mut self, on: bool) -> Self {
        self.strip_metadata = on;
        self
    }

    pub fn max_bytes(mut self, n: u64) -> Self {
        self.max_bytes = Some(n);
        self
    }

    pub fn quality(mut self, q: Quality) -> Self {
        self.quality = q;
        self
    }

    pub fn output(mut self, o: OutputParams) -> Self {
        self.output = o;
        self
    }

    pub fn page(mut self, n: u32) -> Self {
        self.page = n;
        self
    }

    pub fn cancel(mut self, token: Arc<AtomicBool>) -> Self {
        self.cancel = Some(token);
        self
    }

    /// `Err(Cancelled)` once the cancel token is set.
    pub fn check_cancel(&self) -> Result<(), ConvertError> {
        match &self.cancel {
            Some(c) if c.load(Ordering::Relaxed) => Err(ConvertError::Cancelled),
            _ => Ok(()),
        }
    }

    /// Every field pinned to its valid range (`convert` does this itself).
    pub fn clamped(&self) -> Self {
        Options {
            quality: self.quality.clamped(),
            resize: self.resize.map(Resize::clamped),
            max_bytes: self.max_bytes.map(|n| n.max(1)),
            page: self.page.max(1),
            ..self.clone()
        }
    }

    /// True when the job changes the content (so same-format output is meaningful).
    pub fn transforms(&self) -> bool {
        self.resize.is_some() || self.strip_metadata || self.max_bytes.is_some()
    }
}

/// Targets whose encoder is not in this crate (done by ffmpeg).
/// AAC is only offered for video sources by `supported_targets`.
pub fn needs_ffmpeg(target: Format) -> bool {
    target.is_video() || matches!(target, Format::Mp3 | Format::Ogg | Format::Aac)
}

/// True when the conversion runs through ffmpeg: a video source (even for WAV/FLAC/GIF
/// output), a HEIC/AVIF source (decoded by ffmpeg) or a target the pure-Rust engine
/// cannot write.
pub fn uses_ffmpeg(src: Format, target: Format) -> bool {
    src.is_video() || matches!(src, Format::Heic | Format::Avif) || needs_ffmpeg(target)
}

/// Targets offered for a source extension (empty if the extension is unknown).
/// Includes ffmpeg-only targets; filter with `uses_ffmpeg` when ffmpeg is missing.
pub fn supported_targets(ext: &str) -> Vec<Format> {
    let Some(src) = Format::from_ext(ext) else {
        return vec![];
    };
    let pdf = [Format::Pdf];
    let (family, extra): (&[Format], &[Format]) = if src.is_image() {
        (&IMAGES, &pdf)
    } else if src == Format::Pdf {
        (&IMAGES, &[])
    } else if src.is_video() {
        (&VIDEO, &FROM_VIDEO)
    } else {
        (&AUDIO, &[])
    };
    family
        .iter()
        .chain(extra)
        .copied()
        .filter(|&t| t != src && (src.is_video() || t != Format::Aac))
        .collect()
}

/// `supported_targets` plus same-format output for every writable format except PDF when
/// the job transforms the file (resize, size limit, strip), which is how "shrink this JPEG"
/// or "email under 10 MB" works. Audio and video go through a re-encode.
pub fn allows(ext: &str, from: Format, target: Format, opts: &Options) -> bool {
    supported_targets(ext).contains(&target)
        || (target == from
            && from != Format::Pdf
            && Format::ALL.contains(&from)
            && opts.transforms())
}

/// Converts `input` to `target`, returning the written file's path.
/// `progress` receives 0.0..=1.0.
pub fn convert(
    input: &Path,
    target: Format,
    opts: &Options,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    let opts = &opts.clamped();
    let (src, from) = security::validate_input(input)?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !allows(ext, from, target, opts) {
        return Err(ConvertError::Unsupported);
    }
    if uses_ffmpeg(from, target) {
        return Err(ConvertError::NeedsFfmpeg);
    }

    opts.check_cancel()?;
    let dest = output::plan(&src, target.ext(), &opts.output)?;
    progress(0.0);
    output::write_atomic(&dest, |w| {
        if from.is_image() || from == Format::Pdf {
            let photo = matches!(from, Format::Jpg | Format::Raw);
            image::convert(&src, from, target, photo, opts, w)
        } else {
            audio::convert(&src, ext, target, w, &mut progress, opts.cancel.as_deref())
        }
    })?;
    // the pure-Rust audio writers (WAV, FLAC) cannot trade quality for size
    if let (Some(max), false) = (opts.max_bytes, from.is_image() || from == Format::Pdf) {
        if std::fs::metadata(&dest)?.len() > max {
            let _ = std::fs::remove_file(&dest);
            return Err(ConvertError::TargetSize);
        }
    }
    if opts.output.conflict == Conflict::ReplaceOriginal {
        output::recycle(&src)?;
    }
    progress(1.0);
    Ok(dest)
}

/// Decodes a still image the way `convert` would (with the options' orientation and resize)
/// and encodes it to `w`; used by the ffmpeg path once ffmpeg has produced a PNG.
pub(crate) fn encode_decoded(
    png: &Path,
    target: Format,
    photo: bool,
    opts: &Options,
    w: &mut (impl std::io::Write + std::io::Seek),
) -> Result<(), ConvertError> {
    image::convert(png, Format::Png, target, photo, opts, w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_matrix() {
        let t = supported_targets("JPEG");
        assert!(!t.contains(&Format::Jpg));
        assert!(t.contains(&Format::Png) && t.contains(&Format::Ico) && t.len() == 8);
        assert!(t.contains(&Format::Avif) && t.contains(&Format::Pdf));
        assert!(supported_targets("tif").contains(&Format::Webp));
        let a = supported_targets("mp3");
        assert_eq!(a, vec![Format::Wav, Format::Flac, Format::Ogg]);
        assert!(supported_targets("m4a").contains(&Format::Mp3));
        assert!(!supported_targets("png").contains(&Format::Wav));
        assert!(supported_targets("exe").is_empty());
    }

    #[test]
    fn ffmpeg_split() {
        assert!(needs_ffmpeg(Format::Mp3) && needs_ffmpeg(Format::Ogg));
        assert!(!needs_ffmpeg(Format::Wav) && !needs_ffmpeg(Format::Flac));
        assert!(!needs_ffmpeg(Format::Png));
        assert!(needs_ffmpeg(Format::Mp4) && needs_ffmpeg(Format::Webm));
        assert!(uses_ffmpeg(Format::Mkv, Format::Wav) && uses_ffmpeg(Format::Mp4, Format::Gif));
        assert!(!uses_ffmpeg(Format::Png, Format::Gif));
    }

    #[test]
    fn video_matrix() {
        let t = supported_targets("MP4");
        assert!(!t.contains(&Format::Mp4) && t.contains(&Format::Mkv));
        for f in [
            Format::Gif,
            Format::Mp3,
            Format::Aac,
            Format::Wav,
            Format::Flac,
        ] {
            assert!(t.contains(&f), "{f:?}");
        }
        assert_eq!(t.len(), 4 + 6);
        assert!(!supported_targets("wav").contains(&Format::Mp4));
        assert!(!supported_targets("gif").contains(&Format::Mp4));
    }
}
