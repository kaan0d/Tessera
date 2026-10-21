//! HKCU-only Explorer context menu and autostart. No admin rights needed.

use std::io;
use std::path::Path;

use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS};
use winreg::RegKey;

use crate::app_info::APP_NAME;
use crate::convert::combine::MergeKind;
use crate::convert::{supported_targets, uses_ffmpeg, Format};
use crate::i18n::{self, Lang};
use crate::preset;
use crate::settings::{MenuSort, Settings};

/// Real registry root for file associations.
pub const REAL_ROOT: &str = r"Software\Classes";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Source extensions (must all be accepted by `Format::from_ext`).
pub const EXTS: [&str; 34] = [
    "png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "ico", "avif", "svg", "heic",
    "heif", "dng", "cr2", "cr3", "nef", "arw", "orf", "rw2", "raf", "pdf", "wav", "flac", "mp3",
    "ogg", "oga", "aac", "m4a", "mp4", "mkv", "avi", "webm", "mov",
];

fn label(t: Format, lang: Lang) -> String {
    i18n::fmt(lang, "menu.convert_to", &[("fmt", &t.ext().to_uppercase())])
}

fn menu_key(root: &str, ext: &str) -> String {
    format!(r"{root}\SystemFileAssociations\.{ext}\shell\{APP_NAME}")
}

/// Verbs shown for source `ext`, in menu order. Empty when the source is switched off.
fn verbs(ext: &str, s: &Settings) -> Vec<Format> {
    if !s.menu_sources.iter().any(|e| e == ext) {
        return vec![];
    }
    let src = Format::from_ext(ext);
    let mut t: Vec<Format> = supported_targets(ext)
        .into_iter()
        .filter(|&t| s.target_enabled(t))
        .filter(|&t| s.ffmpeg_ready || src.is_none_or(|f| !uses_ffmpeg(f, t)))
        .collect();
    if s.menu.sort == MenuSort::Alphabetical {
        t.sort_by_key(|t| t.ext());
    }
    t
}

/// Preset verbs for `ext`: (id, label), visible ones only.
fn preset_verbs(ext: &str, s: &Settings) -> Vec<(String, String)> {
    if !s.menu.presets_enabled || !s.menu_sources.iter().any(|e| e == ext) {
        return vec![];
    }
    preset::all(s)
        .into_iter()
        .filter(|p| !s.hidden_presets.contains(&p.id) && p.menu_target(ext, s).is_some())
        .map(|p| {
            let name = p.name(s.language);
            (p.id, name)
        })
        .collect()
}

/// Merge verb for `ext`: (kind, label).
fn merge_verb(ext: &str, s: &Settings) -> Option<(MergeKind, String)> {
    if !s.menu.merge || !s.menu_sources.iter().any(|e| e == ext) {
        return None;
    }
    let kind = MergeKind::for_ext(ext)?;
    let key = match kind {
        MergeKind::Pdf => "menu.merge_pdf",
        MergeKind::Audio => "menu.merge_audio",
    };
    Some((kind, i18n::t(s.language, key).to_string()))
}

/// Registers the cascading menu under `root` (idempotent: starts from a clean slate).
/// Explorer lists verbs by subkey name, so each key carries a two-digit position prefix.
pub fn install_at(root: &str, exe: &Path, s: &Settings) -> io::Result<()> {
    let exe = exe.to_string_lossy();
    if exe.contains('"') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad exe path"));
    }
    remove_at(root)?;
    let icon = format!(r#""{exe}",0"#);
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for ext in EXTS {
        let targets = verbs(ext, s);
        let presets = preset_verbs(ext, s);
        let merge = merge_verb(ext, s);
        if targets.is_empty() && presets.is_empty() && merge.is_none() {
            continue;
        }
        let (menu, _) = hkcu.create_subkey(menu_key(root, ext))?;
        menu.set_value("MUIVerb", &APP_NAME)?;
        menu.set_value("SubCommands", &"")?;
        if s.menu.icons {
            menu.set_value("Icon", &icon)?;
        }
        // (verb name, label, command arguments); a group's first verb gets a separator above
        // it (CommandFlags 0x20) unless the menu starts with that group
        let mut groups: Vec<Vec<(String, String, String)>> = vec![];
        groups.push(
            targets
                .into_iter()
                .map(|t| {
                    (
                        t.ext().to_string(),
                        label(t, s.language),
                        format!("--convert {}", t.ext()),
                    )
                })
                .collect(),
        );
        groups.push(
            presets
                .into_iter()
                .map(|(id, name)| (format!("preset_{id}"), name, format!("--preset {id}")))
                .collect(),
        );
        groups.push(
            merge
                .into_iter()
                .map(|(k, name)| {
                    (
                        format!("merge_{}", k.arg()),
                        name,
                        format!("--merge {}", k.arg()),
                    )
                })
                .collect(),
        );
        let mut i = 0;
        for group in groups.into_iter().filter(|g| !g.is_empty()) {
            for (n, (name, text, args)) in group.into_iter().enumerate() {
                let (verb, _) = menu.create_subkey(format!(r"shell\{i:02}_{name}"))?;
                verb.set_value("MUIVerb", &text)?;
                if i > 0 && n == 0 {
                    verb.set_value("CommandFlags", &0x20u32)?;
                }
                if s.menu.icons {
                    verb.set_value("Icon", &icon)?;
                }
                let (cmd, _) = verb.create_subkey("command")?;
                cmd.set_value("", &format!(r#""{exe}" {args} "%1""#))?;
                i += 1;
            }
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

pub fn install(s: &Settings) -> io::Result<()> {
    install_at(REAL_ROOT, &std::env::current_exe()?, s)
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

    fn test_root(tag: &str) -> String {
        format!(
            r"Software\{APP_NAME}Test{}{tag}\Classes",
            std::process::id()
        )
    }

    fn cleanup(root: &str) {
        let top = root.split('\\').take(2).collect::<Vec<_>>().join("\\");
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(top);
    }

    fn ready(ffmpeg_ready: bool) -> Settings {
        Settings {
            ffmpeg_ready,
            ..Settings::default()
        }
    }

    fn exts(v: Vec<Format>) -> Vec<&'static str> {
        v.into_iter().map(Format::ext).collect()
    }

    fn verb_names(menu: &RegKey) -> Vec<String> {
        menu.open_subkey("shell")
            .unwrap()
            .enum_keys()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn exts_are_allowlisted() {
        for e in EXTS {
            assert!(Format::from_ext(e).is_some(), "{e}");
        }
    }

    #[test]
    fn install_remove_idempotent_and_clean() {
        let root = test_root("a");
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let exe = Path::new(r"C:\Program Files\X\x.exe");
        install_at(&root, exe, &ready(false)).unwrap();
        install_at(&root, exe, &ready(false)).unwrap();
        assert!(hkcu.open_subkey(menu_key(&root, "mp4")).is_err());

        let png = hkcu.open_subkey(menu_key(&root, "png")).unwrap();
        assert_eq!(png.get_value::<String, _>("MUIVerb").unwrap(), APP_NAME);
        assert!(png
            .get_value::<String, _>("SubCommands")
            .unwrap()
            .is_empty());
        assert_eq!(
            png.get_value::<String, _>("Icon").unwrap(),
            r#""C:\Program Files\X\x.exe",0"#
        );
        // png's first target is jpg
        let verb = png.open_subkey(r"shell\00_jpg").unwrap();
        assert!(verb.get_value::<String, _>("Icon").is_ok());
        assert_eq!(
            verb.open_subkey("command")
                .unwrap()
                .get_value::<String, _>("")
                .unwrap(),
            r#""C:\Program Files\X\x.exe" --convert jpg "%1""#
        );
        // ffmpeg-only targets hidden until ffmpeg is ready
        let mp3 = hkcu.open_subkey(menu_key(&root, "mp3")).unwrap();
        assert_eq!(verb_names(&mp3), ["00_wav", "01_flac", "02_merge_audio"]);

        install_at(&root, exe, &ready(true)).unwrap();
        let mp3 = hkcu.open_subkey(menu_key(&root, "mp3")).unwrap();
        assert_eq!(
            verb_names(&mp3),
            ["00_wav", "01_flac", "02_ogg", "03_merge_audio"]
        );
        let mp4 = hkcu.open_subkey(menu_key(&root, "mp4")).unwrap();
        let names = verb_names(&mp4);
        // 10 formats, then email/discord/strip presets
        assert_eq!((names[0].as_str(), names.len()), ("00_mkv", 13));
        assert_eq!(names[10], "10_preset_email");

        remove_at(&root).unwrap();
        remove_at(&root).unwrap();
        assert!(hkcu
            .open_subkey(format!(r"{root}\SystemFileAssociations"))
            .is_err());
        cleanup(&root);
    }

    #[test]
    fn filters_and_ordering() {
        let mut s = ready(true);
        assert_eq!(
            exts(verbs("png", &s)),
            ["jpg", "webp", "bmp", "gif", "tiff", "ico", "avif", "pdf"]
        );
        s.menu.sort = MenuSort::Alphabetical;
        assert_eq!(
            exts(verbs("png", &s)),
            ["avif", "bmp", "gif", "ico", "jpg", "pdf", "tiff", "webp"]
        );
        s.enabled_targets = vec!["jpg".into(), "wav".into()];
        assert_eq!(verbs("png", &s), [Format::Jpg]);
        assert_eq!(verbs("mp3", &s), [Format::Wav]);
        s.menu_sources = vec!["mp3".into()];
        assert!(verbs("png", &s).is_empty() && !verbs("mp3", &s).is_empty());
        s.enabled_targets.clear();
        assert!(verbs("mp3", &s).is_empty());
    }

    #[test]
    fn icons_sort_and_sources_reach_registry() {
        let root = test_root("b");
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let mut s = ready(false);
        s.menu.icons = false;
        s.menu.sort = MenuSort::Alphabetical;
        s.menu_sources = vec!["png".into()];
        install_at(&root, Path::new(r"C:\x.exe"), &s).unwrap();
        let png = hkcu.open_subkey(menu_key(&root, "png")).unwrap();
        assert!(png.get_value::<String, _>("Icon").is_err());
        assert!(png
            .open_subkey(r"shell _avif")
            .unwrap()
            .get_value::<String, _>("Icon")
            .is_err());
        assert!(hkcu.open_subkey(menu_key(&root, "jpg")).is_err());
        remove_at(&root).unwrap();
        cleanup(&root);
    }

    #[test]
    fn labels_follow_the_language() {
        assert_eq!(label(Format::Jpg, Lang::En), "Convert to JPG");
        assert_eq!(label(Format::Jpg, Lang::Tr), "JPG olarak dönüştür");
        let root = test_root("d");
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for (lang, want) in [
            (Lang::En, "Convert to JPG"),
            (Lang::Tr, "JPG olarak dönüştür"),
        ] {
            let s = Settings {
                language: lang,
                ..ready(false)
            };
            install_at(&root, Path::new(r"C:\x.exe"), &s).unwrap();
            let png = hkcu.open_subkey(menu_key(&root, "png")).unwrap();
            let verb = png.open_subkey(r"shell\00_jpg").unwrap();
            assert_eq!(verb.get_value::<String, _>("MUIVerb").unwrap(), want);
        }
        remove_at(&root).unwrap();
        cleanup(&root);
    }

    #[test]
    fn rejects_quote_in_exe() {
        assert!(install_at(&test_root("c"), Path::new(r#"C:\a"b.exe"#), &ready(false)).is_err());
    }

    #[test]
    fn preset_and_merge_verbs_reach_registry() {
        let root = test_root("e");
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let exe = Path::new(r"C:\x.exe");
        install_at(&root, exe, &ready(false)).unwrap();
        let png = hkcu.open_subkey(menu_key(&root, "png")).unwrap();
        let names = verb_names(&png);
        // 8 formats, 6 presets, merge
        assert_eq!(names.len(), 15);
        assert_eq!(names[8], "08_preset_web");
        assert_eq!(names[14], "14_merge_pdf");
        let flags = |n: &str| {
            png.open_subkey(format!(r"shell\{n}"))
                .unwrap()
                .get_value::<u32, _>("CommandFlags")
                .ok()
        };
        assert_eq!(flags("08_preset_web"), Some(0x20));
        assert_eq!(flags("14_merge_pdf"), Some(0x20));
        assert_eq!(flags("09_preset_whatsapp"), None);
        assert_eq!(flags("00_jpg"), None);
        let cmd = |n: &str| {
            png.open_subkey(format!(r"shell\{n}\command"))
                .unwrap()
                .get_value::<String, _>("")
                .unwrap()
        };
        assert_eq!(cmd("10_preset_email"), r#""C:\x.exe" --preset email "%1""#);
        assert_eq!(cmd("14_merge_pdf"), r#""C:\x.exe" --merge pdf "%1""#);
        // ffmpeg-less menu hides video presets; mp4 has no verbs at all
        assert!(hkcu.open_subkey(menu_key(&root, "mp4")).is_err());

        // switches: no presets, no merge, hidden preset, source off
        let mut st = ready(true);
        st.menu.presets_enabled = false;
        st.menu.merge = false;
        install_at(&root, exe, &st).unwrap();
        let png = hkcu.open_subkey(menu_key(&root, "png")).unwrap();
        assert_eq!(verb_names(&png).len(), 8);
        st.menu.presets_enabled = true;
        st.hidden_presets = vec!["web".into(), "pdf".into()];
        st.menu_sources = vec!["png".into(), "heic".into()];
        install_at(&root, exe, &st).unwrap();
        let png = hkcu.open_subkey(menu_key(&root, "png")).unwrap();
        let names = verb_names(&png);
        assert_eq!(names[8], "08_preset_whatsapp");
        assert_eq!(names.len(), 8 + 4);
        // heic: no format verbs ... presets and no merge verb
        let heic = hkcu.open_subkey(menu_key(&root, "heic")).unwrap();
        assert!(verb_names(&heic).iter().all(|n| !n.contains("merge")));
        assert!(hkcu.open_subkey(menu_key(&root, "jpg")).is_err());
        // only presets: no separator flag on the first verb of the menu
        let mut only = ready(false);
        only.enabled_targets.clear();
        only.menu.merge = false;
        install_at(&root, exe, &only).unwrap();
        let png = hkcu.open_subkey(menu_key(&root, "png")).unwrap();
        let first = png.open_subkey(r"shell\00_preset_web").unwrap();
        assert!(first.get_value::<u32, _>("CommandFlags").is_err());
        remove_at(&root).unwrap();
        cleanup(&root);
    }

    #[test]
    fn preset_verb_labels_follow_the_language() {
        let tr = Settings {
            language: Lang::Tr,
            ..ready(false)
        };
        let v = preset_verbs("png", &tr);
        assert_eq!(v[0], ("web".to_string(), "Web için küçült".to_string()));
        assert_eq!(
            merge_verb("wav", &tr).unwrap().1,
            "Sesleri tek dosyada birleştir"
        );
        assert!(merge_verb("mp4", &tr).is_none());
    }
}
