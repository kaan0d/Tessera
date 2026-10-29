//! Command-line subcommands: run the engine directly, print to the parent terminal, exit.
//! No tray, no window, no single-instance forwarding and no history.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::cli::{Command, ConvertArgs};
use crate::convert::{
    convert, needs_ffmpeg, supported_targets, uses_ffmpeg, uses_office, Format, Options, OutDir,
};
use crate::i18n::{t, Lang};
use crate::{app_info, ffmpeg, preset, settings};

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;

pub const HELP: &str = "\
Usage:
  {p} convert <files, folders or *.png> --to <ext> [options]
  {p} preset <id> <files...> [--dry-run --json --quiet --out <dir> --conflict <mode>]
  {p} merge <pdf|audio> <files...> [--out <dir>] [--max-size <size>] [--to mp3|wav|flac]
                      images, PDFs and documents into one PDF, or audio files into one (audio is FLAC unless --to)
  {p} pdf <split|extract|delete|rotate|compress> <files...> [--pages 1-3,5] [--angle 90|180|270] [--strong]
                      split into pages, keep or delete pages, turn pages clockwise, shrink pictures
  {p} list-presets
  {p} formats <ext>
  {p} --version | --help

Convert options:
  --to <ext>          target format (png, jpg, webp, pdf, mp3, mp4, ...)
  --max-edge <px>     longest side at most <px> pixels
  --percent <n>       scale to <n> percent (1-400)
  --strip             remove EXIF/GPS and other metadata
  --max-size <size>   compress to at most <size>, e.g. 8MB or 500KB (a bare number is bytes)
  --quality <1-100>   image quality
  --out <dir>         output folder (default: next to the source)
  --conflict <mode>   rename (default) or overwrite
  --recursive         with a folder: go into its subfolders too
  --dry-run           show what would happen, change nothing
  --json              one JSON line per file instead of text
  --quiet             only errors and the summary
  --start <time>      cut audio/video: keep from here (90 or 1:30)
  --end <time>        cut audio/video: keep until here
  --audio-bitrate <k> audio quality in kbit/s (8-512)
  --mono              one audio channel
  --no-audio          video without its sound track
  --gif-fps <5-30>    frames per second of a GIF
  --gif-width <px>    width of a GIF (120-1280)
  --video-mbps <n>    video quality in Mbit/s (1-30)

Presets come from `list-presets` and use your saved settings.
Video, MP3 and OGG need the ffmpeg the app installed; it is never downloaded here.
Exit codes: 0 ok, 1 a file failed, 2 usage error.
Scripts: use tessera-cli.exe (next to tessera.exe); it waits for the job and returns the exit code.
";

/// The name the user typed: `tessera-cli` when this is the console copy.
fn prog() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.file_stem().map(|s| s.to_string_lossy().to_lowercase()))
        .filter(|s| s == "tessera-cli")
        .unwrap_or_else(|| "tessera".into())
}

/// "1.99 MB", "412 KB".
fn human(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.2} MB", bytes as f64 / 1e6)
    } else {
        format!("{} KB", bytes.div_ceil(1000))
    }
}

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
            out(&HELP.replace("{p}", &prog()));
            EXIT_OK
        }
        Command::Version => {
            out(&version_line());
            EXIT_OK
        }
        Command::Usage(msg) => {
            err(&format!("error: {msg}"));
            err(&format!("Run `{} --help` for usage.", prog()));
            EXIT_USAGE
        }
        Command::ListPresets => list_presets(),
        Command::Formats(ext) => formats(&ext),
        Command::Convert(a) => convert_cmd(a),
        Command::Preset { id, files, flags } => preset_cmd(&id, &files, flags),
        Command::MergeFiles { kind, files, flags } => merge_cmd(kind, &files, flags),
        Command::PdfFiles { tool, files, flags } => pdf_cmd(&tool, &files, flags),
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
        return EXIT_USAGE;
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

/// Why a file cannot be planned: shown as "skipped" with this reason.
type Plan = Result<(Format, Options), String>;

/// How a run reports itself (`--dry-run`, `--json`, `--quiet`).
#[derive(Clone, Copy, Default)]
struct Style {
    dry_run: bool,
    json: bool,
    quiet: bool,
}

fn convert_cmd(a: ConvertArgs) -> i32 {
    let mut opts = Options {
        resize: a.resize,
        strip_metadata: a.strip,
        max_bytes: a.max_bytes,
        trim_start: a.start,
        trim_end: a.end,
        audio_kbps: a.audio_kbps,
        mono: a.mono,
        no_audio: a.no_audio,
        // the Quality tab's saved values are the defaults, so the app and the CLI agree
        quality: settings::load_from(&settings::settings_path())
            .quality
            .clamped(),
        ..Options::default()
    };
    if let Some(f) = a.gif_fps {
        opts.quality.gif_fps = f;
    }
    if let Some(w) = a.gif_width {
        opts.quality.gif_width = w;
    }
    if let Some(m) = a.video_mbps {
        opts.quality.video_mbps = m;
    }
    if let Some(q) = a.quality {
        opts.quality.jpeg = q;
        opts.quality.webp = q;
    }
    opts.output.conflict = a.conflict;
    if let Some(d) = a.out.clone() {
        opts.output.dir = OutDir::Fixed(d);
    }
    for w in ignored_flags(&a) {
        err(&format!("warning: {w}"));
    }
    let files = expand_inputs(&a.files, a.recursive);
    if files.is_empty() {
        err("error: no files matched");
        return EXIT_USAGE;
    }
    let target = a.target;
    let style = Style {
        dry_run: a.dry_run,
        json: a.json,
        quiet: a.quiet,
    };
    run_files(&files, style, |path| {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return Err(
                "no file extension: add one (like .png) so Tessera can tell the format".into(),
            );
        };
        if Format::from_ext(ext) == Some(target) && !opts.transforms() {
            return Err(format!(
                "already {}: nothing to convert (add --strip, --max-edge, --percent or --max-size to change it)",
                target.ext().to_uppercase()
            ));
        }
        Ok((target, opts.clone()))
    })
}

/// Flags the chosen target cannot use: not an error, but silence would mislead.
fn ignored_flags(a: &ConvertArgs) -> Vec<String> {
    let t = a.target;
    let kind = t.kind();
    let av = kind == "audio" || kind == "video";
    let video = kind == "video";
    let mut v = Vec::new();
    let mut add = |on: bool, flag: &str, why: &str| {
        if on {
            v.push(format!("--{flag} is ignored for {} ({why})", t.ext()));
        }
    };
    add(a.mono && !av, "mono", "the result has no audio");
    add(
        a.no_audio && !video,
        "no-audio",
        "only video has a sound track to remove",
    );
    add(
        (a.start.is_some() || a.end.is_some()) && !av,
        "start and --end",
        "only audio and video can be cut",
    );
    add(
        a.audio_kbps.is_some() && !(av && !matches!(t, Format::Wav | Format::Flac)),
        "audio-bitrate",
        "lossless or no audio",
    );
    add(
        (a.gif_fps.is_some() || a.gif_width.is_some()) && t != Format::Gif,
        "gif-fps and --gif-width",
        "only GIF output uses them",
    );
    add(
        a.video_mbps.is_some() && !video,
        "video-mbps",
        "only video uses it",
    );
    add(
        a.quality.is_some() && !matches!(t, Format::Jpg | Format::Webp),
        "quality",
        "only JPG and WebP use it",
    );
    v
}

/// `*` and `?` in a file name, folders (their supported files), and `--recursive`.
fn expand_inputs(files: &[PathBuf], recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for f in files {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.contains('*') || name.contains('?') {
            let dir = f
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let mut hits: Vec<PathBuf> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| wildcard(name, n))
                })
                .collect();
            hits.sort();
            out.extend(hits);
        } else if f.is_dir() {
            collect_dir(f, recursive, 0, &mut out);
        } else {
            out.push(f.clone());
        }
    }
    out
}

fn collect_dir(dir: &Path, recursive: bool, depth: u8, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        let is_link = std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink());
        if p.is_file() {
            let supported = p
                .extension()
                .and_then(|e| e.to_str())
                .and_then(Format::from_ext)
                .is_some();
            if supported {
                out.push(p);
            }
        } else if p.is_dir() && recursive && depth < 8 && !is_link {
            collect_dir(&p, recursive, depth + 1, out);
        }
    }
}

/// Case-insensitive match of `name` against a pattern with `*` and `?`.
fn wildcard(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

fn preset_cmd(id: &str, files: &[PathBuf], flags: crate::cli::RunFlags) -> i32 {
    let s = settings::load_from(&settings::settings_path());
    let Some(p) = preset::find(&s, id) else {
        err(&format!(
            "unknown preset: {id} (see `{} list-presets`)",
            prog()
        ));
        return EXIT_USAGE;
    };
    let mut opts = p.options_for(&s);
    if let Some(d) = flags.out {
        opts.output.dir = OutDir::Fixed(d);
    }
    if flags.overwrite {
        opts.output.conflict = crate::convert::Conflict::Overwrite;
    }
    if flags.max_bytes.is_some() {
        opts.max_bytes = flags.max_bytes;
    }
    let style = Style {
        dry_run: flags.dry_run,
        json: flags.json,
        quiet: flags.quiet,
    };
    let files = expand_inputs(files, false);
    run_files(&files, style, |path| {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        p.resolve(ext, &opts)
            .map(|t| (t, opts.clone()))
            .ok_or_else(|| "this preset does not apply to this kind of file".to_string())
    })
}

fn merge_cmd(kind: crate::MergeKind, files: &[PathBuf], flags: crate::cli::RunFlags) -> i32 {
    let files = expand_inputs(files, false);
    if files.len() < 2 {
        err("error: a merge needs at least two files");
        return EXIT_USAGE;
    }
    let mut opts = settings::load_from(&settings::settings_path()).to_options();
    if let Some(d) = flags.out {
        opts.output.dir = OutDir::Fixed(d);
    }
    if flags.overwrite {
        opts.output.conflict = crate::convert::Conflict::Overwrite;
    }
    opts.max_bytes = flags.max_bytes;
    if flags.dry_run {
        out(&format!(
            "{} files -> one {} (dry run)",
            files.len(),
            kind.arg()
        ));
        return EXIT_OK;
    }
    let merged = crate::convert::combine::run(kind, &files, &opts, |_| {}).and_then(|dest| {
        // audio is merged as FLAC; --to asks for another result
        match flags.to.as_deref().and_then(Format::from_ext) {
            Some(t) if kind == crate::MergeKind::Audio && t != Format::Flac => {
                let mut o = opts.clone();
                o.max_bytes = None;
                let mut ff = None;
                let r = convert_one(&dest, t, &o, &mut ff, |_| {});
                let _ = std::fs::remove_file(&dest);
                r.map_err(crate::convert::ConvertError::Encode)
            }
            _ => Ok(dest),
        }
    });
    match merged {
        Ok(dest) => {
            let size = std::fs::metadata(&dest)
                .map(|m| human(m.len()))
                .unwrap_or_default();
            if flags.json {
                json_line(&dest, true, "merged", &files.len().to_string());
            } else if !flags.quiet {
                out(&format!(
                    "{} files -> {} ({size})",
                    files.len(),
                    dest.display()
                ));
            }
            EXIT_OK
        }
        Err(e) => {
            err(&format!("error: {}", e.message(Lang::En)));
            EXIT_FAILED
        }
    }
}

/// One line of JSON per file (`--json`).
fn pdf_cmd(
    tool: &crate::convert::pdf_tools::PdfTool,
    files: &[PathBuf],
    flags: crate::cli::RunFlags,
) -> i32 {
    let files = expand_inputs(files, false);
    let mut opts = settings::load_from(&settings::settings_path()).to_options();
    if let Some(d) = flags.out {
        opts.output.dir = OutDir::Fixed(d);
    }
    if flags.overwrite {
        opts.output.conflict = crate::convert::Conflict::Overwrite;
    }
    let mut failed = 0;
    for (i, path) in files.iter().enumerate() {
        let tag = format!("[{}/{}] {}", i + 1, files.len(), path.display());
        match crate::convert::pdf_tools::run(tool, path, &opts, |_| {}) {
            Ok(dest) if flags.json => json_line(path, true, "output", &dest.display().to_string()),
            Ok(dest) if !flags.quiet => out(&format!("{tag} -> {}", dest.display())),
            Ok(_) => {}
            Err(e) => {
                failed += 1;
                let why = e.message(Lang::En);
                if flags.json {
                    json_line(path, false, "error", &why);
                } else {
                    err(&format!("{tag}: {why}"));
                }
            }
        }
    }
    if failed > 0 {
        err(&format!("{failed} of {} failed", files.len()));
        return EXIT_FAILED;
    }
    EXIT_OK
}

fn json_line(file: &Path, ok: bool, key: &str, text: &str) {
    let v = serde_json::json!({ "file": file.display().to_string(), "ok": ok, key: text });
    out(&v.to_string());
}

/// Converts each file with what `plan` picks. Exit 1 if any failed or nothing could be done.
fn run_files(files: &[PathBuf], style: Style, plan: impl Fn(&Path) -> Plan) -> i32 {
    let (mut failed, mut skipped, mut done, mut ffmpeg_ok) = (0, 0, 0, None);
    let n = files.len();
    for (i, path) in files.iter().enumerate() {
        let tag = format!("[{}/{n}] {}", i + 1, path.display());
        let (target, opts) = match plan(path) {
            Ok(p) => p,
            Err(reason) => {
                if style.json {
                    json_line(path, false, "skipped", &reason);
                } else {
                    err(&format!("{tag}: skipped: {reason}"));
                }
                skipped += 1;
                continue;
            }
        };
        // the same checks a real run makes, so a dry run never promises what would fail
        if let Err(why) = precheck(path, target, &mut ffmpeg_ok) {
            if style.json {
                json_line(path, false, "error", &why);
            } else {
                err(&format!("{tag}: {why}"));
            }
            failed += 1;
            continue;
        }
        if style.dry_run {
            let name = dry_name(path, target, &opts);
            if style.json {
                json_line(path, true, "output", &name.display().to_string());
            } else if !style.quiet {
                out(&format!("{tag} -> {} (dry run)", name.display()));
            }
            done += 1;
            continue;
        }
        let mut last = 0.0f32;
        let on_progress = |p: f32| {
            let step = (p * 4.0).floor() / 4.0;
            if step > last && step < 1.0 && !style.quiet && !style.json {
                last = step;
                out(&format!("{tag} {:.0}%", step * 100.0));
            }
        };
        crate::convert::ALPHA_FLATTENED.store(false, std::sync::atomic::Ordering::Relaxed);
        match convert_one(path, target, &opts, &mut ffmpeg_ok, on_progress) {
            Ok(dest) => {
                done += 1;
                if crate::convert::ALPHA_FLATTENED.swap(false, std::sync::atomic::Ordering::Relaxed)
                    && !style.quiet
                {
                    err(&format!(
                        "{tag}: note: transparency was replaced with white ({} has none)",
                        target.ext().to_uppercase()
                    ));
                }
                if style.json {
                    json_line(path, true, "output", &dest.display().to_string());
                } else if !style.quiet {
                    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).ok();
                    let note = match (size(&dest), size(path)) {
                        (Some(n), Some(o)) => format!(" ({}, was {})", human(n), human(o)),
                        (Some(n), None) => format!(" ({})", human(n)),
                        _ => String::new(),
                    };
                    out(&format!("{tag} -> {}{note}", dest.display()));
                }
                let big = std::fs::metadata(&dest).is_ok_and(|m| m.len() > 20_000_000);
                if big && target == Format::Gif {
                    err(&format!(
                        "warning: {} is over 20 MB: lower --gif-width or --gif-fps for a smaller GIF",
                        dest.display()
                    ));
                }
            }
            Err(e) => {
                if style.json {
                    json_line(path, false, "error", &e);
                } else {
                    err(&format!("{tag}: {e}"));
                }
                failed += 1;
            }
        }
    }
    if failed > 0 {
        err(&format!("{failed} of {n} failed"));
    }
    if skipped > 0 && !style.json {
        err(&format!("{skipped} skipped"));
    }
    if failed > 0 || (done == 0 && skipped > 0) {
        EXIT_FAILED
    } else {
        EXIT_OK
    }
}

/// The file a real run would write, for `--dry-run`: the same naming and conflict rules, but no
/// folder is created.
fn dry_name(path: &Path, target: Format, opts: &Options) -> PathBuf {
    let plain = |dir: Option<&Path>| {
        let f = path.with_extension(target.ext());
        match dir {
            Some(d) => d.join(f.file_name().unwrap_or_default()),
            None => f,
        }
    };
    match &opts.output.dir {
        OutDir::Fixed(d) if !d.is_dir() => plain(Some(d)),
        _ => crate::convert::output::plan(path, target.ext(), &opts.output)
            .unwrap_or_else(|_| plain(None)),
    }
}

/// Why a file cannot be converted to `target`, found before any work starts.
fn precheck(path: &Path, target: Format, ffmpeg_ok: &mut Option<bool>) -> Result<(), String> {
    if !path.is_file() {
        return Err("File not found".into());
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let Some(from) = Format::from_ext(ext) else {
        return Err(format!(
            "cannot read .{ext} files (see `{} formats <ext>`)",
            prog()
        ));
    };
    if from != target && !supported_targets(ext).contains(&target) {
        return Err(format!(
            "cannot convert .{} to .{}: run `{} formats {}` to see what it can become",
            ext.to_lowercase(),
            target.ext(),
            prog(),
            ext.to_lowercase()
        ));
    }
    if uses_office(from, target) && crate::office::soffice().is_none() {
        return Err(t(Lang::En, "cli.no_office").into());
    }
    if (uses_ffmpeg(from, target) || needs_ffmpeg(target))
        && !*ffmpeg_ok.get_or_insert_with(|| ffmpeg::is_working(&ffmpeg::exe_path()))
    {
        return Err(t(Lang::En, "cli.no_ffmpeg").into());
    }
    Ok(())
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
    let r = if from.is_some_and(|f| uses_office(f, target)) {
        crate::office::convert(path, target, opts, progress)
    } else if from.is_some_and(|f| uses_ffmpeg(f, target) || opts.ffmpeg_only(f))
        || needs_ffmpeg(target)
    {
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
    } else if target == Format::Webp && opts.ffmpeg.is_none() {
        // lossy WebP wants libwebp from the installed ffmpeg; without it the file stays lossless
        let exe = ffmpeg::exe_path();
        let ok = *ffmpeg_ok.get_or_insert_with(|| ffmpeg::is_working(&exe));
        let o = if ok {
            opts.clone().ffmpeg(exe)
        } else {
            opts.clone()
        };
        convert(path, target, &o, progress)
    } else {
        convert(path, target, opts, progress)
    };
    r.map_err(|e| match e {
        crate::convert::ConvertError::TargetSize => {
            format!("{} (or add a smaller --max-edge)", e.message(lang))
        }
        e => e.message(lang),
    })
}

#[cfg(test)]
mod more_cli_tests {
    use super::*;
    use crate::cli::{parse, Cli, Command};

    #[test]
    fn wildcards_match_like_a_shell_would() {
        assert!(wildcard("*.png", "A.PNG"));
        assert!(wildcard("img_??.jpg", "img_07.jpg"));
        assert!(wildcard("*", "x"));
        assert!(!wildcard("*.png", "a.jpg"));
        assert!(!wildcard("a?c", "ac"));
    }

    #[test]
    fn folders_and_globs_expand_to_files() {
        let d = tempfile::tempdir().unwrap();
        for n in ["a.png", "b.jpg", "c.exe"] {
            std::fs::write(d.path().join(n), b"x").unwrap();
        }
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub").join("d.png"), b"x").unwrap();
        let names = |v: Vec<PathBuf>| -> Vec<String> {
            v.iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(
            names(expand_inputs(&[d.path().to_path_buf()], false)),
            ["a.png", "b.jpg"]
        );
        assert_eq!(
            names(expand_inputs(&[d.path().to_path_buf()], true)),
            ["a.png", "b.jpg", "d.png"]
        );
        assert_eq!(
            names(expand_inputs(&[d.path().join("*.png")], false)),
            ["a.png"]
        );
    }

    #[test]
    fn ignored_flags_name_the_flag_and_the_reason() {
        let args = |v: &[&str]| -> ConvertArgs {
            let a: Vec<String> = v.iter().map(|s| s.to_string()).collect();
            match parse(&[vec!["convert".to_string()], a].concat(), None) {
                Cli::Cmd(Command::Convert(c)) => c,
                other => panic!("not a convert: {other:?}"),
            }
        };
        let w = ignored_flags(&args(&[
            "x.png",
            "--to",
            "jpg",
            "--mono",
            "--gif-fps",
            "10",
            "--quality",
            "80",
        ]));
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w.iter().any(|m| m.contains("--mono")) && w.iter().any(|m| m.contains("gif")));
        assert!(
            ignored_flags(&args(&["x.mp4", "--to", "mp3", "--mono", "--start", "2"])).is_empty()
        );
        assert_eq!(
            ignored_flags(&args(&["x.mp4", "--to", "wav", "--audio-bitrate", "96"])).len(),
            1
        );
    }

    #[test]
    fn new_flags_parse() {
        let a: Vec<String> = [
            "convert",
            "dir",
            "--to",
            "jpg",
            "--recursive",
            "--dry-run",
            "--json",
            "--quiet",
            "--video-mbps",
            "4",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let Cli::Cmd(Command::Convert(c)) = parse(&a, None) else {
            panic!("did not parse");
        };
        assert!(c.recursive && c.dry_run && c.json && c.quiet);
        assert_eq!(c.video_mbps, Some(4));
    }
}
