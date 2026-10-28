//! Document support: LibreOffice as a headless converter, never as an app the user opens.
//! Downloaded only when the user asks (Settings > Advanced): one pinned MSI, SHA-256 verified,
//! unpacked with an administrative install (no admin rights, nothing registered) and trimmed
//! to what conversion needs. Lives in `<data>\libreoffice\`; an installed LibreOffice is used
//! instead when there is one. Notes and measurements: development/OFFICE.md.

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

use crate::convert::{ConvertError, Format, Options};
use crate::ffmpeg::{download_capped, verify_sha256, Download, FfError};
use crate::settings::data_dir;

/// LibreOffice 26.2.6 (build 26.2.6.3), x86-64 MSI, MPL-2.0. The mirror link goes away when the
/// version leaves "stable"; the archive keeps it. Same file, same hash.
pub const SOURCES: [&str; 2] = [
    "https://download.documentfoundation.org/libreoffice/stable/26.2.6/win/x86_64/LibreOffice_26.2.6_Win_x86-64.msi",
    "https://downloadarchive.documentfoundation.org/libreoffice/old/26.2.6.3/win/x86_64/LibreOffice_26.2.6.3_Win_x86-64.msi",
];
pub const SHA256: &str = "f9877032fd908beb9c0ddf06df4af5c2e85f419c42e14876c4cce5aae5fb2660";
const EXPECTED_SIZE: u64 = 373_252_096;
const MAX_MSI: u64 = 700 << 20;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Our own copy (only this one is ever removed).
pub fn own_dir() -> PathBuf {
    data_dir().join("libreoffice")
}

/// The isolated user profile every conversion runs with (never the user's own). Its name must
/// not start with `libreoffice`: a profile URL that begins with the program folder's URL makes
/// soffice exit at once without converting anything.
pub fn profile_dir() -> PathBuf {
    data_dir().join("office-profile")
}

/// Where the converter comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Own,
    System,
}

/// The soffice.exe to run: our download first, then an installed LibreOffice.
pub fn soffice() -> Option<(PathBuf, Source)> {
    let own = own_dir().join("program").join("soffice.exe");
    if own.is_file() {
        return Some((own, Source::Own));
    }
    system_soffice().map(|p| (p, Source::System))
}

/// An installed LibreOffice: the UNO install path in the registry, else the default folder.
fn system_soffice() -> Option<PathBuf> {
    let from_registry = [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
        .into_iter()
        .filter_map(|root| {
            RegKey::predef(root)
                .open_subkey(r"SOFTWARE\LibreOffice\UNO\InstallPath")
                .and_then(|k| k.get_value::<String, _>(""))
                .ok()
        })
        .map(|dir| PathBuf::from(dir).join("soffice.exe"));
    let default = std::env::var_os("ProgramFiles")
        .map(|p| PathBuf::from(p).join(r"LibreOffice\program\soffice.exe"));
    from_registry.chain(default).find(|p| p.is_file())
}

/// Converts with whichever LibreOffice `soffice` finds; `NeedsOffice` when there is none.
pub fn convert(
    input: &Path,
    target: Format,
    opts: &Options,
    progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    let (exe, _) = soffice().ok_or(ConvertError::NeedsOffice)?;
    crate::convert::office::convert_with_office(&exe, &profile_dir(), input, target, opts, progress)
}

/// Where an unpacked file goes inside our copy, or None when conversion does not need it:
/// help, readmes, UI translations, spelling and thesaurus files. Hyphenation patterns stay
/// (they move line breaks). The VC++ runtime goes next to soffice.exe and the bundled fonts
/// into LibreOffice's own font folder, since nothing is installed system-wide.
pub fn place(rel: &Path) -> Option<PathBuf> {
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    let name = parts.last()?.as_str();
    let ext = rel
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let rest = || rel.components().skip(1).collect::<PathBuf>();
    let p: Vec<&str> = parts.iter().map(String::as_str).collect();
    match p.as_slice() {
        ["help" | "readmes" | "system", ..] => None,
        _ if ext == "msi" => None,
        ["system64", ..] => Some(Path::new("program").join(rest())),
        ["fonts", ..] => Some(Path::new(r"share\fonts\truetype").join(rest())),
        ["program", "resource", ..] => None,
        ["share", "registry", "res", ..] if name != "registry_en-us.xcd" => None,
        ["share", "extensions", "nlpsolver" | "wiki-publisher", ..] => None,
        ["share", "extensions", ..]
            if matches!(ext.as_str(), "dic" | "aff" | "idx" | "dat")
                && !name.starts_with("hyph") =>
        {
            None
        }
        _ => Some(rel.to_path_buf()),
    }
}

/// Moves every kept file of `from` to its `place` under `to` (same drive, so renames).
fn move_kept(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut stack = vec![from.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir)? {
            let e = e?;
            let path = e.path();
            if e.file_type()?.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(from).unwrap_or(&path);
            if let Some(dst) = place(rel).map(|r| to.join(r)) {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::rename(&path, &dst)?;
            }
        }
    }
    Ok(())
}

/// `msiexec /a`: unpacks the MSI into `target` without installing or registering anything.
fn unpack(msi: &Path, target: &Path) -> Result<(), FfError> {
    // raw_arg: msiexec wants PROPERTY="value", which Command's own quoting would break;
    // data paths never contain quotes (install::normalize_dir refuses them)
    let status = Command::new("msiexec.exe")
        .raw_arg(format!(
            r#"/a "{}" /qn TARGETDIR="{}""#,
            msi.display(),
            target.display()
        ))
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| FfError::with("lo.unpack", e))?;
    if status.success() {
        Ok(())
    } else {
        // 1618: another installation is running
        Err(FfError::with("lo.unpack", status.code().unwrap_or(-1)))
    }
}

/// Download, verify, unpack and trim into `own_dir`, reporting percent (0..=100).
/// A previous copy is replaced only once the new one is complete.
pub fn install(progress: &mut dyn FnMut(f32)) -> Result<(), FfError> {
    let work = data_dir().join("libreoffice.work");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| FfError::with("ff.mkdir", e))?;
    let result = install_in(&work, progress);
    let _ = fs::remove_dir_all(&work);
    result
}

fn install_in(work: &Path, progress: &mut dyn FnMut(f32)) -> Result<(), FfError> {
    let msi = work.join("libreoffice.msi");
    let mut last = FfError::new("ff.start");
    let mut fetched = false;
    for url in SOURCES {
        progress(0.0);
        let got = download_capped(url, &msi, MAX_MSI, EXPECTED_SIZE, &mut |f| {
            progress(f * 75.0)
        })
        .and_then(|()| verify_sha256(&msi, SHA256));
        match got {
            Ok(()) => {
                fetched = true;
                break;
            }
            Err(e) => last = e,
        }
    }
    if !fetched {
        return Err(last);
    }
    progress(78.0);
    let raw = work.join("raw");
    unpack(&msi, &raw)?;
    progress(95.0);
    let kept = work.join("kept");
    move_kept(&raw, &kept).map_err(|e| FfError::with("lo.unpack", e))?;
    if !kept.join(r"program\soffice.exe").is_file() {
        return Err(FfError::new("lo.missing"));
    }
    let dir = own_dir();
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&kept, &dir).map_err(|e| FfError::with("ff.write", e))?;
    progress(100.0);
    Ok(())
}

/// Deletes our copy and its profile; an installed LibreOffice is never touched.
pub fn remove() -> std::io::Result<()> {
    let _ = fs::remove_dir_all(profile_dir());
    match fs::remove_dir_all(own_dir()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Runs `install`, retrying twice with a growing pause; updates `dl` as it goes.
pub fn install_with_retries(dl: &Download) -> Result<(), FfError> {
    let mut last = FfError::new("ff.start");
    for pause in [0u64, 3, 10] {
        std::thread::sleep(Duration::from_secs(pause));
        match install(&mut |p| dl.set_percent(p)) {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rel: &str) -> Option<String> {
        place(Path::new(rel)).map(|p| p.to_string_lossy().into_owned())
    }

    #[test]
    fn conversion_files_stay_and_extras_go() {
        assert_eq!(
            at(r"program\soffice.exe").as_deref(),
            Some(r"program\soffice.exe")
        );
        assert_eq!(
            at(r"System64\msvcp140.dll").as_deref(),
            Some(r"program\msvcp140.dll")
        );
        assert_eq!(
            at(r"Fonts\DejaVuSans.ttf").as_deref(),
            Some(r"share\fonts\truetype\DejaVuSans.ttf")
        );
        assert!(at(r"share\registry\res\registry_en-US.xcd").is_some());
        assert!(at(r"share\registry\main.xcd").is_some());
        assert!(at(r"share\extensions\dict-tr\hyph_tr_TR.dic").is_some());
        assert!(at(r"share\extensions\dict-tr\dictionaries.xcu").is_some());
        for gone in [
            r"help\en-US\a.html",
            r"readmes\readme_en-US.txt",
            r"System\msvcp140.dll",
            r"LibreOffice_26.2.6_Win_x86-64.msi",
            r"program\resource\tr\LC_MESSAGES\sw.mo",
            r"share\registry\res\registry_tr.xcd",
            r"share\extensions\dict-tr\tr_TR.dic",
            r"share\extensions\dict-en\th_en_US_v2.dat",
            r"share\extensions\wiki-publisher\x.jar",
        ] {
            assert_eq!(at(gone), None, "{gone}");
        }
    }

    #[test]
    fn kept_files_move_and_the_rest_stays_behind() {
        let t = tempfile::tempdir().unwrap();
        let (raw, kept) = (t.path().join("raw"), t.path().join("kept"));
        for f in [r"program\soffice.exe", r"help\x.html", r"Fonts\a.ttf"] {
            let p = raw.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"x").unwrap();
        }
        move_kept(&raw, &kept).unwrap();
        assert!(kept.join(r"program\soffice.exe").is_file());
        assert!(kept.join(r"share\fonts\truetype\a.ttf").is_file());
        assert!(!kept.join("help").exists());
        assert!(raw.join(r"help\x.html").is_file());
    }
}
