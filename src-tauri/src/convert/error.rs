use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConvertError {
    #[error("Geçersiz dosya yolu: {0}")]
    InvalidPath(String),
    #[error("Dosya bulunamadı")]
    NotFound,
    #[error("Erişim reddedildi")]
    AccessDenied,
    #[error("Disk dolu")]
    DiskFull,
    #[error("Dosya bozuk veya okunamıyor")]
    Corrupt,
    #[error("Desteklenmeyen biçim")]
    Unsupported,
    #[error("Dosya çok büyük")]
    TooLarge,
    #[error("Bu dönüşüm için ffmpeg gerekiyor")]
    NeedsFfmpeg,
    #[error("Dönüştürme başarısız: {0}")]
    Encode(String),
    #[error("Dosya hatası: {0}")]
    Io(String),
}

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
