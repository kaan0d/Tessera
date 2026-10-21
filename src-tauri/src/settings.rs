use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app_info::APP_NAME;
use crate::convert::{Conflict, Options, OutDir, OutputParams};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub autostart: bool,
    pub output_dir: OutputDir,
    pub conflict: ConflictMode,
    pub jpeg_quality: u8,
    /// Owned by the ffmpeg stage; `set_settings` never overwrites it from the UI.
    pub ffmpeg_ready: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            autostart: false,
            output_dir: OutputDir::SameAsSource,
            conflict: ConflictMode::AutoRename,
            jpeg_quality: 85,
            ffmpeg_ready: false,
        }
    }
}

impl Settings {
    pub fn normalized(mut self) -> Self {
        self.jpeg_quality = self.jpeg_quality.clamp(1, 100);
        self
    }

    pub fn to_options(&self) -> Options {
        Options {
            jpeg_quality: self.jpeg_quality.clamp(1, 100),
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
            },
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
    fn clamps_quality_and_fills_missing_fields() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        fs::write(&p, br#"{"jpeg_quality":250}"#).unwrap();
        assert_eq!(load_from(&p).jpeg_quality, 100);
        fs::write(&p, br#"{"jpeg_quality":0,"autostart":true}"#).unwrap();
        let s = load_from(&p);
        assert!(s.autostart && s.jpeg_quality == 1 && s.conflict == ConflictMode::AutoRename);
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
            jpeg_quality: 70,
            ffmpeg_ready: true,
        };
        save_to(&p, &s).unwrap();
        assert_eq!(load_from(&p), s);
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["output_dir"]["kind"], "fixed");
        assert_eq!(v["conflict"], "replace_original");
        let v = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(v["output_dir"]["kind"], "same_as_source");
    }

    #[test]
    fn maps_to_options() {
        let o = Settings::default().to_options();
        assert_eq!(o.output.conflict, Conflict::AutoRename);
        assert!(matches!(o.output.dir, OutDir::SameAsSource));
    }
}
