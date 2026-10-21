use std::fmt;
use std::io;

use crate::i18n::{fmt as tf, t, Lang};

/// Why a path was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathIssue {
    NotFile,
    NotDir,
    Network,
    BadName,
    NoFreeName,
}

#[derive(Debug)]
pub enum ConvertError {
    InvalidPath(PathIssue),
    NotFound,
    AccessDenied,
    DiskFull,
    Corrupt,
    Unsupported,
    TooLarge,
    /// `max_bytes` cannot be met (quality and size floors reached, or bitrate too low).
    TargetSize,
    /// Password-protected document.
    Protected,
    NeedsFfmpeg,
    Cancelled,
    Encode(String),
    Io(String),
    NoStream,
    EmptyOutput,
    FfmpegStart(String),
    FfmpegWait(String),
    Recycle(String),
}

impl ConvertError {
    /// Text for the UI and notifications.
    pub fn message(&self, lang: Lang) -> String {
        use ConvertError as E;
        let plain = |k| t(lang, k).to_string();
        let with = |k, d: &str| tf(lang, k, &[("detail", d)]);
        match self {
            E::InvalidPath(i) => with(
                "ce.invalid_path",
                t(
                    lang,
                    match i {
                        PathIssue::NotFile => "ce.not_file",
                        PathIssue::NotDir => "ce.not_dir",
                        PathIssue::Network => "ce.network_path",
                        PathIssue::BadName => "ce.bad_name",
                        PathIssue::NoFreeName => "ce.no_free_name",
                    },
                ),
            ),
            E::NotFound => plain("ce.not_found"),
            E::AccessDenied => plain("ce.access_denied"),
            E::DiskFull => plain("ce.disk_full"),
            E::Corrupt => plain("ce.corrupt"),
            E::Unsupported => plain("ce.unsupported"),
            E::TooLarge => plain("ce.too_large"),
            E::TargetSize => plain("ce.target_size"),
            E::Protected => plain("ce.protected"),
            E::NeedsFfmpeg => plain("ce.needs_ffmpeg"),
            E::Cancelled => plain("ce.cancelled"),
            E::Encode(d) => with("ce.encode", d),
            E::Io(d) => with("ce.io", d),
            E::NoStream => plain("ce.no_stream"),
            E::EmptyOutput => plain("ce.empty_output"),
            E::FfmpegStart(d) => with("ce.ffmpeg_start", d),
            E::FfmpegWait(d) => with("ce.ffmpeg_wait", d),
            E::Recycle(d) => with("ce.recycle", d),
        }
    }
}

/// English, for logs and test output.
impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message(Lang::En))
    }
}

impl std::error::Error for ConvertError {}

impl From<io::Error> for ConvertError {
    fn from(e: io::Error) -> Self {
        // 112 = ERROR_DISK_FULL, 39 = ERROR_HANDLE_DISK_FULL on Windows
        if matches!(e.raw_os_error(), Some(112 | 39)) || e.kind() == io::ErrorKind::StorageFull {
            return Self::DiskFull;
        }
        match e.kind() {
            io::ErrorKind::PermissionDenied => Self::AccessDenied,
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData => Self::Corrupt,
            _ => Self::Io(e.to_string()),
        }
    }
}

impl From<image::ImageError> for ConvertError {
    fn from(e: image::ImageError) -> Self {
        use image::ImageError as E;
        match e {
            E::IoError(e) => e.into(),
            E::Decoding(_) => Self::Corrupt,
            E::Unsupported(_) => Self::Unsupported,
            E::Limits(_) => Self::TooLarge,
            E::Encoding(e) => Self::Encode(e.to_string()),
            E::Parameter(e) => Self::Encode(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_text_in_both_languages() {
        let all = [
            ConvertError::InvalidPath(PathIssue::NotFile),
            ConvertError::InvalidPath(PathIssue::NotDir),
            ConvertError::InvalidPath(PathIssue::Network),
            ConvertError::InvalidPath(PathIssue::BadName),
            ConvertError::InvalidPath(PathIssue::NoFreeName),
            ConvertError::NotFound,
            ConvertError::AccessDenied,
            ConvertError::DiskFull,
            ConvertError::Corrupt,
            ConvertError::Unsupported,
            ConvertError::TooLarge,
            ConvertError::TargetSize,
            ConvertError::Protected,
            ConvertError::NeedsFfmpeg,
            ConvertError::Cancelled,
            ConvertError::Encode("x".into()),
            ConvertError::Io("x".into()),
            ConvertError::NoStream,
            ConvertError::EmptyOutput,
            ConvertError::FfmpegStart("x".into()),
            ConvertError::FfmpegWait("x".into()),
            ConvertError::Recycle("x".into()),
        ];
        for e in &all {
            let (en, tr) = (e.message(Lang::En), e.message(Lang::Tr));
            for m in [&en, &tr] {
                // a missing key would come back as the key itself
                assert!(!m.contains("ce.") && !m.contains("{detail}"), "{e:?}: {m}");
            }
            assert_ne!(en, tr, "{e:?}");
            assert_eq!(e.to_string(), en);
        }
    }
}
