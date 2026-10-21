//! HKCU-only Explorer context menu and autostart. No admin rights needed.

use std::io;
use std::path::Path;

use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS};
use winreg::RegKey;

use crate::app_info::APP_NAME;
use crate::convert::{needs_ffmpeg, supported_targets, Format};

/// Real registry root for file associations.
pub const REAL_ROOT: &str = r"Software\Classes";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Source extensions (must all be accepted by `Format::from_ext`).
const EXTS: [&str; 16] = [
    "png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "ico", "wav", "flac", "mp3", "ogg",
    "oga", "aac", "m4a",
];

fn label(t: Format) -> String {
    format!("{}'ye dönüştür", t.ext().to_uppercase())
}

fn menu_key(root: &str, ext: &str) -> String {
    format!(r"{root}\SystemFileAssociations\.{ext}\shell\{APP_NAME}")
}

/// Registers the cascading menu under `root` (idempotent: starts from a clean slate).
pub fn install_at(root: &str, exe: &Path, ffmpeg_ready: bool) -> io::Result<()> {
    let exe = exe.to_string_lossy();
    if exe.contains('"') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad exe path"));
    }
    remove_at(root)?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for ext in EXTS {
        let targets: Vec<Format> = supported_targets(ext)
            .into_iter()
            .filter(|&t| ffmpeg_ready || !needs_ffmpeg(t))
            .collect();
        if targets.is_empty() {
            continue;
        }
        let (menu, _) = hkcu.create_subkey(menu_key(root, ext))?;
        menu.set_value("MUIVerb", &APP_NAME)?;
        menu.set_value("SubCommands", &"")?;
        for t in targets {
            let (verb, _) = menu.create_subkey(format!(r"shell\{}", t.ext()))?;
            verb.set_value("MUIVerb", &label(t))?;
            let (cmd, _) = verb.create_subkey("command")?;
            cmd.set_value("", &format!(r#""{exe}" --convert {} "%1""#, t.ext()))?;
        }
    }
    Ok(())
}

/// Removes every key `install_at` made; missing keys are fine. Needs no running app.
pub fn remove_at(root: &str) -> io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for ext in EXTS {
        match hkcu.delete_subkey_all(menu_key(root, ext)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        }
        // Drop the now-empty parents; a non-empty one (other tools' verbs) makes this fail, which is fine.
        let shell = format!(r"{root}\SystemFileAssociations\.{ext}\shell");
        let _ = hkcu.delete_subkey(&shell);
        let _ = hkcu.delete_subkey(format!(r"{root}\SystemFileAssociations\.{ext}"));
    }
    let _ = hkcu.delete_subkey(format!(r"{root}\SystemFileAssociations"));
    Ok(())
}

pub fn install(ffmpeg_ready: bool) -> io::Result<()> {
    install_at(REAL_ROOT, &std::env::current_exe()?, ffmpeg_ready)
}

pub fn remove() -> io::Result<()> {
    remove_at(REAL_ROOT)
}

pub fn set_autostart(enabled: bool) -> io::Result<()> {
    let (run, _) =
        RegKey::predef(HKEY_CURRENT_USER).create_subkey_with_flags(RUN_KEY, KEY_ALL_ACCESS)?;
    if enabled {
        let exe = std::env::current_exe()?;
        run.set_value(APP_NAME, &format!(r#""{}" --background"#, exe.display()))
    } else {
        match run.delete_value(APP_NAME) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root() -> String {
        format!(r"Software\{APP_NAME}Test{}\Classes", std::process::id())
    }

    fn cleanup(root: &str) {
        let top = root.split('\\').take(2).collect::<Vec<_>>().join("\\");
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(top);
    }

    #[test]
    fn exts_are_allowlisted() {
        for e in EXTS {
            assert!(Format::from_ext(e).is_some(), "{e}");
        }
    }

    #[test]
    fn install_remove_idempotent_and_clean() {
        let root = test_root();
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let exe = Path::new(r"C:\Program Files\X\x.exe");
        install_at(&root, exe, false).unwrap();
        install_at(&root, exe, false).unwrap();

        let png = hkcu.open_subkey(menu_key(&root, "png")).unwrap();
        assert_eq!(png.get_value::<String, _>("MUIVerb").unwrap(), APP_NAME);
        assert!(png
            .get_value::<String, _>("SubCommands")
            .unwrap()
            .is_empty());
        let cmd = png.open_subkey(r"shell\jpg\command").unwrap();
        assert_eq!(
            cmd.get_value::<String, _>("").unwrap(),
            r#""C:\Program Files\X\x.exe" --convert jpg "%1""#
        );
        // ffmpeg-only targets hidden until ffmpeg is ready
        let mp3 = hkcu.open_subkey(menu_key(&root, "mp3")).unwrap();
        assert!(mp3.open_subkey(r"shell\ogg").is_err() && mp3.open_subkey(r"shell\wav").is_ok());

        install_at(&root, exe, true).unwrap();
        let mp3 = hkcu.open_subkey(menu_key(&root, "mp3")).unwrap();
        assert!(mp3.open_subkey(r"shell\ogg").is_ok());

        remove_at(&root).unwrap();
        remove_at(&root).unwrap();
        assert!(hkcu
            .open_subkey(format!(r"{root}\SystemFileAssociations"))
            .is_err());
        cleanup(&root);
    }

    #[test]
    fn rejects_quote_in_exe() {
        assert!(install_at(&test_root(), Path::new(r#"C:\a"b.exe"#), false).is_err());
    }
}
