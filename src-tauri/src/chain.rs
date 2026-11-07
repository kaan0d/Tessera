//! Runs one file through a preset's chain of steps. Every step writes into its own folder under
//! `%TEMP%\tessera-chain-*`; only the last result is moved to where the output rules say. The
//! folder is removed on every way out, and old ones left by a crash are swept at start.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::convert::pdf_tools::{self, PdfTool};
use crate::convert::{
    Conflict, ConvertError, Format, Options, OutDir, OutputParams, convert, output, uses_ffmpeg,
    uses_office,
};
use crate::preset::StepPlan;

const PREFIX: &str = "tessera-chain-";
const STALE: Duration = Duration::from_secs(24 * 3600);

/// The temp folder of one run; dropped = deleted.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Scratch, ConvertError> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{PREFIX}{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Removes chain folders older than a day (a crash or power loss left them).
pub fn sweep() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > STALE);
        if old && e.file_name().to_string_lossy().starts_with(PREFIX) && e.path().is_dir() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Overall progress (0..1) of step `i` of `n` at `p` (0..1): every step gets an equal share.
pub fn scale(i: usize, n: usize, p: f32) -> f32 {
    (i as f32 + p.clamp(0.0, 1.0)) / n.max(1) as f32
}

/// One conversion or PDF tool. `opts` already carries the cancel token and the output rules.
pub fn run_step(
    path: &Path,
    from: Option<Format>,
    target: Format,
    pdf: Option<&PdfTool>,
    opts: &Options,
    ffmpeg_ready: bool,
    progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    if let Some(tool) = pdf {
        pdf_tools::run(tool, path, opts, progress)
    } else if from.is_some_and(|f| uses_office(f, target)) {
        crate::office::convert(path, target, opts, progress)
    } else if from.is_some_and(|f| uses_ffmpeg(f, target) || opts.ffmpeg_only(f)) {
        if !ffmpeg_ready {
            return Err(ConvertError::NeedsFfmpeg);
        }
        let never = AtomicBool::new(false);
        crate::convert::ffmpeg::convert_with_ffmpeg(
            &crate::ffmpeg::exe_path(),
            path,
            target,
            opts,
            opts.cancel.as_deref().unwrap_or(&never),
            progress,
        )
    } else {
        convert(path, target, opts, progress)
    }
}

/// What a chain needs before it starts, so no step fails halfway for a missing tool.
fn preflight(steps: &[StepPlan], ffmpeg_ready: bool) -> Result<(), ConvertError> {
    for s in steps {
        let ff = uses_ffmpeg(s.from, s.target) || s.opts.ffmpeg_only(s.from);
        if ff && !ffmpeg_ready {
            return Err(ConvertError::NeedsFfmpeg);
        }
        if s.pdf.is_none() && uses_office(s.from, s.target) && !crate::office::ready() {
            return Err(ConvertError::NeedsOffice);
        }
    }
    Ok(())
}

/// Runs `steps` on `input`; `out` says where the last result goes. `progress(step, overall)`
/// gets the 0-based step and the 0..1 progress of the whole chain. A failing step stops the
/// chain: the error names it, nothing is written, the original is untouched.
pub fn run(
    input: &Path,
    steps: &[StepPlan],
    out: &OutputParams,
    ffmpeg_ready: bool,
    cancel: &Arc<AtomicBool>,
    mut progress: impl FnMut(usize, f32),
) -> Result<PathBuf, ConvertError> {
    let n = steps.len();
    if n == 0 {
        return Err(ConvertError::Unsupported);
    }
    preflight(steps, ffmpeg_ready)?;
    let scratch = Scratch::new()?;
    let mut cur = input.to_path_buf();
    for (i, s) in steps.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(ConvertError::Cancelled);
        }
        let mut o = s.opts.clone().cancel(cancel.clone());
        o.output = OutputParams {
            dir: OutDir::Fixed(scratch.0.join(format!("step{}", i + 1))),
            conflict: Conflict::Overwrite,
            template: String::new(),
        };
        let r = run_step(
            &cur,
            Some(s.from),
            s.target,
            s.pdf.as_ref(),
            &o,
            ffmpeg_ready,
            |p| progress(i, scale(i, n, p)),
        )
        .and_then(|f| f.is_file().then_some(f).ok_or(ConvertError::Unsupported));
        cur = match r {
            Ok(f) => f,
            Err(ConvertError::Cancelled) => return Err(ConvertError::Cancelled),
            Err(e) => return Err(ConvertError::InStep(i + 1, n, Box::new(e))),
        };
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(ConvertError::Cancelled);
    }
    let ext = cur.extension().and_then(|e| e.to_str()).unwrap_or("");
    let dest = output::plan(input, ext, out)?;
    output::write_via_temp(&dest, |tmp| {
        std::fs::copy(&cur, tmp)?;
        Ok(())
    })?;
    // the original goes to the Recycle Bin only now that the whole chain worked
    if out.conflict == Conflict::ReplaceOriginal {
        output::recycle(input)?;
    }
    progress(n - 1, 1.0);
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn plan(from: Format, target: Format, pdf: Option<PdfTool>) -> StepPlan {
        StepPlan {
            from,
            target,
            pdf,
            opts: Settings::default().to_options(),
        }
    }

    fn png(dir: &Path) -> PathBuf {
        let p = dir.join("pic.png");
        image::RgbImage::from_pixel(64, 64, image::Rgb([200, 30, 30]))
            .save(&p)
            .unwrap();
        p
    }

    fn chains() -> usize {
        std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(PREFIX))
            .count()
    }

    #[test]
    fn progress_is_shared_between_steps() {
        assert_eq!(scale(0, 3, 0.0), 0.0);
        assert!((scale(1, 4, 0.5) - 0.375).abs() < 1e-6);
        assert_eq!(scale(2, 3, 1.0), 1.0);
        assert_eq!(scale(0, 2, 7.0), 0.5);
    }

    #[test]
    fn three_steps_make_one_output_and_no_leftovers() {
        let d = tempfile::tempdir().unwrap();
        let src = png(d.path());
        let steps = [
            plan(Format::Png, Format::Jpg, None),
            plan(Format::Jpg, Format::Pdf, None),
            plan(
                Format::Pdf,
                Format::Pdf,
                Some(PdfTool::Compress { strong: false }),
            ),
        ];
        let before = chains();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut seen = vec![];
        let out = run(
            &src,
            &steps,
            &OutputParams::default(),
            false,
            &cancel,
            |i, p| seen.push((i, p)),
        )
        .unwrap();
        assert_eq!(out, d.path().join("pic.pdf"));
        assert!(std::fs::metadata(&out).unwrap().len() > 0);
        // only the source and the result are in the folder
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 2);
        assert!(chains() <= before, "the scratch folder is gone");
        assert!(seen.windows(2).all(|w| w[0].1 <= w[1].1));
        assert_eq!(seen.last().unwrap().1, 1.0);
        assert!(seen.iter().any(|s| s.0 == 2));
    }

    #[test]
    fn a_failing_step_names_itself_and_writes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let src = png(d.path());
        // a PNG is no PDF: step 2 fails
        let steps = [
            plan(Format::Png, Format::Jpg, None),
            plan(
                Format::Pdf,
                Format::Pdf,
                Some(PdfTool::Compress { strong: false }),
            ),
        ];
        let cancel = Arc::new(AtomicBool::new(false));
        let e = run(
            &src,
            &steps,
            &OutputParams::default(),
            false,
            &cancel,
            |_, _| {},
        )
        .unwrap_err();
        assert!(matches!(e, ConvertError::InStep(2, 2, _)), "{e:?}");
        assert!(e.to_string().starts_with("Step 2 of 2"), "{e}");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn cancel_between_steps_stops_and_cleans_up() {
        let d = tempfile::tempdir().unwrap();
        let src = png(d.path());
        let steps = [
            plan(Format::Png, Format::Jpg, None),
            plan(Format::Jpg, Format::Pdf, None),
        ];
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let e = run(
            &src,
            &steps,
            &OutputParams::default(),
            false,
            &cancel,
            |i, p| {
                // the first step is done: stop before the second
                if i == 0 && p >= 0.5 {
                    c.store(true, Ordering::Relaxed);
                }
            },
        )
        .unwrap_err();
        assert!(matches!(e, ConvertError::Cancelled), "{e:?}");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn missing_ffmpeg_fails_before_any_work() {
        let d = tempfile::tempdir().unwrap();
        let src = png(d.path());
        let steps = [
            plan(Format::Png, Format::Jpg, None),
            plan(Format::Gif, Format::Mp4, None),
        ];
        let cancel = Arc::new(AtomicBool::new(false));
        let e = run(
            &src,
            &steps,
            &OutputParams::default(),
            false,
            &cancel,
            |_, _| {},
        )
        .unwrap_err();
        assert!(matches!(e, ConvertError::NeedsFfmpeg), "{e:?}");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }
}
