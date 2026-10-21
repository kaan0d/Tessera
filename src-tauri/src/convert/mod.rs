mod audio;
mod error;
pub mod ffmpeg;
mod image;
pub mod output;
pub mod security;

use std::path::{Path, PathBuf};

pub use error::ConvertError;
pub use output::{Conflict, OutDir, OutputParams};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Png,
    Jpg,
    Webp,
    Bmp,
    Gif,
    Tiff,
    Ico,
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

const IMAGES: [Format; 7] = [
    Format::Png,
    Format::Jpg,
    Format::Webp,
    Format::Bmp,
    Format::Gif,
    Format::Tiff,
    Format::Ico,
];
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

    /// Extension used for output files.
    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpg => "jpg",
            Format::Webp => "webp",
            Format::Bmp => "bmp",
            Format::Gif => "gif",
            Format::Tiff => "tiff",
            Format::Ico => "ico",
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

    pub fn is_image(self) -> bool {
        IMAGES.contains(&self)
    }

    pub fn is_video(self) -> bool {
        VIDEO.contains(&self)
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    /// 1..=100
    pub jpeg_quality: u8,
    pub output: OutputParams,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            jpeg_quality: 85,
            output: OutputParams::default(),
        }
    }
}

/// Targets whose encoder is not in this crate (done by ffmpeg).
/// AAC is only offered for video sources by `supported_targets`.
pub fn needs_ffmpeg(target: Format) -> bool {
    target.is_video() || matches!(target, Format::Mp3 | Format::Ogg | Format::Aac)
}

/// True when the conversion runs through ffmpeg: a video source (even for WAV/FLAC/GIF
/// output) or a target the pure-Rust engine cannot write.
pub fn uses_ffmpeg(src: Format, target: Format) -> bool {
    src.is_video() || needs_ffmpeg(target)
}

/// Targets offered for a source extension (empty if the extension is unknown).
/// Includes ffmpeg-only targets; filter with `needs_ffmpeg` when ffmpeg is missing.
pub fn supported_targets(ext: &str) -> Vec<Format> {
    let Some(src) = Format::from_ext(ext) else {
        return vec![];
    };
    let (family, extra): (&[Format], &[Format]) = if src.is_image() {
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

/// Converts `input` to `target`, returning the written file's path.
/// `progress` receives 0.0..=1.0.
pub fn convert(
    input: &Path,
    target: Format,
    opts: &Options,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    let (src, from) = security::validate_input(input)?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !supported_targets(ext).contains(&target) {
        return Err(ConvertError::Unsupported);
    }
    if uses_ffmpeg(from, target) {
        return Err(ConvertError::NeedsFfmpeg);
    }

    let dest = output::plan(&src, target.ext(), &opts.output)?;
    progress(0.0);
    output::write_atomic(&dest, |w| {
        if from.is_image() {
            image::convert(&src, target, opts.jpeg_quality, w)
        } else {
            audio::convert(&src, ext, target, w, &mut progress)
        }
    })?;
    if opts.output.conflict == Conflict::ReplaceOriginal {
        output::recycle(&src)?;
    }
    progress(1.0);
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_matrix() {
        let t = supported_targets("JPEG");
        assert!(!t.contains(&Format::Jpg));
        assert!(t.contains(&Format::Png) && t.contains(&Format::Ico) && t.len() == 6);
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
