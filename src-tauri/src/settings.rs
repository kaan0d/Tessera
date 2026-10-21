use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app_info::APP_NAME;
use crate::convert::output::validate_template;
use crate::convert::{Conflict, Format, Options, OutDir, OutputParams, Quality};
use crate::i18n::Lang;
use crate::panel::Corner;
use crate::shell::EXTS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputDir {
    SameAsSource,
    Fixed { path: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictMode {
    AutoRename,
    Overwrite,
    ReplaceOriginal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Panel {
    pub corner: Corner,
    /// How long the panel stays after the queue empties, 2..=30.
    pub seconds: u8,
    pub windows_toast: bool,
}

impl Default for Panel {
    fn default() -> Self {
        Panel {
            corner: Corner::TopRight,
            seconds: 7,
            windows_toast: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MenuSort {
    #[default]
    Default,
    Alphabetical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MenuOpts {
    pub icons: bool,
    pub sort: MenuSort,
}

impl Default for MenuOpts {
    fn default() -> Self {
        MenuOpts {
            icons: true,
            sort: MenuSort::Default,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// UI and message language; anything unknown reads as English.
    pub language: Lang,
    pub autostart: bool,
    pub output_dir: OutputDir,
    pub conflict: ConflictMode,
    /// Target extensions the user allows (menu and queue).
    pub enabled_targets: Vec<String>,
    /// Source extensions that get a context menu.
    pub menu_sources: Vec<String>,
    pub name_template: String,
    pub quality: Quality,
    pub panel: Panel,
    pub menu: MenuOpts,
    /// Old top-level setting; read once and folded into `quality.jpeg`, never written.
    #[serde(rename = "jpeg_quality", skip_serializing)]
    pub legacy_jpeg_quality: Option<u8>,
    /// Owned by the ffmpeg stage; `set_settings` never overwrites it from the UI.
    pub ffmpeg_ready: bool,
    /// The first-run ffmpeg info card was dismissed.
    pub ffmpeg_notice_seen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: Lang::En,
            autostart: false,
            output_dir: OutputDir::SameAsSource,
            conflict: ConflictMode::AutoRename,
            enabled_targets: Format::ALL.iter().map(|f| f.ext().to_string()).collect(),
            menu_sources: EXTS.iter().map(|e| e.to_string()).collect(),
            name_template: "{name}".into(),
            quality: Quality::default(),
            panel: Panel::default(),
            menu: MenuOpts::default(),
            legacy_jpeg_quality: None,
            ffmpeg_ready: false,
            ffmpeg_notice_seen: false,
        }
    }
}

/// Lowercased, deduplicated, restricted to `allowed`; order kept.
fn clean_list(list: &mut Vec<String>, allowed: impl Fn(&str) -> bool) {
    let mut seen = std::collections::HashSet::new();
    list.iter_mut().for_each(|e| *e = e.trim().to_lowercase());
    list.retain(|e| allowed(e) && seen.insert(e.clone()));
}

impl Settings {
    /// Clamps ranges, drops unknown extensions, resets an invalid template and folds in the
    /// legacy JPEG setting.
    pub fn normalized(mut self) -> Self {
        if let Some(q) = self.legacy_jpeg_quality.take() {
            self.quality.jpeg = q;
        }
        self.quality = self.quality.clamped();
        self.panel.seconds = self.panel.seconds.clamp(2, 30);
        clean_list(&mut self.enabled_targets, |e| {
            Format::ALL.iter().any(|f| f.ext() == e)
        });
        clean_list(&mut self.menu_sources, |e| EXTS.contains(&e));
        if validate_template(&self.name_template, self.language).is_err() {
            self.name_template = "{name}".into();
        }
        self
    }

    /// Errors the UI shows; call on the raw input, `normalized` would hide them.
    pub fn validate(&self) -> Result<(), String> {
        validate_template(&self.name_template, self.language)
    }

    pub fn target_enabled(&self, t: Format) -> bool {
        self.enabled_targets.iter().any(|e| e == t.ext())
    }

    pub fn to_options(&self) -> Options {
        Options {
            quality: self.quality.clamped(),
            output: OutputParams {
                dir: match &self.output_dir {
                    OutputDir::SameAsSource => OutDir::SameAsSource,
                    OutputDir::Fixed { path } => OutDir::Fixed(path.clone()),
                },
                conflict: match self.conflict {
                    ConflictMode::AutoRename => Conflict::AutoRename,
                    ConflictMode::Overwrite => Conflict::Overwrite,
                    ConflictMode::ReplaceOriginal => Conflict::ReplaceOriginal,
                },
                template: self.name_template.clone(),
            },
            ..Options::default()
        }
    }
}

pub fn settings_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join(APP_NAME).join("settings.json")
}

/// Missing or corrupt file gives defaults.
pub fn load_from(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
        .unwrap_or_default()
        .normalized()
}

/// Writes a temp file then renames it over the target.
pub fn save_to(path: &Path, s: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_and_missing_give_defaults() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        assert_eq!(load_from(&p), Settings::default());
        fs::write(&p, b"{not json").unwrap();
        assert_eq!(load_from(&p), Settings::default());
    }

    #[test]
    fn migrates_legacy_jpeg_quality() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        fs::write(&p, br#"{"jpeg_quality":250}"#).unwrap();
        assert_eq!(load_from(&p).quality.jpeg, 100);
        fs::write(&p, br#"{"jpeg_quality":0,"autostart":true}"#).unwrap();
        let s = load_from(&p);
        assert!(s.autostart && s.quality.jpeg == 1 && s.conflict == ConflictMode::AutoRename);
        assert!(!s.ffmpeg_notice_seen && !s.ffmpeg_ready);
        assert_eq!(s.enabled_targets.len(), Format::ALL.len());
        assert_eq!(s.name_template, "{name}");
        // the legacy key is never written back
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("jpeg_quality").is_none() && v["quality"]["jpeg"] == 1);
    }

    #[test]
    fn clamps_every_range_and_cleans_lists() {
        let json = r#"{"quality":{"jpeg":0,"mp3_q":99,"ogg_q":99,"video_mbps":0,"gif_fps":1,"gif_width":9999},
            "panel":{"seconds":0,"corner":"bottom_left"},
            "enabled_targets":["PNG","png","exe","mp4"],"menu_sources":["jpg","zzz"],
            "name_template":"a/b"}"#;
        let s = serde_json::from_str::<Settings>(json).unwrap().normalized();
        let q = s.quality;
        assert_eq!(
            (
                q.jpeg,
                q.mp3_q,
                q.ogg_q,
                q.video_mbps,
                q.gif_fps,
                q.gif_width
            ),
            (1, 9, 10, 1, 5, 1280)
        );
        assert_eq!((s.panel.seconds, s.panel.corner), (2, Corner::BottomLeft));
        assert_eq!(s.enabled_targets, ["png", "mp4"]);
        assert_eq!(s.menu_sources, ["jpg"]);
        assert_eq!(s.name_template, "{name}");
        assert!(s.target_enabled(Format::Png) && !s.target_enabled(Format::Jpg));
        let mut hi = Settings::default();
        hi.panel.seconds = 200;
        assert_eq!(hi.normalized().panel.seconds, 30);
    }

    #[test]
    fn validate_reports_template_errors() {
        let mut s = Settings::default();
        assert!(s.validate().is_ok());
        for bad in ["a/b", "con", "x*", &"a".repeat(101)] {
            s.name_template = bad.into();
            assert!(s.validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn roundtrip_and_json_shape() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub").join("s.json");
        let s = Settings {
            autostart: true,
            output_dir: OutputDir::Fixed {
                path: PathBuf::from(r"C:\out"),
            },
            conflict: ConflictMode::ReplaceOriginal,
            ffmpeg_ready: true,
            ffmpeg_notice_seen: true,
            name_template: "{date} {name}".into(),
            menu: MenuOpts {
                icons: false,
                sort: MenuSort::Alphabetical,
            },
            ..Settings::default()
        };
        save_to(&p, &s).unwrap();
        assert_eq!(load_from(&p), s);
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["output_dir"]["kind"], "fixed");
        assert_eq!(v["conflict"], "replace_original");
        assert_eq!(v["menu"]["sort"], "alphabetical");
        let v = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(v["output_dir"]["kind"], "same_as_source");
        assert_eq!(v["panel"]["corner"], "top_right");
        assert_eq!(v["quality"]["gif_width"], 480);
    }

    #[test]
    fn language_defaults_to_english_and_survives_roundtrip() {
        assert_eq!(Settings::default().language, Lang::En);
        let load = |j: &str| serde_json::from_str::<Settings>(j).unwrap().normalized();
        assert_eq!(load("{}").language, Lang::En);
        assert_eq!(load(r#"{"language":"tr"}"#).language, Lang::Tr);
        assert_eq!(load(r#"{"language":"xx"}"#).language, Lang::En);
        assert_eq!(load(r#"{"language":5}"#).language, Lang::En);
        let v = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(v["language"], "en");
        let tr = Settings {
            language: Lang::Tr,
            ..Settings::default()
        };
        assert_eq!(serde_json::to_value(&tr).unwrap()["language"], "tr");
    }

    #[test]
    fn validate_speaks_the_settings_language() {
        let mut s = Settings {
            name_template: "a/b".into(),
            ..Settings::default()
        };
        let en = s.validate().unwrap_err();
        s.language = Lang::Tr;
        assert_ne!(en, s.validate().unwrap_err());
        assert!(en.starts_with("The file name"));
    }

    #[test]
    fn maps_to_options() {
        let o = Settings::default().to_options();
        assert_eq!(o.output.conflict, Conflict::AutoRename);
        assert!(matches!(o.output.dir, OutDir::SameAsSource));
        assert_eq!(o.output.template, "{name}");
        assert_eq!(o.quality, Quality::default());
    }
}
