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

/// Set when an encode replaced real transparency with white (JPG, BMP have none); the CLI
/// reads and clears it to print a note.
pub static ALPHA_FLATTENED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// Set (width << 32 | height) when a size limit was met by making the picture smaller; the
/// CLI reads and clears it to say so. 0 = not shrunk.
pub static SHRUNK_TO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
mod error;
pub mod ffmpeg;
mod image;
pub mod office;
pub mod output;
pub mod pdf;
pub mod pdf_edit;
pub mod pdf_tools;
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
    // Office documents, converted by LibreOffice (`office.rs`).
    Docx,
    Doc,
    Odt,
    Rtf,
    Txt,
    Xlsx,
    Xls,
    Ods,
    Csv,
    Pptx,
    Ppt,
    Odp,
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
/// Office families: any member converts to any other member and to PDF.
const TEXT: [Format; 5] = [
    Format::Docx,
    Format::Doc,
    Format::Odt,
    Format::Rtf,
    Format::Txt,
];
const SHEETS: [Format; 4] = [Format::Xlsx, Format::Xls, Format::Ods, Format::Csv];
const SLIDES: [Format; 3] = [Format::Pptx, Format::Ppt, Format::Odp];
/// Slides can also become pictures, one per slide (through a PDF).
const FROM_SLIDES: [Format; 3] = [Format::Pdf, Format::Png, Format::Jpg];
/// Editable text recovered from a PDF (LibreOffice's PDF import; layout often breaks).
const FROM_PDF_TEXT: [Format; 2] = [Format::Docx, Format::Odt];
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
            "docx" => Format::Docx,
            "doc" => Format::Doc,
            "odt" => Format::Odt,
            "rtf" => Format::Rtf,
            "txt" => Format::Txt,
            "xlsx" => Format::Xlsx,
            "xls" => Format::Xls,
            "ods" => Format::Ods,
            "csv" => Format::Csv,
            "pptx" => Format::Pptx,
            "ppt" => Format::Ppt,
            "odp" => Format::Odp,
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
            Format::Docx => "docx",
            Format::Doc => "doc",
            Format::Odt => "odt",
            Format::Rtf => "rtf",
            Format::Txt => "txt",
            Format::Xlsx => "xlsx",
            Format::Xls => "xls",
            Format::Ods => "ods",
            Format::Csv => "csv",
            Format::Pptx => "pptx",
            Format::Ppt => "ppt",
            Format::Odp => "odp",
        }
    }

    /// Pictures: raster formats plus SVG, HEIC and RAW sources.
    pub fn is_image(self) -> bool {
        IMAGES.contains(&self) || IMAGE_ONLY_SOURCES.contains(&self)
    }

    pub fn is_video(self) -> bool {
        VIDEO.contains(&self)
    }

    /// Word processor, spreadsheet and presentation files (LibreOffice). PDF is not one.
    pub fn is_office(self) -> bool {
        TEXT.contains(&self) || SHEETS.contains(&self) || SLIDES.contains(&self)
    }

    /// "image" | "audio" | "video" | "document"
    pub fn kind(self) -> &'static str {
        if self.is_image() {
            "image"
        } else if self.is_video() {
            "video"
        } else if self == Format::Pdf || self.is_office() {
            "document"
        } else {
            "audio"
        }
    }

    /// Every output format (what the settings grid lists). Source-only formats are not here.
    pub const ALL: [Format; 31] = [
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
        Format::Docx,
        Format::Doc,
        Format::Odt,
        Format::Rtf,
        Format::Txt,
        Format::Xlsx,
        Format::Xls,
        Format::Ods,
        Format::Csv,
        Format::Pptx,
        Format::Ppt,
        Format::Odp,
    ];
}

/// Longest cut point accepted (seconds): 100 hours.
const MAX_TRIM: f32 = 360_000.0;

/// Encoder quality knobs; `clamped` pins each to its valid range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quality {
    /// 1..=100 (JPEG; also the AVIF quality and the JPEG quality inside PDFs)
    pub jpeg: u8,
    /// 1..=100, WebP quality when the installed ffmpeg encodes it (lossy)
    pub webp: u8,
    /// WebP stays lossless (the pure-Rust encoder, no ffmpeg needed)
    pub webp_lossless: bool,
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
            webp: 80,
            webp_lossless: false,
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
            webp: self.webp.clamp(1, 100),
            webp_lossless: self.webp_lossless,
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
    /// Video targets: leave the audio track out.
    pub no_audio: bool,
    /// Audio bitrate in kbit/s for lossy audio (also the audio track of a video).
    pub audio_kbps: Option<u16>,
    /// Audio and video: mix the sound down to one channel.
    pub mono: bool,
    /// Audio and video: cut from this second ...
    pub trim_start: Option<f32>,
    /// ... to this second (end of the file when `None`).
    pub trim_end: Option<f32>,
    /// PDF source: page to render, 1-based.
    pub page: u32,
    /// Set from another thread to stop a running job: decode/encode loops, every image of a
    /// batch and every PDF page check it and return `ConvertError::Cancelled`.
    pub cancel: Option<Arc<AtomicBool>>,
    /// A working ffmpeg.exe, when there is one. Only enhances the pure-Rust paths: lossy
    /// WebP goes through its libwebp. `None` (or `quality.webp_lossless`) keeps WebP lossless.
    pub ffmpeg: Option<PathBuf>,
    /// The job asked for its own image quality or video bitrate: a same-format file is encoded
    /// again instead of being skipped or copied.
    pub recode: bool,
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
            no_audio: false,
            audio_kbps: None,
            mono: false,
            trim_start: None,
            trim_end: None,
            page: 1,
            cancel: None,
            ffmpeg: None,
            recode: false,
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

    pub fn ffmpeg(mut self, exe: PathBuf) -> Self {
        self.ffmpeg = Some(exe);
        self
    }

    /// The ffmpeg that should encode a WebP target (lossy), if any.
    pub fn lossy_webp(&self) -> Option<&Path> {
        self.ffmpeg
            .as_deref()
            .filter(|_| !self.quality.webp_lossless)
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
            audio_kbps: self.audio_kbps.map(|k| k.clamp(8, 512)),
            trim_start: self.trim_start.map(|t| t.clamp(0.0, MAX_TRIM)),
            trim_end: self.trim_end.map(|t| t.clamp(0.0, MAX_TRIM)),
            page: self.page.max(1),
            ..self.clone()
        }
    }

    /// True when the job changes the content (so same-format output is meaningful).
    pub fn transforms(&self) -> bool {
        self.resize.is_some()
            || self.strip_metadata
            || self.max_bytes.is_some()
            || self.no_audio
            || self.recode
            || self.cuts_media()
    }

    /// A same-format job whose only goal is a size limit the file already meets: the result is a
    /// plain copy, because re-encoding could only make it bigger.
    pub fn already_fits(&self, src: &Path, from: Format, target: Format) -> bool {
        from == target
            && self.resize.is_none()
            && !self.strip_metadata
            && !self.recode
            && !self.no_audio
            && !self.cuts_media()
            && self
                .max_bytes
                .zip(std::fs::metadata(src).ok())
                .is_some_and(|(max, m)| m.len() <= max)
    }

    /// Writes the copy for [`Options::already_fits`] and finishes the job like a conversion.
    pub fn copy_as_result(&self, src: &Path, dest: &Path) -> Result<PathBuf, ConvertError> {
        output::write_atomic(dest, |w| {
            std::io::copy(&mut std::fs::File::open(src)?, w)?;
            Ok(())
        })?;
        if self.output.conflict == Conflict::ReplaceOriginal {
            output::recycle(src)?;
        }
        Ok(dest.to_path_buf())
    }

    /// Options only ffmpeg applies: a cut, a bitrate, mono.
    fn cuts_media(&self) -> bool {
        self.audio_kbps.is_some()
            || self.mono
            || self.trim_start.is_some_and(|t| t > 0.0)
            || self.trim_end.is_some()
    }

    /// True when `from` is sound or video and the job asks for something only ffmpeg does,
    /// even where the target alone would not need it (WAV to WAV).
    pub fn ffmpeg_only(&self, from: Format) -> bool {
        matches!(from.kind(), "audio" | "video") && self.cuts_media()
    }
}

/// Targets whose encoder is not in this crate (done by ffmpeg).
/// AAC is only offered for video sources by `supported_targets`.
pub fn needs_ffmpeg(target: Format) -> bool {
    target.is_video() || matches!(target, Format::Mp3 | Format::Ogg | Format::Aac)
}

/// True when the conversion has to run through ffmpeg: a video source (even for WAV/FLAC/GIF
/// output), a HEIC/AVIF source (decoded by ffmpeg) or a target the pure-Rust engine
/// cannot write. WebP is never in this list: without ffmpeg it is written lossless by the
/// pure-Rust encoder, with a ready ffmpeg (`Options::ffmpeg`) it is lossy through libwebp.
pub fn uses_ffmpeg(src: Format, target: Format) -> bool {
    src.is_video() || matches!(src, Format::Heic | Format::Avif) || needs_ffmpeg(target)
}

/// True when LibreOffice does the conversion: an office source, or a PDF read back as text.
pub fn uses_office(src: Format, target: Format) -> bool {
    src.is_office() || target.is_office()
}

/// Targets offered for a source extension (empty if the extension is unknown).
/// Includes ffmpeg-only targets; filter with `uses_ffmpeg` when ffmpeg is missing.
pub fn supported_targets(ext: &str) -> Vec<Format> {
    let Some(src) = Format::from_ext(ext) else {
        return vec![];
    };
    let pdf = [Format::Pdf];
    // an animated GIF can become a video as well
    let gif = [Format::Pdf, Format::Mp4, Format::Webm];
    let (family, extra): (&[Format], &[Format]) = if src == Format::Gif {
        (&IMAGES, &gif)
    } else if src.is_image() {
        (&IMAGES, &pdf)
    } else if src == Format::Pdf {
        (&IMAGES, &FROM_PDF_TEXT)
    } else if TEXT.contains(&src) {
        (&TEXT, &pdf)
    } else if SHEETS.contains(&src) {
        (&SHEETS, &pdf)
    } else if SLIDES.contains(&src) {
        (&SLIDES, &FROM_SLIDES)
    } else if src.is_video() {
        (&VIDEO, &FROM_VIDEO)
    } else {
        (&AUDIO, &[])
    };
    family
        .iter()
        .chain(extra)
        .copied()
        .filter(|&t| t != src)
        .collect()
}

/// `supported_targets` plus same-format output for every writable format except PDF and office when
/// the job transforms the file (resize, size limit, strip), which is how "shrink this JPEG"
/// or "email under 10 MB" works. Audio and video go through a re-encode.
pub fn allows(ext: &str, from: Format, target: Format, opts: &Options) -> bool {
    supported_targets(ext).contains(&target)
        || (target == from
            && from != Format::Pdf
            && !from.is_office()
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
    if uses_office(from, target) {
        return Err(ConvertError::NeedsOffice);
    }
    if uses_ffmpeg(from, target) || opts.ffmpeg_only(from) {
        return Err(ConvertError::NeedsFfmpeg);
    }

    opts.check_cancel()?;
    if from == Format::Pdf && target.is_image() {
        let dest = pages_to_images(&src, &src, target, opts, &mut progress)?;
        // a folder of pages is no replacement; one page is a normal conversion
        if dest.is_file() && opts.output.conflict == Conflict::ReplaceOriginal {
            output::recycle(&src)?;
        }
        return Ok(dest);
    }
    let dest = output::plan(&src, target.ext(), &opts.output)?;
    progress(0.0);
    if opts.already_fits(&src, from, target) {
        let done = opts.copy_as_result(&src, &dest)?;
        progress(1.0);
        return Ok(done);
    }
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

/// Every page of `pdf` as a `target` picture. One page gives one file named after `named`;
/// more go into a new folder `<stem>` holding `<stem>-01.png` and so on (removed on failure).
pub fn pages_to_images(
    pdf: &Path,
    named: &Path,
    target: Format,
    opts: &Options,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    progress(0.0);
    let count = pdf::page_count(pdf)?;
    let render = |page: usize, dest: &Path| {
        let o = opts.clone().page(page as u32);
        output::write_atomic(dest, |w| {
            image::convert(pdf, Format::Pdf, target, false, &o, w)
        })
    };
    if count <= 1 {
        let dest = output::plan(named, target.ext(), &opts.output)?;
        render(1, &dest)?;
        progress(1.0);
        return Ok(dest);
    }
    let dir = output::new_dir(named, "", &opts.output)?;
    let stem = named.file_stem().and_then(|s| s.to_str()).unwrap_or("page");
    let width = count.to_string().len().max(2);
    let result = (1..=count).try_for_each(|page| {
        opts.check_cancel()?;
        render(
            page,
            &dir.join(format!("{stem}-{page:0width$}.{}", target.ext())),
        )?;
        progress(page as f32 / count as f32);
        Ok(())
    });
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(dir)
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
        assert_eq!(a, vec![Format::Wav, Format::Flac, Format::Ogg, Format::Aac]);
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
        assert!(supported_targets("gif").contains(&Format::Mp4)); // animated GIFs become videos
    }
}

#[cfg(test)]
mod gif_video_tests {
    use super::*;

    #[test]
    fn a_gif_can_become_a_video_but_a_png_cannot() {
        let gif = supported_targets("gif");
        assert!(
            gif.contains(&Format::Mp4) && gif.contains(&Format::Webm) && gif.contains(&Format::Pdf)
        );
        assert!(!supported_targets("png").contains(&Format::Mp4));
        assert!(uses_ffmpeg(Format::Gif, Format::Mp4));
        assert!(!uses_ffmpeg(Format::Gif, Format::Png));
    }
}

#[cfg(test)]
mod office_tests {
    use super::*;

    #[test]
    fn office_files_convert_inside_their_family_and_to_pdf() {
        use Format::*;
        assert_eq!(supported_targets("DOCX"), vec![Doc, Odt, Rtf, Txt, Pdf]);
        assert_eq!(supported_targets("csv"), vec![Xlsx, Xls, Ods, Pdf]);
        assert_eq!(supported_targets("odp"), vec![Pptx, Ppt, Pdf, Png, Jpg]);
        // no word processor to spreadsheet and the like
        assert!(!supported_targets("docx").contains(&Xlsx));
        assert!(!supported_targets("xlsx").contains(&Png));
    }

    #[test]
    fn pdf_reads_back_as_text_through_office() {
        let t = supported_targets("pdf");
        assert!(t.contains(&Format::Docx) && t.contains(&Format::Odt) && t.contains(&Format::Png));
        assert!(uses_office(Format::Pdf, Format::Docx));
        assert!(!uses_office(Format::Pdf, Format::Png));
        assert!(!uses_office(Format::Png, Format::Pdf));
        assert!(uses_office(Format::Xls, Format::Pdf));
        assert!(!uses_ffmpeg(Format::Docx, Format::Pdf));
        assert_eq!(Format::Pptx.kind(), "document");
    }

    #[test]
    fn office_files_are_never_rewritten_to_the_same_format() {
        let shrink = Options::default().max_bytes(1000);
        assert!(!allows("docx", Format::Docx, Format::Docx, &shrink));
        assert!(allows("jpg", Format::Jpg, Format::Jpg, &shrink));
    }
    #[test]
    fn every_pdf_page_becomes_a_picture() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("deck.pdf");
        std::fs::write(&src, pdf_edit::tests::sample(3)).unwrap();
        let out = convert(&src, Format::Png, &Options::default(), |_| {}).unwrap();
        assert!(out.is_dir() && out.ends_with("deck"));
        for n in ["deck-01.png", "deck-02.png", "deck-03.png"] {
            assert!(::image::open(out.join(n)).is_ok(), "{n}");
        }
        // one page stays one file
        let one = d.path().join("one.pdf");
        std::fs::write(&one, pdf_edit::tests::sample(1)).unwrap();
        let out = convert(&one, Format::Jpg, &Options::default(), |_| {}).unwrap();
        assert!(out.ends_with("one.jpg"));
    }
}
