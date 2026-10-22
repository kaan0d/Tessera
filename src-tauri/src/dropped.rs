//! Files dropped on the exe, sent through "Send To" or opened with it: the chooser window
//! asks what to do with them. Everything that decides what is offered lives here (pure, tested);
//! `lib.rs` owns the window and the commands.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::convert::combine::{MergeKind, MAX_FILES as MERGE_MAX};
use crate::convert::{needs_ffmpeg, supported_targets, uses_ffmpeg, Format};
use crate::folder;
use crate::preset::PresetOptions;
use crate::settings::Settings;

/// Most files one drop handles (same cap as a folder verb).
pub const MAX_FILES: usize = folder::MAX_FILES;

/// What the user picked in the chooser.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Choice {
    Target {
        target: String,
    },
    Preset {
        id: String,
    },
    Merge {
        merge: String,
    },
    /// One-off settings from the chooser's custom page. `target` is a format or "same".
    Custom {
        target: String,
        options: PresetOptions,
    },
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct DropTarget {
    pub ext: &'static str,
    pub kind: &'static str,
    pub needs_ffmpeg: bool,
    /// When it needs ffmpeg, ffmpeg is ready. (Formats switched off in the settings are not
    /// offered at all.)
    pub allowed: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Choices {
    /// Kinds present among the convertible files, in order of appearance.
    pub kinds: Vec<&'static str>,
    /// Targets every convertible file can become.
    pub targets: Vec<DropTarget>,
    /// Presets that apply to at least one file (the rest of the files are skipped).
    pub preset_ids: Vec<String>,
    /// "pdf" and/or "audio": the whole selection can be merged into one file.
    pub merge: Vec<&'static str>,
    /// Convertible files.
    pub count: usize,
    /// Dropped files with an unknown extension.
    pub skipped: usize,
    /// Names of the first few skipped files, so the chooser can say which ones.
    pub skipped_names: Vec<String>,
}

fn ext_of(p: &Path) -> &str {
    p.extension().and_then(|e| e.to_str()).unwrap_or("")
}

/// Existing files from what Windows handed us: folders contribute their direct files,
/// duplicates and non-files go, at most `MAX_FILES`.
pub fn collect(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for p in paths {
        let found = if p.is_dir() {
            let supported = |f: &Path| Format::from_ext(ext_of(f)).is_some();
            folder::scan(p, false, MAX_FILES, supported, |_| false).files
        } else if p.is_file() {
            vec![p.clone()]
        } else {
            vec![]
        };
        for f in found {
            if out.len() < MAX_FILES && seen.insert(f.to_string_lossy().to_lowercase()) {
                out.push(f);
            }
        }
    }
    out
}

/// Files with a known source extension, and how many had none.
pub fn split(files: &[PathBuf]) -> (Vec<(&PathBuf, Format)>, usize) {
    let known: Vec<_> = files
        .iter()
        .filter_map(|f| Format::from_ext(ext_of(f)).map(|fmt| (f, fmt)))
        .collect();
    let skipped = files.len() - known.len();
    (known, skipped)
}

/// Merge kind that fits the whole selection (two or more files, all one kind).
fn merge_kinds(known: &[(&PathBuf, Format)]) -> Vec<&'static str> {
    if known.len() < 2 || known.len() > MERGE_MAX {
        return vec![];
    }
    let kinds: Vec<_> = known
        .iter()
        .map(|(f, _)| MergeKind::for_ext(ext_of(f)))
        .collect();
    match kinds[0] {
        Some(k) if kinds.iter().all(|x| *x == Some(k)) => vec![k.arg()],
        _ => vec![],
    }
}

pub fn choices(files: &[PathBuf], s: &Settings) -> Choices {
    let (known, skipped) = split(files);
    let skipped_names = files
        .iter()
        .filter(|f| Format::from_ext(ext_of(f)).is_none())
        .take(3)
        .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    let mut kinds: Vec<&'static str> = vec![];
    for (_, f) in &known {
        if !kinds.contains(&f.kind()) {
            kinds.push(f.kind());
        }
    }
    let targets = if known.is_empty() {
        vec![]
    } else {
        Format::ALL
            .iter()
            .copied()
            .filter(|t| s.target_enabled(*t))
            // a file already in the target format is simply left out of that run
            .filter(|t| {
                known
                    .iter()
                    .all(|(p, f)| f == t || supported_targets(ext_of(p)).contains(t))
                    && known.iter().any(|(_, f)| f != t)
            })
            .map(|t| {
                let ff = needs_ffmpeg(t) || known.iter().any(|(_, f)| uses_ffmpeg(*f, t));
                DropTarget {
                    ext: t.ext(),
                    kind: t.kind(),
                    needs_ffmpeg: ff,
                    allowed: s.ffmpeg_ready || !ff,
                }
            })
            .collect()
    };
    let preset_ids = crate::preset::all(s)
        .into_iter()
        .filter(|p| !s.hidden_presets.contains(&p.id))
        .filter(|p| {
            known
                .iter()
                .any(|(f, _)| p.menu_target(ext_of(f), s).is_some())
        })
        .map(|p| p.id)
        .collect();
    Choices {
        kinds,
        targets,
        preset_ids,
        merge: merge_kinds(&known),
        count: known.len(),
        skipped,
        skipped_names,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn ready() -> Settings {
        Settings {
            ffmpeg_ready: true,
            ..Settings::default()
        }
    }

    fn exts(c: &Choices) -> Vec<&'static str> {
        c.targets.iter().map(|t| t.ext).collect()
    }

    #[test]
    fn images_get_image_targets_presets_and_pdf_merge() {
        let c = choices(&p(&["a.png", "b.JPG", "c.txt"]), &ready());
        assert_eq!((c.count, c.skipped), (2, 1));
        assert_eq!(c.kinds, ["image"]);
        // a file already in the target format is skipped in that run, so both stay on offer
        assert_eq!(
            exts(&c),
            ["png", "jpg", "webp", "bmp", "gif", "tiff", "ico", "avif", "pdf"]
        );
        assert!(c.targets.iter().all(|t| t.allowed && !t.needs_ffmpeg));
        assert!(c.preset_ids.contains(&"web".to_string()));
        assert_eq!(c.merge, ["pdf"]);
    }

    #[test]
    fn mixed_kinds_share_nothing_and_cannot_merge() {
        let c = choices(&p(&["a.png", "b.mp3"]), &ready());
        assert_eq!(c.kinds, ["image", "audio"]);
        assert!(c.targets.is_empty() && c.merge.is_empty());
        // but presets still apply to the file they fit
        assert!(c.preset_ids.contains(&"web".to_string()));
        let none = choices(&p(&["x.txt"]), &ready());
        assert_eq!((none.count, none.skipped), (0, 1));
        assert!(none.targets.is_empty() && none.kinds.is_empty());
    }

    #[test]
    fn audio_targets_follow_ffmpeg_and_settings() {
        let off = choices(&p(&["a.wav", "b.flac"]), &Settings::default());
        let get = |c: &Choices, e: &str| c.targets.iter().find(|t| t.ext == e).map(|t| t.allowed);
        assert_eq!(get(&off, "mp3"), Some(false));
        assert_eq!(get(&off, "ogg"), Some(false));
        assert_eq!(off.merge, ["audio"]);
        let on = choices(&p(&["a.wav", "b.flac"]), &ready());
        assert_eq!(get(&on, "mp3"), Some(true));
        let s = Settings {
            enabled_targets: vec!["mp3".into()],
            ..ready()
        };
        let c = choices(&p(&["a.wav"]), &s);
        assert_eq!(get(&c, "mp3"), Some(true));
        assert_eq!(get(&c, "flac"), None); // switched off in the settings: not offered
        assert_eq!(exts(&c), ["mp3"]);
        // a single file never offers a merge
        assert!(c.merge.is_empty());
    }

    #[test]
    fn video_needs_ffmpeg_for_everything() {
        let c = choices(&p(&["a.mp4"]), &Settings::default());
        assert!(c.targets.iter().all(|t| t.needs_ffmpeg && !t.allowed));
        assert!(!c.preset_ids.contains(&"web".to_string()));
    }

    #[test]
    fn hidden_presets_are_not_offered() {
        let s = Settings {
            hidden_presets: vec!["web".into()],
            ..ready()
        };
        assert!(!choices(&p(&["a.png"]), &s)
            .preset_ids
            .contains(&"web".to_string()));
    }

    #[test]
    fn choice_json_shapes() {
        let c: Choice = serde_json::from_str(r#"{"kind":"target","target":"jpg"}"#).unwrap();
        assert_eq!(
            c,
            Choice::Target {
                target: "jpg".into()
            }
        );
        let c: Choice = serde_json::from_str(r#"{"kind":"preset","id":"web"}"#).unwrap();
        assert_eq!(c, Choice::Preset { id: "web".into() });
        let c: Choice = serde_json::from_str(r#"{"kind":"merge","merge":"pdf"}"#).unwrap();
        assert_eq!(
            c,
            Choice::Merge {
                merge: "pdf".into()
            }
        );
        assert!(serde_json::from_str::<Choice>(r#"{"kind":"x"}"#).is_err());
    }

    #[test]
    fn collect_keeps_files_expands_folders_and_dedups() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        for n in ["a.png", "b.mp3", "c.txt"] {
            std::fs::write(r.join(n), b"x").unwrap();
        }
        std::fs::create_dir(r.join("sub")).unwrap();
        std::fs::write(r.join("sub").join("deep.png"), b"x").unwrap();
        let got = collect(&[
            r.join("a.png"),
            r.join("A.PNG"),
            r.join("missing.png"),
            r.to_path_buf(),
        ]);
        let names: Vec<_> = got
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().to_lowercase())
            .collect();
        // the folder adds its convertible direct files (no c.txt, no sub/deep.png)
        assert_eq!(names, ["a.png", "b.mp3"]);
    }
}
