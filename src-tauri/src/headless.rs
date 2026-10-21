//! Command-line subcommands: run the engine directly, print to the parent terminal, exit.
//! No tray, no window, no single-instance forwarding and no history.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::cli::{Command, ConvertArgs};
use crate::convert::{
    convert, needs_ffmpeg, supported_targets, uses_ffmpeg, Format, Options, OutDir,
};
use crate::i18n::{t, Lang};
use crate::{app_info, ffmpeg, preset, settings};

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;

pub const HELP: &str = "\
Usage:
  new-app convert <files...> --to <ext> [options]
  new-app preset <id> <files...>
  new-app list-presets
  new-app formats <ext>
  new-app --version | --help

Convert options:
  --to <ext>          target format (png, jpg, webp, pdf, mp3, mp4, ...)
  --max-edge <px>     longest side at most <px> pixels
  --percent <n>       scale to <n> percent (1-400)
  --strip             remove EXIF/GPS and other metadata
  --max-size <size>   compress to at most <size>, e.g. 8MB or 500KB
  --quality <1-100>   image quality
  --out <dir>         output folder (default: next to the source)
  --conflict <mode>   rename (default) or overwrite

Presets come from `list-presets` and use your saved settings.
Video, MP3 and OGG need the ffmpeg the app installed; it is never downloaded here.
Exit codes: 0 ok, 1 a file failed, 2 usage error.
";

fn out(s: &str) {
    let _ = writeln!(std::io::stdout(), "{s}");
}

fn err(s: &str) {
    let _ = writeln!(std::io::stderr(), "{s}");
}

/// The exe is a GUI-subsystem program: borrow the parent's console so output shows up there.
/// Redirected (piped or file) standard handles are left alone.
fn attach_console() {
    use std::os::windows::io::IntoRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
        STD_OUTPUT_HANDLE,
    };
    // SAFETY: plain console API calls; the handle given to SetStdHandle is leaked on purpose
    // and stays valid for the life of the process.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        for std in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let unset = GetStdHandle(std).map_or(true, |h| h.is_invalid() || h.0.is_null());
            if unset {
                if let Ok(f) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
                    let _ = SetStdHandle(std, HANDLE(f.into_raw_handle()));
                }
            }
        }
    }
}

/// Runs a subcommand and returns the process exit code.
pub fn run(cmd: Command) -> i32 {
    attach_console();
    match cmd {
        Command::Help => {
            out(&format!("{}\n", version_line()));
            out(HELP);
            EXIT_OK
        }
        Command::Version => {
            out(&version_line());
            EXIT_OK
        }
        Command::Usage(msg) => {
            err(&format!("error: {msg}"));
            err("Run `new-app --help` for usage.");
            EXIT_USAGE
        }
        Command::ListPresets => list_presets(),
        Command::Formats(ext) => formats(&ext),
        Command::Convert(a) => convert_cmd(a),
        Command::Preset { id, files } => preset_cmd(&id, &files),
    }
}

fn version_line() -> String {
    format!("{} {}", app_info::APP_NAME, env!("CARGO_PKG_VERSION"))
}

fn list_presets() -> i32 {
    let s = settings::load_from(&settings::settings_path());
    for p in preset::all(&s) {
        out(&format!(
            "{:<14}{}  [{}] -> {}",
            p.id,
            p.name(Lang::En),
            p.kinds.join(", "),
            p.target
        ));
    }
    EXIT_OK
}

fn formats(ext: &str) -> i32 {
    let ext = ext.trim_start_matches('.');
    let Some(src) = Format::from_ext(ext) else {
        err(&format!("unknown format: {ext}"));
        return EXIT_FAILED;
    };
    out(&format!("{} ({})", ext.to_lowercase(), src.kind()));
    let targets = supported_targets(ext);
    if targets.is_empty() {
        out("cannot be converted");
        return EXIT_OK;
    }
    let list: Vec<String> = targets
        .iter()
        .map(|&t| {
            let star = if uses_ffmpeg(src, t) { "*" } else { "" };
            format!("{}{star}", t.ext())
        })
        .collect();
    out(&format!("converts to: {}", list.join(", ")));
    if targets.iter().any(|&t| uses_ffmpeg(src, t)) {
        out("* needs ffmpeg");
    }
    EXIT_OK
}

fn convert_cmd(a: ConvertArgs) -> i32 {
    let mut opts = Options {
        resize: a.resize,
        strip_metadata: a.strip,
        max_bytes: a.max_bytes,
        ..Options::default()
    };
    if let Some(q) = a.quality {
        opts.quality.jpeg = q;
    }
    opts.output.conflict = a.conflict;
    if let Some(d) = a.out {
        opts.output.dir = OutDir::Fixed(d);
    }
    run_files(&a.files, |_| Some((a.target, opts.clone())))
}

fn preset_cmd(id: &str, files: &[PathBuf]) -> i32 {
    let s = settings::load_from(&settings::settings_path());
    let Some(p) = preset::find(&s, id) else {
        err(&format!(
            "unknown preset: {id} (see `new-app list-presets`)"
        ));
        return EXIT_FAILED;
    };
    let opts = p.options_for(&s);
    run_files(files, |path| {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        p.resolve(ext, &opts).map(|t| (t, opts.clone()))
    })
}

/// Converts each file with what `plan` picks (None = not applicable). Exit code 1 if any failed.
fn run_files(files: &[PathBuf], plan: impl Fn(&Path) -> Option<(Format, Options)>) -> i32 {
    let (mut failed, mut ffmpeg_ok) = (0, None);
    let n = files.len();
    for (i, path) in files.iter().enumerate() {
        let tag = format!("[{}/{n}] {}", i + 1, path.display());
        let Some((target, opts)) = plan(path) else {
            err(&format!("{tag}: not supported here, skipped"));
            failed += 1;
            continue;
        };
        let mut last = 0.0f32;
        let on_progress = |p: f32| {
            let step = (p * 4.0).floor() / 4.0;
            if step > last && step < 1.0 {
                last = step;
                out(&format!("{tag} {:.0}%", step * 100.0));
            }
        };
        match convert_one(path, target, &opts, &mut ffmpeg_ok, on_progress) {
            Ok(dest) => out(&format!("{tag} -> {}", dest.display())),
            Err(e) => {
                err(&format!("{tag}: {e}"));
                failed += 1;
            }
        }
    }
    if failed > 0 {
        err(&format!("{failed} of {n} failed"));
        EXIT_FAILED
    } else {
        EXIT_OK
    }
}

/// `ffmpeg_ok` caches whether the installed ffmpeg works (checked on first need).
fn convert_one(
    path: &Path,
    target: Format,
    opts: &Options,
    ffmpeg_ok: &mut Option<bool>,
    progress: impl FnMut(f32),
) -> Result<PathBuf, String> {
    let lang = Lang::En;
    let from = path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(Format::from_ext);
    let r = if from.is_some_and(|f| uses_ffmpeg(f, target)) || needs_ffmpeg(target) {
        let exe = ffmpeg::exe_path();
        if !*ffmpeg_ok.get_or_insert_with(|| ffmpeg::is_working(&exe)) {
            return Err(t(lang, "cli.no_ffmpeg").into());
        }
        crate::convert::ffmpeg::convert_with_ffmpeg(
            &exe,
            path,
            target,
            opts,
            &AtomicBool::new(false),
            progress,
        )
    } else {
        convert(path, target, opts, progress)
    };
    r.map_err(|e| e.message(lang))
}
