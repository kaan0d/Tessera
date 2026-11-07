//! Goal-oriented menu entries ("For WhatsApp"). Built-ins live in code (not editable, can be
//! hidden); user presets are stored in settings. A preset bypasses `enabled_targets`.

use serde::{Deserialize, Serialize};

use crate::convert::pdf_tools::PdfTool;

use crate::convert::{Format, Options, Resize, allows, uses_ffmpeg, uses_office};
use crate::i18n::{Lang, fmt, t};
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
    /// A chain: each step works on the result of the one before. Empty = the single step
    /// above (`target`/`options`). With steps, `target` only mirrors the last one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
}

/// Most steps a chain may have.
pub const MAX_STEPS: usize = 5;

/// One step of a chain: a conversion to `target`, or a PDF tool (then `target` is "pdf").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    pub target: String,
    pub tool: Option<PdfTool>,
    pub options: PresetOptions,
}

/// A step resolved for one source file.
#[derive(Debug, Clone)]
pub struct StepPlan {
    pub from: Format,
    pub target: Format,
    pub pdf: Option<PdfTool>,
    pub opts: Options,
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
        steps: vec![],
    }
}

fn step(target: &str, tool: Option<PdfTool>) -> Step {
    Step {
        target: target.into(),
        tool,
        options: PresetOptions::default(),
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
        // files sent to others lose their location and camera data, even when they already fit
        builtin(
            "email",
            &["image", "video"],
            "same",
            None,
            PresetOptions {
                max_bytes: Some(10_000_000),
                strip_metadata: true,
                ..o()
            },
        ),
        builtin(
            "discord",
            &["image", "video"],
            "same",
            Some("mp4"),
            PresetOptions {
                max_bytes: Some(10_000_000),
                strip_metadata: true,
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
        builtin("pdf", &["image", "document"], "pdf", None, o()),
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
        builtin(
            "instagram",
            &["image"],
            "jpg",
            None,
            PresetOptions {
                resize: Some(Resize::MaxEdge(1080)),
                strip_metadata: true,
                jpeg_quality: Some(85),
                ..o()
            },
        ),
        builtin(
            "podcast",
            &["audio"],
            "mp3",
            None,
            PresetOptions {
                audio_kbps: Some(96),
                mono: true,
                ..o()
            },
        ),
        // scan -> searchable PDF -> smaller PDF (a PDF source skips the first step)
        Preset {
            steps: vec![
                step("pdf", None),
                step("pdf", Some(PdfTool::Ocr { lang: None })),
                step("pdf", Some(PdfTool::Compress { strong: false })),
            ],
            ..builtin("scan_share", &["image", "document"], "pdf", None, o())
        },
    ]
}

/// Some file of `kind` can be converted to `target` (an image preset can make an MP4 from a
/// GIF, but no image becomes a DOCX).
pub fn kind_reaches(kind: &str, target: Format) -> bool {
    Format::ALL.iter().any(|f| {
        f.kind() == kind
            && (*f == target || crate::convert::supported_targets(f.ext()).contains(&target))
    })
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
        self.options.to_options(s)
    }
}

impl PresetOptions {
    pub fn to_options(&self, s: &Settings) -> Options {
        let mut o = s.to_options();
        let p = self;
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
            o.recode = true;
        }
        if let Some(m) = p.video_mbps {
            o.quality.video_mbps = m;
            o.recode = true;
        }
        o.clamped()
    }

    /// Clamps every value into its range; the error is an i18n key.
    fn clamp(&mut self) -> Result<(), &'static str> {
        let o = self;
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
            return Err("preset.bad_trim");
        }
        Ok(())
    }

    /// Some option changes the file itself (so "same format" still does something).
    fn transforms(&self) -> bool {
        let o = self;
        o.resize.is_some()
            || o.strip_metadata
            || o.max_bytes.is_some()
            || o.no_audio
            || o.audio_kbps.is_some()
            || o.mono
            || o.trim_start.is_some_and(|t| t > 0.0)
            || o.trim_end.is_some()
            || o.jpeg_quality.is_some()
            || o.video_mbps.is_some()
    }
}

impl Preset {
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

    /// Like `resolve`, also hidden when the job needs ffmpeg or LibreOffice and it is missing.
    pub fn menu_target(&self, ext: &str, s: &Settings) -> Option<Format> {
        if !self.steps.is_empty() {
            let plan = self.plan_chain(ext, s)?;
            let ready = plan.iter().all(|p| {
                let ff = uses_ffmpeg(p.from, p.target) || p.opts.ffmpeg_only(p.from);
                let office = p.pdf.is_none() && uses_office(p.from, p.target);
                (s.ffmpeg_ready || !ff) && (!office || crate::office::ready())
            });
            return ready.then(|| plan.last().map(|p| p.target)).flatten();
        }
        let target = self.resolve(ext, &self.options_for(s))?;
        let from = Format::from_ext(ext)?;
        let ffmpeg_ok = s.ffmpeg_ready || !uses_ffmpeg(from, target);
        let office_ok = !uses_office(from, target) || crate::office::ready();
        (ffmpeg_ok && office_ok).then_some(target)
    }

    /// The steps of a chain for a source extension, or None when the chain does not apply to it
    /// (wrong kind, a step the previous result cannot take). A plain conversion to the format
    /// the file already has (a PDF "made" a PDF) is left out; the result may be empty.
    pub fn plan_chain(&self, ext: &str, s: &Settings) -> Option<Vec<StepPlan>> {
        self.plan_from(Format::from_ext(ext)?, s)
    }

    fn plan_from(&self, mut cur: Format, s: &Settings) -> Option<Vec<StepPlan>> {
        if !self.applies_to_kind(cur) {
            return None;
        }
        let mut plan = vec![];
        for st in &self.steps {
            let opts = st.options.to_options(s);
            let (target, pdf) = match &st.tool {
                Some(tool) => {
                    if cur != Format::Pdf || !tool.valid() || *tool == PdfTool::Split {
                        return None;
                    }
                    (Format::Pdf, Some(tool.clone()))
                }
                None => {
                    let t = parse_target(&st.target, cur)?;
                    if t == cur && cur == Format::Pdf {
                        continue;
                    }
                    // pages of a PDF as pictures would be a folder, not a file to pass on
                    if cur == Format::Pdf && t.is_image() {
                        return None;
                    }
                    if !allows(cur.ext(), cur, t, &opts) {
                        return None;
                    }
                    (t, None)
                }
            };
            plan.push(StepPlan {
                from: cur,
                target,
                pdf,
                opts,
            });
            cur = target;
        }
        Some(plan)
    }

    /// `validate` for a chain: every step must be known and each kind the preset takes must
    /// get through all of them.
    fn validate_steps(&mut self, lang: Lang) -> Result<(), String> {
        let bad = |k| Err(t(lang, k).to_string());
        if self.steps.len() > MAX_STEPS {
            return Err(fmt(
                lang,
                "preset.too_many_steps",
                &[("max", &MAX_STEPS.to_string())],
            ));
        }
        for st in &mut self.steps {
            st.target = st.target.trim().to_lowercase();
            match &st.tool {
                Some(tool) if !tool.valid() || *tool == PdfTool::Split => {
                    return bad("preset.bad_step");
                }
                Some(_) => st.target = "pdf".into(),
                None if parse_target(&st.target, Format::Png).is_none() => {
                    return bad("preset.bad_target");
                }
                None => {}
            }
            st.options.clamp().or_else(bad)?;
        }
        let s = Settings::default();
        let fits = |k: &String| {
            Format::ALL
                .iter()
                .any(|f| f.kind() == k && self.plan_from(*f, &s).is_some_and(|p| !p.is_empty()))
        };
        if !self.kinds.iter().all(fits) {
            return bad("preset.chain_kind");
        }
        self.target = self
            .steps
            .last()
            .map(|st| st.target.clone())
            .unwrap_or_default();
        self.video_target = None;
        Ok(())
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
        if !self.steps.is_empty() {
            return self.validate_steps(lang);
        }
        self.target = self.target.trim().to_lowercase();
        let known = |s: &str| s == "same" || parse_target(s, Format::Png).is_some();
        if !known(&self.target) || self.video_target.as_deref().is_some_and(|v| !known(v)) {
            return bad("preset.bad_target");
        }
        self.options.clamp().or_else(bad)?;
        let transforms = self.options.transforms();
        let keeps = self.target == "same" || self.video_target.as_deref() == Some("same");
        if keeps && !transforms {
            return bad("preset.same_needs_option");
        }
        // every kind the preset takes must be able to become its target (video: its own one)
        for k in &self.kinds {
            let spec = match (k.as_str(), &self.video_target) {
                ("video", Some(v)) => v.as_str(),
                _ => self.target.as_str(),
            };
            let reachable = spec == "same"
                || parse_target(spec, Format::Png).is_some_and(|t| kind_reaches(k, t));
            if !reachable {
                return bad("preset.kind_target");
            }
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
            [
                "web",
                "whatsapp",
                "email",
                "discord",
                "strip",
                "pdf",
                "webp",
                "ico",
                "instagram",
                "scan_share"
            ]
        );
        // a JPEG stays JPEG for web (same format with transforms); pdf source gets none
        assert_eq!(ids("jpg", &s).len(), 10);
        // only the chain takes a PDF (it makes it searchable and smaller)
        assert_eq!(ids("pdf", &s), ["scan_share"]);
        assert_eq!(ids("mp4", &s), ["email", "discord", "strip"]);
        assert_eq!(ids("mp3", &s), ["podcast"]);
        // source-only image formats cannot keep their format, but can become JPG/PDF
        assert_eq!(
            ids("heic", &s),
            [
                "web",
                "whatsapp",
                "pdf",
                "webp",
                "ico",
                "instagram",
                "scan_share"
            ]
        );
        assert_eq!(
            ids("svg", &s),
            [
                "web",
                "whatsapp",
                "pdf",
                "webp",
                "ico",
                "instagram",
                "scan_share"
            ]
        );
    }

    #[test]
    fn pdf_preset_takes_documents() {
        let s = Settings::default();
        let pdf = find(&s, "pdf").unwrap();
        let o = pdf.options_for(&s);
        for ext in ["docx", "xlsx", "pptx", "odt"] {
            assert_eq!(pdf.resolve(ext, &o), Some(Format::Pdf), "{ext}");
        }
        assert_eq!(pdf.resolve("pdf", &o), None);
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

    fn chain(kinds: &[&str], steps: Vec<Step>) -> Preset {
        Preset {
            id: "ch".into(),
            custom_name: Some("Chain".into()),
            kinds: kinds.iter().map(|k| k.to_string()).collect(),
            steps,
            ..Preset::default()
        }
    }

    #[test]
    fn old_files_load_without_steps_and_stay_that_way() {
        let old = r#"{"id":"a","custom_name":"A","kinds":["image"],"target":"png",
            "options":{"strip_metadata":true}}"#;
        let p: Preset = serde_json::from_str(old).unwrap();
        assert!(p.steps.is_empty() && p.options.strip_metadata);
        // a single-step preset is saved exactly as before: no "steps" key
        assert!(!serde_json::to_string(&p).unwrap().contains("steps"));
        let s: Settings = serde_json::from_str(&format!(r#"{{"presets":[{old}]}}"#)).unwrap();
        assert_eq!(s.normalized().presets.len(), 1);
    }

    #[test]
    fn a_chain_survives_a_json_round_trip() {
        let mut p = chain(
            &["image"],
            vec![
                step("pdf", None),
                step(
                    "pdf",
                    Some(PdfTool::Ocr {
                        lang: Some("tr".into()),
                    }),
                ),
            ],
        );
        p.validate(Lang::En).unwrap();
        let back: Preset = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.target, "pdf");
    }

    #[test]
    fn chain_validation() {
        let ok = chain(
            &["image"],
            vec![
                step("jpg", None),
                step("pdf", None),
                step("pdf", Some(PdfTool::Compress { strong: true })),
            ],
        );
        let mut p = ok.clone();
        p.validate(Lang::En).unwrap();
        // too many steps
        let mut p = ok.clone();
        p.steps = (0..6).map(|_| step("png", None)).collect();
        assert!(p.validate(Lang::En).unwrap_err().contains('5'));
        // a PDF tool cannot start from a picture
        let mut p = chain(
            &["image"],
            vec![step("pdf", Some(PdfTool::Compress { strong: false }))],
        );
        assert!(p.validate(Lang::En).is_err());
        // nothing converts a PDF back into a JPG file in one go (a folder of pages)
        let mut p = chain(&["image"], vec![step("pdf", None), step("jpg", None)]);
        assert!(p.validate(Lang::En).is_err());
        // splitting makes many files, no chain step
        let mut p = chain(&["document"], vec![step("pdf", Some(PdfTool::Split))]);
        assert!(p.validate(Lang::En).is_err());
        // unknown target, bad tool arguments
        let mut p = chain(&["image"], vec![step("exe", None)]);
        assert!(p.validate(Lang::En).is_err());
        let bad_pages = PdfTool::Extract { pages: "x".into() };
        let mut p = chain(
            &["image"],
            vec![step("pdf", None), step("pdf", Some(bad_pages))],
        );
        assert!(p.validate(Lang::En).is_err());
    }

    #[test]
    fn scan_share_plans_per_source() {
        let s = Settings::default();
        let p = find(&s, "scan_share").unwrap();
        let shape = |ext: &str| -> Vec<String> {
            p.plan_chain(ext, &s)
                .unwrap()
                .iter()
                .map(|st| match &st.pdf {
                    Some(t) => t.arg(),
                    None => st.target.ext().into(),
                })
                .collect()
        };
        assert_eq!(shape("png"), ["pdf", "ocr", "compress"]);
        // a PDF is already a PDF: the conversion step drops out
        assert_eq!(shape("pdf"), ["ocr", "compress"]);
        assert!(p.plan_chain("mp3", &s).is_none());
        assert_eq!(p.menu_target("png", &s), Some(Format::Pdf));
        assert_eq!(p.menu_target("pdf", &s), Some(Format::Pdf));
        assert_eq!(p.menu_target("mp4", &s), None);
        assert!(is_builtin("scan_share"));
    }

    #[test]
    fn a_chain_step_that_needs_ffmpeg_hides_the_preset_until_it_is_ready() {
        let mut s = Settings::default();
        let mut p = chain(&["image"], vec![step("jpg", None), step("pdf", None)]);
        p.validate(Lang::En).unwrap();
        assert!(p.menu_target("png", &s).is_some());
        let mut g = chain(&["image"], vec![step("gif", None)]);
        g.steps = vec![Step {
            target: "mp4".into(),
            ..Step::default()
        }];
        // a GIF can become a video: that step needs ffmpeg
        assert_eq!(g.menu_target("gif", &s), None);
        s.ffmpeg_ready = true;
        assert_eq!(g.menu_target("gif", &s), Some(Format::Mp4));
    }
}
