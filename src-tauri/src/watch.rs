//! Folder watch rules: new files in a folder are converted automatically.
//!
//! The decision logic (`Stabilizer`, `Recent`, name and rule filters, validation) is pure and
//! takes the clock as an argument; `start` wires it to the `notify` crate.

use std::collections::HashMap;
use std::fs::{self, File};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, UNIX_EPOCH};

use notify::event::{CreateKind, ModifyKind, RenameMode};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};

use crate::convert::security;
use crate::convert::{allows, Conflict, Format, Options, OutDir};
use crate::i18n::{fmt, t, Lang};
use crate::settings::Settings;
use crate::{preset, settings};

pub const MAX_RULES: usize = 20;
/// A file must keep its size and modified time this long before it is queued.
pub const STABLE_MS: u64 = 1500;
const TICK_MS: u64 = 250;
/// A file that never settles (or never opens) is dropped after this long.
const GIVE_UP_MS: u64 = 10 * 60 * 1000;
/// How long a file we produced is remembered (loop guard).
const RECENT_TTL_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Preset { preset_id: String },
    Target { target: String },
}

impl Default for Action {
    fn default() -> Self {
        Action::Target {
            target: "png".into(),
        }
    }
}

/// `output` strings: `same_folder` | `subfolder:<name>` | `fixed:<dir>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputSpec {
    SameFolder,
    Subfolder(String),
    Fixed(PathBuf),
}

pub fn parse_output(s: &str) -> Option<OutputSpec> {
    if s == "same_folder" {
        return Some(OutputSpec::SameFolder);
    }
    let (kind, rest) = s.split_once(':')?;
    match kind {
        "subfolder" if !rest.is_empty() => Some(OutputSpec::Subfolder(rest.into())),
        "fixed" if !rest.is_empty() => Some(OutputSpec::Fixed(PathBuf::from(rest))),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchRule {
    /// `[a-z0-9_-]{1,40}`, unique; empty on save means "make one".
    pub id: String,
    pub enabled: bool,
    pub folder: String,
    pub recursive: bool,
    /// Source extensions to react to; empty means every convertible file.
    pub exts: Vec<String>,
    pub action: Action,
    pub output: String,
}

impl Default for WatchRule {
    fn default() -> Self {
        WatchRule {
            id: String::new(),
            enabled: true,
            folder: String::new(),
            recursive: false,
            exts: vec![],
            action: Action::default(),
            output: "same_folder".into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Watch {
    pub enabled: bool,
    pub rules: Vec<WatchRule>,
}

// ---------------------------------------------------------------- pure logic

/// What a stability check found out about a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// Deleted, moved away, not a file, hidden: forget it.
    Gone,
    /// Empty or cannot be opened for reading yet: keep waiting.
    Busy,
    Stat {
        len: u64,
        mtime: u64,
    },
}

struct Pending {
    first: u64,
    last_change: u64,
    stat: Option<(u64, u64)>,
}

/// Debounces file events and releases a path once it stopped changing.
pub struct Stabilizer {
    stable_ms: u64,
    pending: HashMap<PathBuf, Pending>,
}

impl Stabilizer {
    pub fn new(stable_ms: u64) -> Self {
        Stabilizer {
            stable_ms,
            pending: HashMap::new(),
        }
    }

    /// Starts tracking `path`; repeated events for a tracked path are absorbed.
    pub fn add(&mut self, path: PathBuf, now: u64) {
        self.pending.entry(path).or_insert(Pending {
            first: now,
            last_change: now,
            stat: None,
        });
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Paths that are stable now (and are forgotten); `probe` looks at the file system.
    pub fn tick(&mut self, now: u64, mut probe: impl FnMut(&Path) -> Probe) -> Vec<PathBuf> {
        let stable_ms = self.stable_ms;
        let mut ready = vec![];
        self.pending.retain(|path, p| {
            if now.saturating_sub(p.first) > GIVE_UP_MS {
                return false;
            }
            match probe(path) {
                Probe::Gone => false,
                Probe::Busy => {
                    p.stat = None;
                    p.last_change = now;
                    true
                }
                Probe::Stat { len, mtime } => {
                    if p.stat != Some((len, mtime)) {
                        p.stat = Some((len, mtime));
                        p.last_change = now;
                        true
                    } else if now.saturating_sub(p.last_change) >= stable_ms {
                        ready.push(path.clone());
                        false
                    } else {
                        true
                    }
                }
            }
        });
        ready
    }
}

fn key(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// Files we wrote recently; a watch rule must never convert its own output.
pub struct Recent {
    ttl: u64,
    map: HashMap<String, u64>,
}

impl Recent {
    pub fn new(ttl: u64) -> Self {
        Recent {
            ttl,
            map: HashMap::new(),
        }
    }

    pub fn add(&mut self, path: &Path, now: u64) {
        self.map.insert(key(path), now);
    }

    pub fn contains(&mut self, path: &Path, now: u64) -> bool {
        let ttl = self.ttl;
        self.map.retain(|_, t| now.saturating_sub(*t) <= ttl);
        self.map.contains_key(&key(path))
    }
}

/// Milliseconds on a monotonic clock.
pub fn mono_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

fn recent() -> &'static Mutex<Recent> {
    static R: OnceLock<Mutex<Recent>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Recent::new(RECENT_TTL_MS)))
}

/// Called for every file the queue produced, so no watch rule picks it up again.
pub fn note_output(path: &Path) {
    recent().lock().unwrap().add(path, mono_ms());
}

fn is_recent(path: &Path) -> bool {
    recent().lock().unwrap().contains(path, mono_ms())
}

/// Temp, partial, hidden-by-name and Office lock files, and our own `.name.<nanos>.tmp` outputs.
pub fn ignored_name(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with('.')
        || n.starts_with("~$")
        || n == "thumbs.db"
        || n == "desktop.ini"
        || [
            ".tmp",
            ".temp",
            ".part",
            ".partial",
            ".crdownload",
            ".download",
            ".opdownload",
        ]
        .iter()
        .any(|e| n.ends_with(e))
}

/// Does `path` fall under the rule's extension filter (empty = any convertible file)?
pub fn ext_matches(exts: &[String], path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if exts.is_empty() {
        Format::from_ext(ext).is_some()
    } else {
        exts.iter().any(|e| e.eq_ignore_ascii_case(ext))
    }
}

fn accepts(rule: &WatchRule, path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    !name.is_empty() && !ignored_name(name) && ext_matches(&rule.exts, path) && !is_recent(path)
}

fn probe_file(p: &Path) -> Probe {
    let Ok(m) = fs::metadata(p) else {
        return Probe::Gone;
    };
    // hidden or system files are not ours to touch
    if !m.is_file() || m.file_attributes() & 0x6 != 0 {
        return Probe::Gone;
    }
    if m.len() == 0 {
        return Probe::Busy;
    }
    match File::open(p) {
        Ok(_) => Probe::Stat {
            len: m.len(),
            mtime: m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as u64),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Probe::Gone,
        Err(_) => Probe::Busy,
    }
}

/// Target format and options for a stable file under `rule`, or None when the rule's action
/// does not apply to it (wrong kind, unknown preset, impossible conversion).
pub fn plan_job(rule: &WatchRule, path: &Path, s: &Settings) -> Option<(Format, Options)> {
    let ext = path.extension()?.to_str()?;
    let from = Format::from_ext(ext)?;
    let (target, mut opts) = match &rule.action {
        Action::Preset { preset_id } => {
            let p = preset::find(s, preset_id)?;
            let opts = p.options_for(s);
            (p.resolve(ext, &opts)?, opts)
        }
        Action::Target { target } => {
            let t = Format::from_ext(target).filter(|f| Format::ALL.contains(f))?;
            let opts = s.to_options();
            allows(ext, from, t, &opts).then_some(())?;
            (t, opts)
        }
    };
    // unattended jobs never recycle the original
    if opts.output.conflict == Conflict::ReplaceOriginal {
        opts.output.conflict = Conflict::AutoRename;
    }
    opts.output.dir = match parse_output(&rule.output)? {
        OutputSpec::SameFolder => OutDir::SameAsSource,
        OutputSpec::Subfolder(n) => OutDir::Fixed(path.parent()?.join(n)),
        OutputSpec::Fixed(d) => OutDir::Fixed(d),
    };
    Some((target, opts))
}

// ---------------------------------------------------------------- validation

/// Folders a rule may never watch or write into: system folders and our own data.
fn forbidden_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = [
        "SystemRoot",
        "WINDIR",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "ProgramData",
    ]
    .iter()
    .filter_map(std::env::var_os)
    .map(PathBuf::from)
    .collect();
    v.push(settings::data_dir());
    v
}

fn norm(p: &Path) -> String {
    let s = key(p);
    s.strip_prefix(r"\\?\")
        .unwrap_or(&s)
        .trim_end_matches('\\')
        .to_string()
}

#[derive(Debug, PartialEq, Eq)]
enum FolderIssue {
    /// Relative, UNC or device path.
    Bad,
    Forbidden,
}

/// Lexical check (the folder may not exist yet). `recursive` also refuses a parent of a
/// forbidden folder, since it would be watched too.
fn check_folder(folder: &Path, recursive: bool, forbidden: &[PathBuf]) -> Result<(), FolderIssue> {
    let raw = folder.to_string_lossy();
    if raw.starts_with(r"\\")
        || raw.starts_with("//")
        || !folder.is_absolute()
        || folder.components().any(|c| c.as_os_str() == "..")
    {
        return Err(FolderIssue::Bad);
    }
    let n = norm(folder);
    // a drive root ("c:")
    if n.len() <= 2 {
        return Err(FolderIssue::Forbidden);
    }
    for f in forbidden {
        let f = norm(f);
        if f.is_empty() {
            continue;
        }
        let inside = n == f || n.starts_with(&format!("{f}\\"));
        let contains = recursive && f.starts_with(&format!("{n}\\"));
        if inside || contains {
            return Err(FolderIssue::Forbidden);
        }
    }
    Ok(())
}

fn folder_error(folder: &str, recursive: bool, lang: Lang) -> Result<(), String> {
    // junctions and `..` resolve to their real place when the folder exists
    let p = Path::new(folder);
    let real = security::validate_dir(p).unwrap_or_else(|_| p.to_path_buf());
    match check_folder(&real, recursive, &forbidden_dirs()) {
        Ok(()) => Ok(()),
        Err(FolderIssue::Bad) => Err(fmt(lang, "watch.bad_folder", &[("folder", folder)])),
        Err(FolderIssue::Forbidden) => {
            Err(fmt(lang, "watch.forbidden_folder", &[("folder", folder)]))
        }
    }
}

impl Watch {
    /// Checks and normalizes in place; the error is shown to the user. A folder that does
    /// not exist (yet) is allowed: its rule just reports an error in the status.
    pub fn validate(&mut self, s: &Settings, lang: Lang) -> Result<(), String> {
        if self.rules.len() > MAX_RULES {
            return Err(fmt(
                lang,
                "watch.too_many",
                &[("max", &MAX_RULES.to_string())],
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for (i, r) in self.rules.iter_mut().enumerate() {
            r.id = r.id.trim().to_lowercase();
            if r.id.is_empty() {
                r.id = format!("w{}{i}", crate::history::now_ms());
            }
            let id_ok = r.id.len() <= 40
                && r.id.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                });
            if !id_ok || !seen.insert(r.id.clone()) {
                return Err(t(lang, "watch.bad_id").into());
            }
            r.folder = r.folder.trim().to_string();
            folder_error(&r.folder, r.recursive, lang)?;
            let mut exts = vec![];
            for e in &r.exts {
                let e = e.trim().trim_start_matches('.').to_lowercase();
                if e.is_empty() {
                    continue;
                }
                if Format::from_ext(&e).is_none() {
                    return Err(fmt(lang, "watch.bad_ext", &[("ext", &e)]));
                }
                if !exts.contains(&e) {
                    exts.push(e);
                }
            }
            r.exts = exts;
            match &mut r.action {
                Action::Preset { preset_id } => {
                    *preset_id = preset_id.trim().to_lowercase();
                    if preset::find(s, preset_id).is_none() {
                        return Err(t(lang, "preset.not_found").into());
                    }
                }
                Action::Target { target } => {
                    *target = target.trim().to_lowercase();
                    let ok = Format::from_ext(target)
                        .is_some_and(|f| Format::ALL.contains(&f) && f.ext() == target);
                    if !ok {
                        return Err(t(lang, "preset.bad_target").into());
                    }
                }
            }
            match parse_output(r.output.trim()) {
                None => return Err(t(lang, "watch.bad_output").into()),
                Some(OutputSpec::SameFolder) => r.output = "same_folder".into(),
                Some(OutputSpec::Subfolder(n)) => {
                    let n = n.trim().to_string();
                    crate::convert::output::validate_template(&n, lang)?;
                    if n.contains(['{', '}']) {
                        return Err(t(lang, "watch.bad_output").into());
                    }
                    r.output = format!("subfolder:{n}");
                }
                Some(OutputSpec::Fixed(d)) => {
                    let d = d.to_string_lossy().trim().to_string();
                    folder_error(&d, false, lang)?;
                    r.output = format!("fixed:{d}");
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- running watchers

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleStatus {
    pub id: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// Receives a stable, matching file. Called on the watcher thread.
pub type Sink = Arc<dyn Fn(&WatchRule, PathBuf) + Send + Sync>;

#[derive(Clone, Copy)]
pub struct Timing {
    pub tick_ms: u64,
    pub stable_ms: u64,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            tick_ms: TICK_MS,
            stable_ms: STABLE_MS,
        }
    }
}

pub struct Running {
    watchers: Vec<notify::RecommendedWatcher>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// One entry per rule, in order. `ok` is false only when the rule is enabled but failed.
    pub status: Vec<RuleStatus>,
}

impl Running {
    /// Stops the watchers and waits for the worker thread (at most one tick).
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.watchers.clear();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Active {
    rule: WatchRule,
    stab: Stabilizer,
}

/// Starts one watcher per enabled rule (nothing when `watch.enabled` is off).
pub fn start(watch: &Watch, lang: Lang, sink: Sink, timing: Timing) -> Running {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel::<(usize, notify::Result<Event>)>();
    let (mut watchers, mut active, mut status) = (vec![], vec![], vec![]);
    for rule in &watch.rules {
        let mut st = RuleStatus {
            id: rule.id.clone(),
            ok: true,
            error: None,
        };
        if watch.enabled && rule.enabled {
            match start_one(rule, lang, active.len(), &tx) {
                Ok(w) => {
                    watchers.push(w);
                    active.push(Active {
                        rule: rule.clone(),
                        stab: Stabilizer::new(timing.stable_ms),
                    });
                }
                Err(e) => {
                    st.ok = false;
                    st.error = Some(e);
                }
            }
        }
        status.push(st);
    }
    drop(tx);
    let thread = (!active.is_empty()).then(|| {
        let stop = stop.clone();
        std::thread::spawn(move || run_loop(active, rx, sink, stop, timing))
    });
    Running {
        watchers,
        stop,
        thread,
        status,
    }
}

fn start_one(
    rule: &WatchRule,
    lang: Lang,
    idx: usize,
    tx: &std::sync::mpsc::Sender<(usize, notify::Result<Event>)>,
) -> Result<notify::RecommendedWatcher, String> {
    let folder = Path::new(&rule.folder);
    if !folder.is_dir() {
        return Err(fmt(
            lang,
            "watch.missing_folder",
            &[("folder", &rule.folder)],
        ));
    }
    folder_error(&rule.folder, rule.recursive, lang)?;
    let tx = tx.clone();
    let mut w = notify::recommended_watcher(move |res| {
        let _ = tx.send((idx, res));
    })
    .map_err(|e| fmt(lang, "watch.start_failed", &[("err", &e.to_string())]))?;
    let mode = if rule.recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    w.watch(folder, mode)
        .map_err(|e| fmt(lang, "watch.start_failed", &[("err", &e.to_string())]))?;
    Ok(w)
}

/// Paths of a file that appeared: created, or renamed/moved in. Writes to old files do not count.
fn arrived(ev: &Event) -> Vec<&PathBuf> {
    match ev.kind {
        EventKind::Create(CreateKind::File | CreateKind::Any | CreateKind::Other) => {
            ev.paths.iter().collect()
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => vec![],
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            ev.paths.last().into_iter().collect()
        }
        EventKind::Modify(ModifyKind::Name(_)) => ev.paths.iter().collect(),
        _ => vec![],
    }
}

fn run_loop(
    mut active: Vec<Active>,
    rx: std::sync::mpsc::Receiver<(usize, notify::Result<Event>)>,
    sink: Sink,
    stop: Arc<AtomicBool>,
    timing: Timing,
) {
    let tick = Duration::from_millis(timing.tick_ms);
    let mut last = mono_ms();
    while !stop.load(Ordering::SeqCst) {
        let mut disconnected = false;
        match rx.recv_timeout(tick) {
            Ok(first) => {
                for (i, res) in std::iter::once(first).chain(rx.try_iter()) {
                    let (Ok(ev), Some(a)) = (res, active.get_mut(i)) else {
                        continue;
                    };
                    for p in arrived(&ev) {
                        if accepts(&a.rule, p) {
                            a.stab.add(p.clone(), mono_ms());
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => disconnected = true,
        }
        let now = mono_ms();
        if now.saturating_sub(last) >= timing.tick_ms {
            last = now;
            for a in &mut active {
                for p in a.stab.tick(now, probe_file) {
                    if !stop.load(Ordering::SeqCst) && !is_recent(&p) {
                        sink(&a.rule, p);
                    }
                }
            }
        }
        if disconnected {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn stat(len: u64) -> Probe {
        Probe::Stat { len, mtime: 7 }
    }

    #[test]
    fn stabilizer_waits_for_quiet_size_and_mtime() {
        let mut s = Stabilizer::new(1500);
        s.add(p("a.png"), 0);
        // growing file: the timer restarts on every change
        assert!(s.tick(250, |_| stat(10)).is_empty());
        assert!(s.tick(1000, |_| stat(20)).is_empty());
        assert!(s.tick(2400, |_| stat(20)).is_empty());
        assert_eq!(s.tick(2500, |_| stat(20)), vec![p("a.png")]);
        assert!(s.is_empty());
        // same size, new mtime also counts as change
        s.add(p("b.png"), 0);
        assert!(s.tick(0, |_| Probe::Stat { len: 5, mtime: 1 }).is_empty());
        assert!(s
            .tick(1000, |_| Probe::Stat { len: 5, mtime: 2 })
            .is_empty());
        assert!(s
            .tick(2400, |_| Probe::Stat { len: 5, mtime: 2 })
            .is_empty());
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn stabilizer_debounces_repeated_events() {
        let mut s = Stabilizer::new(100);
        for t in 0..5 {
            s.add(p("a.png"), t);
        }
        assert_eq!(s.len(), 1);
        s.tick(10, |_| stat(1));
        assert_eq!(s.tick(110, |_| stat(1)), vec![p("a.png")]);
        // once released the path can be tracked again
        s.add(p("a.png"), 200);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn busy_files_wait_and_gone_files_are_dropped() {
        let mut s = Stabilizer::new(100);
        s.add(p("locked.png"), 0);
        s.add(p("gone.png"), 0);
        let probe = |path: &Path| {
            if path == Path::new("gone.png") {
                Probe::Gone
            } else {
                Probe::Busy
            }
        };
        assert!(s.tick(500, probe).is_empty());
        assert_eq!(s.len(), 1);
        // unlocked: needs a fresh quiet period
        assert!(s.tick(600, |_| stat(3)).is_empty());
        assert_eq!(s.tick(700, |_| stat(3)), vec![p("locked.png")]);
        // never settles: given up
        s.add(p("x.png"), 0);
        assert!(s.tick(GIVE_UP_MS + 1, |_| Probe::Busy).is_empty());
        assert!(s.is_empty());
    }

    #[test]
    fn recent_outputs_expire() {
        let mut r = Recent::new(1000);
        r.add(Path::new(r"C:\W\Out.JPG"), 0);
        assert!(r.contains(Path::new("c:/w/out.jpg"), 500));
        assert!(!r.contains(Path::new(r"C:\w\other.jpg"), 500));
        assert!(!r.contains(Path::new(r"C:\w\out.jpg"), 1001));
    }

    #[test]
    fn loop_guard_blocks_own_output_in_watched_folder() {
        let rule = WatchRule {
            folder: r"C:\w".into(),
            ..WatchRule::default()
        };
        let out = Path::new(r"C:\w\photo.jpg");
        assert!(accepts(&rule, out));
        note_output(out);
        assert!(!accepts(&rule, out));
        assert!(accepts(&rule, Path::new(r"C:\w\photo2.jpg")));
    }

    #[test]
    fn temp_and_partial_names_are_ignored() {
        for n in [
            "a.tmp",
            "a.png.part",
            "x.crdownload",
            ".hidden.png",
            ".photo.png.123456.tmp",
            "~$doc.pdf",
            "Thumbs.db",
            "A.PART",
        ] {
            assert!(ignored_name(n), "{n}");
        }
        for n in ["photo.png", "my.tmp.png", "report (1).pdf"] {
            assert!(!ignored_name(n), "{n}");
        }
    }

    #[test]
    fn extension_filter() {
        let all: Vec<String> = vec![];
        assert!(ext_matches(&all, Path::new("a.PNG")));
        assert!(ext_matches(&all, Path::new("a.mp4")));
        assert!(!ext_matches(&all, Path::new("a.exe")));
        assert!(!ext_matches(&all, Path::new("noext")));
        let only = vec!["png".to_string()];
        assert!(ext_matches(&only, Path::new("a.PNG")));
        assert!(!ext_matches(&only, Path::new("a.jpg")));
    }

    #[test]
    fn output_strings() {
        assert_eq!(parse_output("same_folder"), Some(OutputSpec::SameFolder));
        assert_eq!(
            parse_output("subfolder:out"),
            Some(OutputSpec::Subfolder("out".into()))
        );
        assert_eq!(
            parse_output(r"fixed:C:\x"),
            Some(OutputSpec::Fixed(p(r"C:\x")))
        );
        for bad in ["", "subfolder:", "fixed:", "other:x", "same"] {
            assert_eq!(parse_output(bad), None, "{bad}");
        }
    }

    fn rule(folder: &str) -> WatchRule {
        WatchRule {
            id: "r1".into(),
            folder: folder.into(),
            ..WatchRule::default()
        }
    }

    fn validate(w: &mut Watch) -> Result<(), String> {
        w.validate(&Settings::default(), Lang::En)
    }

    #[test]
    fn folder_rules() {
        let sys = vec![p(r"C:\Windows"), p(r"C:\Users\u\AppData\Roaming\new-app")];
        let ok = |f: &str, rec| check_folder(Path::new(f), rec, &sys);
        assert_eq!(ok(r"D:\Photos", false), Ok(()));
        assert_eq!(ok(r"D:\Photos\", true), Ok(()));
        assert_eq!(ok(r"\\srv\share", false), Err(FolderIssue::Bad));
        assert_eq!(ok("//srv/share", false), Err(FolderIssue::Bad));
        assert_eq!(ok(r"\\?\C:\x", false), Err(FolderIssue::Bad));
        assert_eq!(ok("photos", false), Err(FolderIssue::Bad));
        assert_eq!(ok(r"D:\a\..\b", false), Err(FolderIssue::Bad));
        assert_eq!(ok(r"C:\", false), Err(FolderIssue::Forbidden));
        assert_eq!(
            ok(r"c:\windows\system32", false),
            Err(FolderIssue::Forbidden)
        );
        assert_eq!(
            ok(r"C:\Users\u\AppData\Roaming\new-app\sub", false),
            Err(FolderIssue::Forbidden)
        );
        // a recursive watch over a parent would include the app data folder
        assert_eq!(ok(r"C:\Users\u", true), Err(FolderIssue::Forbidden));
        assert_eq!(ok(r"C:\Users\u", false), Ok(()));
    }

    #[test]
    fn validation_normalizes_and_rejects() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_string_lossy().to_string();
        let mut w = Watch {
            enabled: true,
            rules: vec![WatchRule {
                id: String::new(),
                exts: vec![".PNG".into(), "png".into(), " ".into(), "jpg".into()],
                action: Action::Target {
                    target: "WEBP".into(),
                },
                output: " subfolder:out ".into(),
                ..rule(&format!("{dir} "))
            }],
        };
        validate(&mut w).unwrap();
        let r = &w.rules[0];
        assert!(r.id.starts_with('w') && r.folder == dir);
        assert_eq!(r.exts, ["png", "jpg"]);
        assert_eq!(
            r.action,
            Action::Target {
                target: "webp".into()
            }
        );
        assert_eq!(r.output, "subfolder:out");

        let bad = |f: &dyn Fn(&mut WatchRule)| {
            let mut r = rule(&dir);
            f(&mut r);
            validate(&mut Watch {
                enabled: true,
                rules: vec![r],
            })
            .is_err()
        };
        assert!(!bad(&|_| {}));
        assert!(bad(&|r| r.folder = r"\\srv\x".into()));
        assert!(bad(&|r| r.folder = r"C:\Windows".into()));
        assert!(bad(&|r| r.id = "A B".into()));
        assert!(bad(&|r| r.exts = vec!["exe".into()]));
        assert!(bad(&|r| r.action = Action::Target {
            target: "heic".into()
        }));
        assert!(bad(&|r| r.action = Action::Preset {
            preset_id: "nope".into()
        }));
        assert!(!bad(&|r| r.action = Action::Preset {
            preset_id: "web".into()
        }));
        for out in [
            "",
            "x",
            "subfolder:a/b",
            "subfolder:..",
            "subfolder:{name}",
            "fixed:rel",
            r"fixed:\\srv\x",
            r"fixed:C:\Windows",
        ] {
            assert!(bad(&|r| r.output = out.into()), "{out}");
        }
        assert!(!bad(&|r| r.output = format!("fixed:{dir}")));
        // a folder that does not exist yet is fine; the status reports it later
        assert!(!bad(&|r| r.folder = r"D:\definitely\not\here".into()));
    }

    #[test]
    fn rule_count_and_duplicate_ids() {
        let many = |n: usize| Watch {
            enabled: true,
            rules: (0..n)
                .map(|i| WatchRule {
                    id: format!("r{i}"),
                    ..rule(r"D:\x")
                })
                .collect(),
        };
        assert!(validate(&mut many(MAX_RULES)).is_ok());
        assert!(validate(&mut many(MAX_RULES + 1)).is_err());
        let mut dup = many(2);
        dup.rules[1].id = "R0".into();
        assert!(validate(&mut dup).is_err());
    }

    #[test]
    fn json_shape_and_old_files() {
        let w: Watch = serde_json::from_str(
            r#"{"enabled":true,"rules":[{"id":"a","folder":"D:\\x",
            "action":{"kind":"preset","preset_id":"web"},"output":"fixed:D:\\o"}]}"#,
        )
        .unwrap();
        assert!(w.enabled && w.rules[0].enabled && !w.rules[0].recursive);
        assert_eq!(
            w.rules[0].action,
            Action::Preset {
                preset_id: "web".into()
            }
        );
        let v = serde_json::to_value(Watch::default()).unwrap();
        assert_eq!(v, serde_json::json!({"enabled":false,"rules":[]}));
        let t = serde_json::to_value(Action::default()).unwrap();
        assert_eq!(t, serde_json::json!({"kind":"target","target":"png"}));
    }

    #[test]
    fn plans_jobs() {
        let s = Settings::default();
        let mut r = rule(r"D:\w");
        r.action = Action::Target {
            target: "jpg".into(),
        };
        let (t, o) = plan_job(&r, Path::new(r"D:\w\a.png"), &s).unwrap();
        assert_eq!(t, Format::Jpg);
        assert!(matches!(o.output.dir, OutDir::SameAsSource));
        assert!(plan_job(&r, Path::new(r"D:\w\a.mp3"), &s).is_none());
        assert!(plan_job(&r, Path::new(r"D:\w\a.jpg"), &s).is_none());
        r.output = "subfolder:done".into();
        let (_, o) = plan_job(&r, Path::new(r"D:\w\sub\a.png"), &s).unwrap();
        assert!(matches!(&o.output.dir, OutDir::Fixed(d) if d == Path::new(r"D:\w\sub\done")));
        r.action = Action::Preset {
            preset_id: "web".into(),
        };
        let (t, o) = plan_job(&r, Path::new(r"D:\w\a.png"), &s).unwrap();
        assert_eq!(t, Format::Jpg);
        assert!(o.strip_metadata);
        // never recycles originals unattended
        let s = Settings {
            conflict: crate::settings::ConflictMode::ReplaceOriginal,
            ..Settings::default()
        };
        let (_, o) = plan_job(&r, Path::new(r"D:\w\a.png"), &s).unwrap();
        assert_eq!(o.output.conflict, Conflict::AutoRename);
    }

    #[test]
    fn missing_folder_is_a_status_not_a_crash() {
        let w = Watch {
            enabled: true,
            rules: vec![rule(r"D:\definitely\not\here")],
        };
        let r = start(&w, Lang::Tr, Arc::new(|_, _| {}), Timing::default());
        assert!(!r.status[0].ok);
        let en = start(&w, Lang::En, Arc::new(|_, _| {}), Timing::default());
        assert_ne!(r.status[0].error, en.status[0].error);
        r.stop();
        en.stop();
        // disabled watch: nothing runs, no errors
        let off = Watch {
            enabled: false,
            ..w
        };
        let r = start(&off, Lang::En, Arc::new(|_, _| {}), Timing::default());
        assert!(r.status[0].ok && r.thread.is_none());
        r.stop();
    }
}
