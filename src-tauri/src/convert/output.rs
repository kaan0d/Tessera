use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{security, ConvertError, PathIssue};
use crate::i18n::{fmt, t, Lang};

#[derive(Debug, Clone, Default)]
pub enum OutDir {
    #[default]
    SameAsSource,
    Fixed(PathBuf),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Conflict {
    /// "name (1).ext"
    #[default]
    AutoRename,
    Overwrite,
    /// Output goes next to the source; the source is moved to the Recycle Bin.
    ReplaceOriginal,
}

#[derive(Debug, Clone, Default)]
pub struct OutputParams {
    pub dir: OutDir,
    pub conflict: Conflict,
    /// File name template with `{name}` `{format}` `{date}`; empty means `{name}`.
    /// Applied before the conflict logic, also under ReplaceOriginal (the original is
    /// recycled either way; when the name changes nothing collides).
    pub template: String,
}

pub const MAX_TEMPLATE_LEN: usize = 100;
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Today's local date as YYYY-MM-DD.
fn local_date() -> String {
    // SAFETY: GetLocalTime only fills the struct it is given.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

fn expand(template: &str, name: &str, format: &str, date: &str) -> String {
    template
        .replace("{name}", name)
        .replace("{format}", format)
        .replace("{date}", date)
}

/// Checks a template; the error is in `lang` and shown to the user as is.
pub fn validate_template(tpl: &str, lang: Lang) -> Result<(), String> {
    if tpl.is_empty() || tpl.chars().count() > MAX_TEMPLATE_LEN {
        let max = MAX_TEMPLATE_LEN.to_string();
        return Err(fmt(lang, "tpl.length", &[("max", &max)]));
    }
    let out = expand(tpl, "x", "x", "0000-00-00");
    if out.contains(['{', '}']) {
        return Err(t(lang, "tpl.unknown_token").into());
    }
    if let Some(c) = out
        .chars()
        .find(|c| c.is_control() || "<>:\"/\\|?*".contains(*c))
    {
        return Err(fmt(lang, "tpl.bad_char", &[("c", &format!("{c:?}"))]));
    }
    if out.ends_with(['.', ' ']) {
        return Err(t(lang, "tpl.trailing").into());
    }
    let base = out.split('.').next().unwrap_or("").trim_end();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(base)) {
        return Err(t(lang, "tpl.reserved").into());
    }
    Ok(())
}

/// Final output path for `source` converted to `ext`.
pub fn plan(source: &Path, ext: &str, p: &OutputParams) -> Result<PathBuf, ConvertError> {
    let src_dir = source.parent().unwrap_or_else(|| Path::new("."));
    let dir = match (&p.dir, p.conflict) {
        (OutDir::Fixed(d), c) if c != Conflict::ReplaceOriginal => {
            security::validate_output_dir(d)?
        }
        _ => src_dir.to_path_buf(),
    };
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or(ConvertError::InvalidPath(PathIssue::BadName))?;
    let stem = &if p.template.is_empty() || validate_template(&p.template, Lang::En).is_err() {
        stem.to_string()
    } else {
        expand(&p.template, stem, ext, &local_date())
    };
    let first = dir.join(format!("{stem}.{ext}"));
    // output equal to the source (same format, a transform job) must never overwrite it
    let is_source = first
        .to_string_lossy()
        .eq_ignore_ascii_case(&source.to_string_lossy());
    if !is_source && (p.conflict == Conflict::Overwrite || !first.exists()) {
        return Ok(first);
    }
    // ReplaceOriginal also renames: the original must survive until the new file is verified
    (1u32..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|c| !c.exists())
        .ok_or(ConvertError::InvalidPath(PathIssue::NoFreeName))
}

/// Creates a new, empty folder for a job with several outputs: `<stem><suffix>` next to the
/// source (or in the fixed folder), with " (n)" when the name is taken. Nothing is overwritten.
pub fn new_dir(source: &Path, suffix: &str, p: &OutputParams) -> Result<PathBuf, ConvertError> {
    let dir = match &p.dir {
        OutDir::Fixed(d) if p.conflict != Conflict::ReplaceOriginal => {
            security::validate_output_dir(d)?
        }
        _ => source
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    };
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or(ConvertError::InvalidPath(PathIssue::BadName))?;
    for n in 0u32..10_000 {
        let name = match n {
            0 => format!("{stem}{suffix}"),
            n => format!("{stem}{suffix} ({n})"),
        };
        match std::fs::create_dir(dir.join(&name)) {
            Ok(()) => return Ok(dir.join(name)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(ConvertError::InvalidPath(PathIssue::NoFreeName))
}

/// Writes to a temp file next to `dest`, checks it is non-empty, then renames it
/// into place. The temp file is removed on any failure.
pub fn write_atomic(
    dest: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> Result<(), ConvertError>,
) -> Result<(), ConvertError> {
    write_via_temp(dest, |tmp| {
        let mut w = BufWriter::new(File::options().write(true).create_new(true).open(tmp)?);
        write(&mut w)?;
        w.flush()?;
        w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(())
    })
}

/// Like `write_atomic`, but `produce` gets the temp path and creates the file itself
/// (used when another process writes the output).
// ponytail: AutoRename has a check-then-rename race with other writers; fine for one app instance.
pub fn write_via_temp(
    dest: &Path,
    produce: impl FnOnce(&Path) -> Result<(), ConvertError>,
) -> Result<(), ConvertError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("out");
    let tmp = dest.with_file_name(format!(".{name}.{nanos}.tmp"));

    let result = (|| {
        produce(&tmp)?;
        if fs::metadata(&tmp)?.len() == 0 {
            return Err(ConvertError::EmptyOutput);
        }
        fs::rename(&tmp, dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// True when `path` is on a fixed local disk (the only kind with a Recycle Bin we can trust).
fn on_fixed_drive(path: &Path) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    const DRIVE_FIXED: u32 = 3;
    let s = path.to_string_lossy();
    let Some(drive) = s.get(..2).filter(|d| d.ends_with(':')) else {
        return false;
    };
    let root: Vec<u16> = format!("{drive}{}", std::path::MAIN_SEPARATOR)
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: `root` is a NUL-terminated UTF-16 string that outlives the call
    unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) == DRIVE_FIXED }
}

/// Moves a file to the Recycle Bin.
pub fn recycle(path: &Path) -> Result<(), ConvertError> {
    // removable and network drives have no Recycle Bin: the shell would delete for good
    if !on_fixed_drive(path) {
        return Err(ConvertError::Recycle("no Recycle Bin on this drive".into()));
    }
    trash::delete(path).map_err(|e| ConvertError::Recycle(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overwrite_never_targets_the_source_file() {
        let dir = std::env::temp_dir().join(format!("tessera-plan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.jpg");
        std::fs::write(&src, b"x").unwrap();
        let out = plan(&src, "jpg", &params(Conflict::Overwrite)).unwrap();
        assert_ne!(out, src);
        assert_eq!(out, dir.join("a (1).jpg"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn params(c: Conflict) -> OutputParams {
        OutputParams {
            dir: OutDir::SameAsSource,
            conflict: c,
            ..Default::default()
        }
    }

    #[test]
    fn auto_rename_counts_up() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("foto.jpg");
        fs::write(&src, b"x").unwrap();
        let p = plan(&src, "png", &params(Conflict::AutoRename)).unwrap();
        assert_eq!(p, d.path().join("foto.png"));
        fs::write(&p, b"x").unwrap();
        let p = plan(&src, "png", &params(Conflict::AutoRename)).unwrap();
        assert_eq!(p, d.path().join("foto (1).png"));
        fs::write(&p, b"x").unwrap();
        let p = plan(&src, "png", &params(Conflict::AutoRename)).unwrap();
        assert_eq!(p, d.path().join("foto (2).png"));
    }

    #[test]
    fn overwrite_reuses_name() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("a.jpg");
        fs::write(d.path().join("a.png"), b"x").unwrap();
        let p = plan(&src, "png", &params(Conflict::Overwrite)).unwrap();
        assert_eq!(p, d.path().join("a.png"));
    }

    #[test]
    fn fixed_dir_is_used_and_created() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("a.jpg");
        let out = d.path().join("cikti");
        let p = OutputParams {
            dir: OutDir::Fixed(out.clone()),
            conflict: Conflict::AutoRename,
            ..Default::default()
        };
        let r = plan(&src, "png", &p).unwrap();
        assert_eq!(r.file_name().unwrap(), "a.png");
        assert!(out.is_dir());
    }

    #[test]
    fn template_names_and_conflicts() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("foto.jpg");
        let mut p = params(Conflict::AutoRename);
        p.template = "{name}_{format}".into();
        let r = plan(&src, "png", &p).unwrap();
        assert_eq!(r, d.path().join("foto_png.png"));
        fs::write(&r, b"x").unwrap();
        assert_eq!(
            plan(&src, "png", &p).unwrap(),
            d.path().join("foto_png (1).png")
        );
        p.template = "{date}-{name}".into();
        let n = plan(&src, "png", &p).unwrap();
        let n = n.file_name().unwrap().to_str().unwrap();
        assert!(n.ends_with("-foto.png") && n.len() == "2026-01-01-foto.png".len());
    }

    #[test]
    fn template_validation() {
        for ok in ["{name}", "{name} ({format})", "{date} {name}", "a.b{name}"] {
            assert!(validate_template(ok, Lang::En).is_ok(), "{ok}");
        }
        for bad in [
            "", "a/b", "a\\b", "x:y", "a?", "a*", "a\"b", "<x>", "a|b", "{nam}", "{", "con",
            "NUL.txt", "{name}.", "{name} ", "a	b",
        ] {
            assert!(validate_template(bad, Lang::En).is_err(), "{bad:?}");
        }
        assert!(validate_template(&"a".repeat(100), Lang::En).is_ok());
        assert!(validate_template(&"a".repeat(101), Lang::En).is_err());
    }

    #[test]
    fn atomic_write_cleans_up_on_error_and_rejects_empty() {
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("o.bin");
        assert!(write_atomic(&dest, |_| Err(ConvertError::Corrupt)).is_err());
        assert!(write_atomic(&dest, |_| Ok(())).is_err());
        assert!(!dest.exists());
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 0);
        write_atomic(&dest, |w| Ok(w.write_all(b"hi")?)).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"hi");
    }
}
