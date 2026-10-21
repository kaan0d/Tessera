use std::fs;
use std::path::{Path, PathBuf};

use super::{ConvertError, Format};

/// Canonicalizes and checks an input path: no UNC/device paths, a real
/// non-empty file, extension on the allowlist.
pub fn validate_input(path: &Path) -> Result<(PathBuf, Format), ConvertError> {
    let canon = canonical(path)?;
    let meta = fs::metadata(&canon)?;
    if !meta.is_file() {
        return Err(ConvertError::InvalidPath("dosya değil".into()));
    }
    if meta.len() == 0 {
        return Err(ConvertError::Corrupt);
    }
    let ext = canon.extension().and_then(|e| e.to_str()).unwrap_or("");
    let fmt = Format::from_ext(ext).ok_or(ConvertError::Unsupported)?;
    Ok((canon, fmt))
}

/// Same checks for a user-chosen output directory; it is created if missing.
pub fn validate_output_dir(dir: &Path) -> Result<PathBuf, ConvertError> {
    reject_device(dir)?;
    fs::create_dir_all(dir)?;
    let canon = canonical(dir)?;
    if !canon.is_dir() {
        return Err(ConvertError::InvalidPath("klasör değil".into()));
    }
    Ok(canon)
}

fn reject_device(path: &Path) -> Result<(), ConvertError> {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\") || s.starts_with("//") {
        return Err(ConvertError::InvalidPath("ağ veya aygıt yolu".into()));
    }
    Ok(())
}

fn canonical(path: &Path) -> Result<PathBuf, ConvertError> {
    reject_device(path)?;
    let canon = fs::canonicalize(path)?;
    let canon = strip_verbatim(&canon);
    reject_device(&canon)?;
    Ok(canon)
}

/// `\\?\C:\x` (what canonicalize returns on Windows) -> `C:\x`.
/// `\\?\UNC\...` and other device forms are left alone and rejected later.
fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => p.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unc_and_device_paths() {
        for p in [
            r"\\server\share\a.png",
            r"\\?\C:\a.png",
            r"\\.\COM1",
            "//srv/a.png",
        ] {
            assert!(matches!(
                validate_input(Path::new(p)),
                Err(ConvertError::InvalidPath(_))
            ));
        }
    }

    #[test]
    fn strips_drive_verbatim_only() {
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\C:\a\b.png")),
            Path::new(r"C:\a\b.png")
        );
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\UNC\s\x")),
            Path::new(r"\\?\UNC\s\x")
        );
    }

    #[test]
    fn rejects_dirs_missing_empty_and_bad_ext() {
        let d = tempfile::tempdir().unwrap();
        assert!(validate_input(d.path()).is_err());
        assert!(matches!(
            validate_input(&d.path().join("yok.png")),
            Err(ConvertError::NotFound)
        ));
        let empty = d.path().join("e.png");
        fs::write(&empty, b"").unwrap();
        assert!(matches!(validate_input(&empty), Err(ConvertError::Corrupt)));
        let exe = d.path().join("a.exe");
        fs::write(&exe, b"MZ").unwrap();
        assert!(matches!(
            validate_input(&exe),
            Err(ConvertError::Unsupported)
        ));
    }

    #[test]
    fn accepts_valid_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.PNG");
        fs::write(&f, b"x").unwrap();
        let (p, fmt) = validate_input(&f).unwrap();
        assert_eq!(fmt, Format::Png);
        assert!(!p.to_string_lossy().starts_with(r"\\"));
    }
}
