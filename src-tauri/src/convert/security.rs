use std::fs;
use std::path::{Path, PathBuf};

use super::{ConvertError, Format, PathIssue};

/// Canonicalizes and checks an input path: no UNC/device paths, a real
/// non-empty file, extension on the allowlist.
pub fn validate_input(path: &Path) -> Result<(PathBuf, Format), ConvertError> {
    let canon = canonical(path)?;
    let meta = fs::metadata(&canon)?;
    if !meta.is_file() {
        return Err(ConvertError::InvalidPath(PathIssue::NotFile));
    }
    if meta.len() == 0 {
        return Err(ConvertError::EmptyFile);
    }
    let ext = canon.extension().and_then(|e| e.to_str()).unwrap_or("");
    let fmt = Format::from_ext(ext).ok_or(ConvertError::Unsupported)?;
    Ok((canon, fmt))
}

/// Same checks for a user-chosen output directory; it is created if missing.
pub fn validate_output_dir(dir: &Path) -> Result<PathBuf, ConvertError> {
    reject_device(dir)?;
    // a file in the way would otherwise surface as a raw "already exists" from Windows
    if dir.exists() && !dir.is_dir() {
        return Err(ConvertError::InvalidPath(PathIssue::NotDir));
    }
    fs::create_dir_all(dir)?;
    let canon = canonical(dir)?;
    if !canon.is_dir() {
        return Err(ConvertError::InvalidPath(PathIssue::NotDir));
    }
    Ok(canon)
}

/// Canonical form of an existing local directory (no UNC/device paths); nothing is created.
pub fn validate_dir(dir: &Path) -> Result<PathBuf, ConvertError> {
    let canon = canonical(dir)?;
    if !canon.is_dir() {
        return Err(ConvertError::InvalidPath(PathIssue::NotDir));
    }
    Ok(canon)
}

/// True for UNC, device and other non-drive-letter forms, whatever slash mix they use
/// (`\\h\s`, `//h/s`, `/\h\s`, `\\.\x`, `\\?\UNC\h\s`). Relative paths are not remote.
pub fn is_remote_or_device(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    let s = path.to_string_lossy();
    let lead = |c: char| c == '\\' || c == '/';
    let mut chars = s.chars();
    let slashes = chars.next().is_some_and(lead) && chars.next().is_some_and(lead);
    let prefix = matches!(
        path.components().next(),
        Some(Component::Prefix(p)) if !matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    );
    slashes || prefix
}

fn reject_device(path: &Path) -> Result<(), ConvertError> {
    if is_remote_or_device(path) {
        return Err(ConvertError::InvalidPath(PathIssue::Network));
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
    fn slash_mixes_of_unc_and_device_paths_are_remote() {
        for p in [
            r"\\h\s\x",
            "//h/s/x",
            r"/\h\s",
            r"\/h/s",
            r"\\.\NUL",
            r"\\?\UNC\h\s\x",
        ] {
            assert!(is_remote_or_device(Path::new(p)), "{p}");
        }
        for p in [r"C:\tmp\a.png", "C:/tmp/a.png", "a.png", r"sub\a.png"] {
            assert!(!is_remote_or_device(Path::new(p)), "{p}");
        }
    }

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
        assert!(matches!(
            validate_input(&empty),
            Err(ConvertError::EmptyFile)
        ));
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
