use std::fmt;
use std::io;

use crate::i18n::{Lang, fmt as tf, t};

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
    /// Office documents need the LibreOffice download first.
    NeedsOffice,
    /// Page numbers that are malformed or not in the PDF (the page count is the detail).
    BadPages(u32),
    /// The pages chosen for deletion are all the pages there are.
    NoPagesLeft,
    /// A zero-byte input.
    EmptyFile,
    /// LibreOffice ran past its time limit (usually a very large document).
    OfficeTimeout,
    /// Replace original on a drive without a Recycle Bin: the original is kept.
    NoRecycleBin,
    /// A target file of that name exists and may not be replaced.
    Exists,
    /// An error about one file of a multi-file job (merge): the file name comes first.
    InFile(String, Box<ConvertError>),
    Cancelled,
    Encode(String),
    Io(String),
    NoStream,
    /// A cut that starts after the clip ends.
    TrimOutside,
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
            E::TrimOutside => plain("ce.trim_outside"),
            E::TooLarge => plain("ce.too_large"),
            E::TargetSize => plain("ce.target_size"),
            E::Protected => plain("ce.protected"),
            E::NeedsFfmpeg => plain("ce.needs_ffmpeg"),
            E::NeedsOffice => plain("ce.needs_office"),
            E::BadPages(n) => tf(
                lang,
                if *n == 1 {
                    "ce.bad_pages_one"
                } else {
                    "ce.bad_pages"
                },
                &[("n", &n.to_string())],
            ),
            E::NoPagesLeft => plain("ce.no_pages_left"),
            E::EmptyFile => plain("ce.empty_file"),
            E::OfficeTimeout => plain("ce.office_timeout"),
            E::NoRecycleBin => plain("ce.no_recycle"),
            E::Exists => plain("ce.exists"),
            E::InFile(name, e) => format!("{name}: {}", e.message(lang)),
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
        // 123 = ERROR_INVALID_NAME (also a path past the length limit)
        if e.raw_os_error() == Some(123) {
            return Self::InvalidPath(PathIssue::BadName);
        }
        match e.kind() {
            io::ErrorKind::PermissionDenied => Self::AccessDenied,
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::AlreadyExists => Self::Exists,
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
            ConvertError::NeedsOffice,
            ConvertError::BadPages(3),
            ConvertError::BadPages(1),
            ConvertError::NoPagesLeft,
            ConvertError::EmptyFile,
            ConvertError::OfficeTimeout,
            ConvertError::NoRecycleBin,
            ConvertError::Exists,
            ConvertError::InFile("a.pdf".into(), Box::new(ConvertError::Corrupt)),
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
