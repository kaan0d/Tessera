//! Goal-oriented menu entries ("For WhatsApp"). Built-ins live in code (not editable, can be
//! hidden); user presets are stored in settings. A preset bypasses `enabled_targets`.

use serde::{Deserialize, Serialize};

use crate::convert::{allows, uses_ffmpeg, Format, Options, Resize};
use crate::i18n::{fmt, t, Lang};
use crate::settings::Settings;

pub const MAX_CUSTOM: usize = 30;
const KINDS: [&str; 4] = ["image", "audio", "video", "document"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetOptions {
    /// `{"max_edge":1920}` | `{"percent":50}` | `{"exact":{"w":..,"h":..,"fit":"contain"}}`
    pub resize: Option<Resize>,
    pub strip_metadata: bool,
    /// Compress to at most this many bytes.
    pub max_bytes: Option<u64>,
    /// 1..=100, overrides the settings quality for this preset.
    pub jpeg_quality: Option<u8>,
    /// 1..=30, overrides the settings video bitrate.
    pub video_mbps: Option<u8>,
    /// Video outputs get no audio track.
    pub no_audio: bool,
    /// 8..=512 kbit/s for lossy audio and the sound of a video.
    pub audio_kbps: Option<u16>,
    /// One audio channel.
    pub mono: bool,
    /// Cut from this second to `trim_end` (seconds; end of file when empty).
    pub trim_start: Option<f32>,
    pub trim_end: Option<f32>,
    /// GIF outputs: frames per second (5..=30) and width in px (120..=1280).
    pub gif_fps: Option<u8>,
    pub gif_width: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    /// `[a-z0-9_-]{1,32}`; empty on save means "make one".
    pub id: String,
    /// i18n key; built-ins only.
    pub name_key: Option<String>,
    /// Display name of a custom preset, 1..=40 characters.
    pub custom_name: Option<String>,
    /// Source kinds it applies to: "image" | "audio" | "video" | "document".
    pub kinds: Vec<String>,
    /// Output extension, or "same" to keep the source format.
    pub target: String,
    /// Overrides `target` for video sources.
    pub video_target: Option<String>,
    pub options: PresetOptions,
}

fn builtin(
    id: &str,
    kinds: &[&str],
    target: &str,
    video_target: Option<&str>,
    options: PresetOptions,
) -> Preset {
    Preset {
        id: id.into(),
        name_key: Some(format!("preset.{id}")),
        custom_name: None,
        kinds: kinds.iter().map(|k| k.to_string()).collect(),
        target: target.into(),
        video_target: video_target.map(String::from),
        options,
    }
}

pub fn builtins() -> Vec<Preset> {
    let o = PresetOptions::default;
    vec![
        builtin(
            "web",
            &["image"],
            "jpg",
            None,
            PresetOptions {
                resize: Some(Resize::MaxEdge(1920)),
                strip_metadata: true,
                jpeg_quality: Some(80),
                ..o()
            },
        ),
        builtin(
            "whatsapp",
            &["image"],
            "jpg",
            None,
            PresetOptions {
                resize: Some(Resize::MaxEdge(1600)),
                strip_metadata: true,
                jpeg_quality: Some(80),
                max_bytes: Some(1_500_000),
                ..o()
            },
        ),
        builtin(
            "email",
            &["image", "video"],
            "same",
            None,
            PresetOptions {
                max_bytes: Some(10_000_000),
                ..o()
            },
        ),
        builtin(
            "discord",
            &["image", "video"],
            "same",
            Some("mp4"),
            PresetOptions {
                max_bytes: Some(8_000_000),
                ..o()
            },
        ),
        builtin(
            "strip",
            &["image", "video"],
            "same",
            None,
            PresetOptions {
                strip_metadata: true,
                jpeg_quality: Some(92),
                ..o()
            },
        ),
        builtin("pdf", &["image"], "pdf", None, o()),
        builtin(
            "webp",
            &["image"],
            "webp",
            None,
            PresetOptions {
                strip_metadata: true,
                jpeg_quality: Some(80),
                ..o()
            },
        ),
        builtin("ico", &["image"], "ico", None, o()),
    ]
}

pub fn is_builtin(id: &str) -> bool {
    builtins().iter().any(|p| p.id == id)
}

/// Built-ins first, then the user's presets.
pub fn all(s: &Settings) -> Vec<Preset> {
    let mut v = builtins();
    v.extend(s.presets.iter().cloned());
    v
}

pub fn find(s: &Settings, id: &str) -> Option<Preset> {
    all(s).into_iter().find(|p| p.id == id)
}

fn parse_target(t: &str, from: Format) -> Option<Format> {
    if t == "same" {
        return Some(from);
    }
    Format::from_ext(t).filter(|f| Format::ALL.contains(f) && f.ext() == t)
}

impl Preset {
    pub fn name(&self, lang: Lang) -> String {
        match (&self.name_key, &self.custom_name) {
            (Some(k), _) => t(lang, k).to_string(),
            (None, Some(n)) => n.clone(),
            _ => self.id.clone(),
        }
    }

    /// Settings options with this preset's overrides on top.
    pub fn options_for(&self, s: &Settings) -> Options {
        let mut o = s.to_options();
        let p = &self.options;
        o.resize = p.resize;
        o.strip_metadata = p.strip_metadata;
        o.max_bytes = p.max_bytes;
        o.no_audio = p.no_audio;
        o.audio_kbps = p.audio_kbps;
        o.mono = p.mono;
        o.trim_start = p.trim_start;
        o.trim_end = p.trim_end;
        if let Some(f) = p.gif_fps {
            o.quality.gif_fps = f;
        }
        if let Some(w) = p.gif_width {
            o.quality.gif_width = w;
        }
        if let Some(q) = p.jpeg_quality {
            o.quality.jpeg = q;
            o.quality.webp = q;
        }
        if let Some(m) = p.video_mbps {
            o.quality.video_mbps = m;
        }
        o.clamped()
    }

    pub fn applies_to_kind(&self, from: Format) -> bool {
        self.kinds.iter().any(|k| k == from.kind())
    }

    /// Target format for a source extension, or None when the preset does not apply to it.
    pub fn resolve(&self, ext: &str, opts: &Options) -> Option<Format> {
        let from = Format::from_ext(ext)?;
        if !self.applies_to_kind(from) {
            return None;
        }
        let spec = match (&self.video_target, from.is_video()) {
            (Some(v), true) => v,
            _ => &self.target,
        };
        let target = parse_target(spec, from)?;
        allows(ext, from, target, opts).then_some(target)
    }

    /// Like `resolve`, also hidden when the job would need ffmpeg and it is not ready (menu).
    pub fn menu_target(&self, ext: &str, s: &Settings) -> Option<Format> {
        let target = self.resolve(ext, &self.options_for(s))?;
        let from = Format::from_ext(ext)?;
        (s.ffmpeg_ready || !uses_ffmpeg(from, target)).then_some(target)
    }

    /// Checks and normalizes a user preset in place. The error is shown to the user.
    pub fn validate(&mut self, lang: Lang) -> Result<(), String> {
        let bad = |k| Err(t(lang, k).to_string());
        self.name_key = None;
        let name = self
            .custom_name
            .take()
            .unwrap_or_default()
            .trim()
            .to_string();
        // a leading '@' would make Explorer read the menu label as an indirect resource string
        if name.is_empty() || name.chars().count() > 40 || name.starts_with('@') {
            return bad("preset.bad_name");
        }
        self.custom_name = Some(name);
        self.id = self.id.trim().to_lowercase();
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 32
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
        if !id_ok {
            return bad("preset.bad_id");
        }
        if is_builtin(&self.id) {
            return bad("preset.builtin");
        }
        self.kinds.retain(|k| KINDS.contains(&k.as_str()));
        self.kinds.sort();
        self.kinds.dedup();
        if self.kinds.is_empty() {
            return bad("preset.bad_kinds");
        }
        self.target = self.target.trim().to_lowercase();
        let known = |s: &str| s == "same" || parse_target(s, Format::Png).is_some();
        if !known(&self.target) || self.video_target.as_deref().is_some_and(|v| !known(v)) {
            return bad("preset.bad_target");
        }
        let o = &mut self.options;
        o.resize = o.resize.map(Resize::clamped);
        o.max_bytes = o.max_bytes.map(|n| n.max(1));
        o.jpeg_quality = o.jpeg_quality.map(|q| q.clamp(1, 100));
        o.video_mbps = o.video_mbps.map(|q| q.clamp(1, 30));
        o.audio_kbps = o.audio_kbps.map(|k| k.clamp(8, 512));
        o.gif_fps = o.gif_fps.map(|f| f.clamp(5, 30));
        o.gif_width = o.gif_width.map(|w| w.clamp(120, 1280));
        // NaN, negative and absurd cut points are dropped; an end before the start is refused
        let ok = |t: f32| t.is_finite() && (0.0..=360_000.0).contains(&t);
        o.trim_start = o.trim_start.filter(|&t| ok(t));
        o.trim_end = o.trim_end.filter(|&t| ok(t));
        if matches!((o.trim_start, o.trim_end), (s, Some(e)) if e <= s.unwrap_or(0.0)) {
            return bad("preset.bad_trim");
        }
        let transforms = o.resize.is_some()
            || o.strip_metadata
            || o.max_bytes.is_some()
            || o.no_audio
            || o.audio_kbps.is_some()
            || o.mono
            || o.trim_start.is_some_and(|t| t > 0.0)
            || o.trim_end.is_some();
        let keeps = self.target == "same" || self.video_target.as_deref() == Some("same");
        if keeps && !transforms {
            return bad("preset.same_needs_option");
        }
        Ok(())
    }
}

/// Error for a full list (too many).
pub fn too_many(lang: Lang) -> String {
    fmt(lang, "preset.too_many", &[("max", &MAX_CUSTOM.to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(ext: &str, s: &Settings) -> Vec<String> {
        all(s)
            .into_iter()
            .filter(|p| p.menu_target(ext, s).is_some())
            .map(|p| p.id)
            .collect()
    }

    #[test]
    fn applicability_per_kind() {
        let s = Settings {
            ffmpeg_ready: true,
            ..Settings::default()
        };
        assert_eq!(
            ids("png", &s),
            ["web", "whatsapp", "email", "discord", "strip", "pdf", "webp", "ico"]
        );
        // a JPEG stays JPEG for web (same format with transforms); pdf source gets none
        assert_eq!(ids("jpg", &s).len(), 8);
        assert!(ids("pdf", &s).is_empty());
        assert_eq!(ids("mp4", &s), ["email", "discord", "strip"]);
        assert!(ids("mp3", &s).is_empty());
        // source-only image formats cannot keep their format, but can become JPG/PDF
        assert_eq!(ids("heic", &s), ["web", "whatsapp", "pdf", "webp", "ico"]);
        assert_eq!(ids("svg", &s), ["web", "whatsapp", "pdf", "webp", "ico"]);
    }

    #[test]
    fn video_target_and_ffmpeg_gate() {
        let s = Settings::default();
        let discord = find(&s, "discord").unwrap();
        let o = discord.options_for(&s);
        assert_eq!(discord.resolve("mkv", &o), Some(Format::Mp4));
        assert_eq!(discord.resolve("png", &o), Some(Format::Png));
        assert_eq!(discord.menu_target("mkv", &s), None);
        let email = find(&s, "email").unwrap();
        assert_eq!(email.resolve("mov", &o), Some(Format::Mov));
        assert_eq!(email.menu_target("png", &s), Some(Format::Png));
    }

    #[test]
    fn options_carry_overrides() {
        let s = Settings::default();
        let o = find(&s, "whatsapp").unwrap().options_for(&s);
        assert_eq!(o.resize, Some(Resize::MaxEdge(1600)));
        assert!(o.strip_metadata && o.quality.jpeg == 80 && o.max_bytes == Some(1_500_000));
        assert_eq!(s.to_options().quality.jpeg, 85);
    }

    #[test]
    fn custom_validation() {
        let ok = Preset {
            id: "Mine_1".into(),
            custom_name: Some(" My preset ".into()),
            kinds: vec!["image".into(), "image".into(), "junk".into()],
            target: "WEBP".into(),
            ..Preset::default()
        };
        let mut p = ok.clone();
        p.validate(Lang::En).unwrap();
        assert_eq!(
            (p.id.as_str(), p.target.as_str(), p.kinds.len()),
            ("mine_1", "webp", 1)
        );
        assert_eq!(p.custom_name.as_deref(), Some("My preset"));
        let fails = |f: fn(&mut Preset)| {
            let mut p = ok.clone();
            f(&mut p);
            p.validate(Lang::En).is_err()
        };
        assert!(fails(|p| p.id = "web".into()));
        assert!(fails(|p| p.id = "a b".into()));
        assert!(fails(|p| p.id.clear()));
        assert!(fails(|p| p.custom_name = None));
        assert!(fails(|p| p.kinds = vec!["x".into()]));
        assert!(fails(|p| p.target = "exe".into()));
        assert!(fails(|p| p.target = "same".into()));
        assert!(!fails(|p| {
            p.target = "same".into();
            p.options.strip_metadata = true;
        }));
        let mut p = ok;
        p.name_key = Some("preset.web".into());
        p.validate(Lang::Tr).unwrap();
        assert!(p.name_key.is_none());
    }

    #[test]
    fn audio_and_cut_options() {
        let s = Settings::default();
        let mut p = Preset {
            id: "cut".into(),
            custom_name: Some("Cut".into()),
            kinds: vec!["video".into()],
            target: "same".into(),
            options: PresetOptions {
                trim_start: Some(1.0),
                trim_end: Some(5.0),
                audio_kbps: Some(1),
                gif_fps: Some(99),
                ..PresetOptions::default()
            },
            ..Preset::default()
        };
        // a cut alone makes "same format" meaningful; values are clamped
        p.validate(Lang::En).unwrap();
        assert_eq!(
            (p.options.audio_kbps, p.options.gif_fps),
            (Some(8), Some(30))
        );
        let o = p.options_for(&s);
        assert_eq!(
            (o.trim_start, o.trim_end, o.quality.gif_fps),
            (Some(1.0), Some(5.0), 30)
        );
        assert!(o.ffmpeg_only(Format::Wav) && !o.ffmpeg_only(Format::Png));
        // end before start is refused, NaN is dropped
        p.options.trim_end = Some(0.5);
        assert!(p.validate(Lang::En).is_err());
        p.options.trim_end = None;
        p.options.audio_kbps = None;
        p.options.trim_start = Some(f32::NAN);
        assert!(p.validate(Lang::En).is_err()); // nothing left to change for "same"
    }

    #[test]
    fn names_follow_language() {
        let w = &builtins()[0];
        assert_eq!(w.name(Lang::En), "Shrink for web");
        assert_ne!(w.name(Lang::Tr), w.name(Lang::En));
    }
}
