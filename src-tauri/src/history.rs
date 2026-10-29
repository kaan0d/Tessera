//! `history.json`: the last 200 finished jobs, and undo for them. Undo only ever moves the
//! output we recorded to the Recycle Bin (after checking it is still the file we wrote) and
//! restores an original that ReplaceOriginal had put in the Recycle Bin.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::i18n::{Lang, fmt, t};

pub const MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Convert,
    Preset,
    Merge,
    /// Reserved for the folder-watch wave.
    Watch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: u64,
    /// Unix milliseconds when the job finished.
    pub time: u64,
    /// Unix milliseconds when the job started (bounds the Recycle Bin search on undo).
    pub started: u64,
    pub input: String,
    pub output: Option<String>,
    pub kind: Kind,
    /// Human text in the language of the time ("Convert to JPG", preset name, ...).
    pub label: String,
    pub ok: bool,
    pub error: Option<String>,
    pub replaced_original: bool,
    pub original_recycled: bool,
    pub undone: bool,
    /// Size and modified time (ms) of the output when we wrote it; undo refuses a changed file.
    pub output_len: Option<u64>,
    pub output_mtime: Option<u64>,
}

impl Default for Entry {
    fn default() -> Self {
        Entry {
            id: 0,
            time: 0,
            started: 0,
            input: String::new(),
            output: None,
            kind: Kind::Convert,
            label: String::new(),
            ok: false,
            error: None,
            replaced_original: false,
            original_recycled: false,
            undone: false,
            output_len: None,
            output_mtime: None,
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn history_path() -> PathBuf {
    crate::settings::data_dir().join("history.json")
}

fn mtime_ms(m: &fs::Metadata) -> Option<u64> {
    m.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

pub struct Store {
    path: PathBuf,
    entries: Vec<Entry>,
    next_id: u64,
    pub enabled: bool,
}

pub type Shared = Arc<Mutex<Store>>;

impl Store {
    /// A missing or corrupt file gives an empty history.
    pub fn load(path: PathBuf, enabled: bool) -> Store {
        let entries: Vec<Entry> = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let next_id = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        Store {
            path,
            entries,
            next_id,
            enabled,
        }
    }

    fn save(&self) {
        let _ = (|| -> std::io::Result<()> {
            if let Some(d) = self.path.parent() {
                fs::create_dir_all(d)?;
            }
            let tmp = self.path.with_extension("json.tmp");
            fs::write(&tmp, serde_json::to_vec(&self.entries)?)?;
            fs::rename(&tmp, &self.path)
        })();
    }

    /// Adds an entry (id and output stats are filled in here) and returns its id; None when
    /// history is off. Keeps the newest `MAX_ENTRIES`.
    pub fn record(&mut self, mut e: Entry) -> Option<u64> {
        if !self.enabled {
            return None;
        }
        e.id = self.next_id;
        self.next_id += 1;
        e.time = now_ms();
        if let (true, Some(out)) = (e.ok, &e.output)
            && let Some((len, mtime)) = stat(Path::new(out))
        {
            e.output_len = Some(len);
            e.output_mtime = mtime;
        }
        let id = e.id;
        self.entries.push(e);
        let extra = self.entries.len().saturating_sub(MAX_ENTRIES);
        self.entries.drain(..extra);
        self.save();
        Some(id)
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Entry> {
        self.entries.iter().rev().cloned().collect()
    }

    pub fn get(&self, id: u64) -> Option<Entry> {
        self.entries.iter().find(|e| e.id == id).cloned()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.save();
    }

    fn mark_undone(&mut self, id: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.undone = true;
        }
        self.save();
    }
}

/// An item of the Recycle Bin, reduced to what the match needs.
#[derive(Debug, Clone)]
pub struct BinItem {
    pub name: String,
    pub parent: PathBuf,
    /// Unix seconds.
    pub deleted: i64,
}

/// Which bin item is the original that ReplaceOriginal recycled at `path`: same folder and
/// name (the bin may hide the extension), deleted since `since_secs`; the newest wins.
pub fn pick_bin_item(items: &[BinItem], path: &Path, since_secs: i64) -> Option<usize> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    let stem = path.file_stem()?.to_string_lossy().to_lowercase();
    let parent = path.parent()?.to_string_lossy().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, i)| {
            let n = i.name.to_lowercase();
            (n == name || n == stem)
                && i.parent.to_string_lossy().to_lowercase() == parent
                && i.deleted >= since_secs
        })
        .max_by_key(|(_, i)| i.deleted)
        .map(|(idx, _)| idx)
}

/// What undo would do, decided from the entry and the disk before anything is touched.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Move the output to the Recycle Bin; also restore the original when asked.
    Go {
        restore_original: bool,
    },
    Refuse(&'static str),
}

/// Pure decision: `output_now` is (len, mtime ms) of the file on disk, None when missing.
/// Size and modified time of an output. A folder of results (split pages, slides) counts the
/// bytes of the files directly in it; adding or removing one changes its time too.
fn stat(out: &Path) -> Option<(u64, Option<u64>)> {
    let m = fs::metadata(out).ok()?;
    if m.is_file() {
        return Some((m.len(), mtime_ms(&m)));
    }
    if !m.is_dir() {
        return None;
    }
    let bytes = fs::read_dir(out)
        .ok()?
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(fs::Metadata::is_file)
        .map(|m| m.len())
        .sum();
    Some((bytes, mtime_ms(&m)))
}

pub fn plan(e: &Entry, output_now: Option<(u64, Option<u64>)>) -> Plan {
    if !e.ok || e.undone || e.output.is_none() {
        return Plan::Refuse("undo.nothing");
    }
    let Some((len, mtime)) = output_now else {
        return Plan::Refuse("undo.no_output");
    };
    // an entry without a recorded size cannot prove the file is still our output
    if e.output_len != Some(len) || (e.output_mtime.is_some() && e.output_mtime != mtime) {
        return Plan::Refuse("undo.changed");
    }
    Plan::Go {
        restore_original: e.original_recycled,
    }
}

fn restore_original(input: &Path, started_ms: u64, lang: Lang) -> Result<(), String> {
    let path_s = input.display().to_string();
    let fail = |d: &str| {
        fmt(
            lang,
            "undo.restore_failed",
            &[("detail", d), ("path", &path_s)],
        )
    };
    if input.exists() {
        return Err(fmt(lang, "undo.occupied", &[("path", &path_s)]));
    }
    let items = trash::os_limited::list().map_err(|e| fail(&e.to_string()))?;
    let view: Vec<BinItem> = items
        .iter()
        .map(|i| BinItem {
            name: i.name.to_string_lossy().into(),
            parent: i.original_parent.clone(),
            deleted: i.time_deleted,
        })
        .collect();
    let idx = pick_bin_item(&view, input, (started_ms / 1000) as i64 - 1)
        .ok_or_else(|| fail("not found in the Recycle Bin"))?;
    trash::os_limited::restore_all([items[idx].clone()]).map_err(|e| fail(&e.to_string()))
}

/// Undoes history entry `id`. Order: check, recycle the output (a locked output stops here
/// with nothing changed), restore the original, mark the entry undone. A failed restore still
/// marks it: the output is gone, so a retry could only fail.
pub fn undo(store: &Shared, id: u64, lang: Lang) -> Result<(), String> {
    let e = store
        .lock()
        .unwrap()
        .get(id)
        .ok_or_else(|| t(lang, "undo.not_found").to_string())?;
    let out = PathBuf::from(e.output.clone().unwrap_or_default());
    match plan(&e, stat(&out)) {
        Plan::Refuse(k) => Err(t(lang, k).to_string()),
        Plan::Go {
            restore_original: r,
        } => {
            let input = Path::new(&e.input);
            if r && input.exists() {
                let path = input.display().to_string();
                return Err(fmt(lang, "undo.occupied", &[("path", &path)]));
            }
            trash::delete(&out)
                .map_err(|x| fmt(lang, "undo.delete_failed", &[("detail", &x.to_string())]))?;
            let restored = if r {
                restore_original(input, e.started, lang)
            } else {
                Ok(())
            };
            store.lock().unwrap().mark_undone(id);
            restored
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(out: &Path) -> Entry {
        Entry {
            input: "C:\\in.png".into(),
            output: Some(out.display().to_string()),
            ok: true,
            label: "x".into(),
            ..Entry::default()
        }
    }

    #[test]
    fn persists_caps_and_clears() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("h").join("history.json");
        let mut s = Store::load(p.clone(), true);
        for _ in 0..(MAX_ENTRIES + 5) {
            s.record(Entry::default());
        }
        assert_eq!(s.list().len(), MAX_ENTRIES);
        assert_eq!(s.list()[0].id, (MAX_ENTRIES + 5) as u64);
        let s2 = Store::load(p.clone(), true);
        assert_eq!(s2.list().len(), MAX_ENTRIES);
        // ids keep counting after a reload
        let mut s2 = s2;
        assert_eq!(s2.record(Entry::default()), Some((MAX_ENTRIES + 6) as u64));
        s2.clear();
        assert!(Store::load(p.clone(), true).list().is_empty());
        fs::write(&p, b"nope").unwrap();
        assert!(Store::load(p, true).list().is_empty());
    }

    #[test]
    fn disabled_records_nothing() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::load(d.path().join("h.json"), false);
        assert_eq!(s.record(Entry::default()), None);
        assert!(s.list().is_empty());
    }

    #[test]
    fn record_stats_the_output() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("o.png");
        fs::write(&f, b"abc").unwrap();
        let mut s = Store::load(d.path().join("h.json"), true);
        let id = s.record(done(&f)).unwrap();
        let e = s.get(id).unwrap();
        assert_eq!(e.output_len, Some(3));
        assert!(e.output_mtime.is_some() && e.time > 0);
    }

    #[test]
    fn plan_decisions() {
        let mut e = done(Path::new("o.png"));
        e.output_len = Some(3);
        e.output_mtime = Some(10);
        assert_eq!(
            plan(&e, Some((3, Some(10)))),
            Plan::Go {
                restore_original: false
            }
        );
        assert_eq!(plan(&e, None), Plan::Refuse("undo.no_output"));
        assert_eq!(plan(&e, Some((4, Some(10)))), Plan::Refuse("undo.changed"));
        assert_eq!(plan(&e, Some((3, Some(11)))), Plan::Refuse("undo.changed"));
        e.original_recycled = true;
        assert_eq!(
            plan(&e, Some((3, Some(10)))),
            Plan::Go {
                restore_original: true
            }
        );
        e.undone = true;
        assert_eq!(plan(&e, Some((3, Some(10)))), Plan::Refuse("undo.nothing"));
        let failed = Entry::default();
        assert_eq!(plan(&failed, None), Plan::Refuse("undo.nothing"));
    }

    #[test]
    fn bin_item_choice() {
        let item = |n: &str, p: &str, t: i64| BinItem {
            name: n.into(),
            parent: PathBuf::from(p),
            deleted: t,
        };
        let items = vec![
            item("a.png", r"C:\pics", 100),
            item("a.png", r"C:\pics", 500),
            item("a.png", r"C:\other", 600),
            item("b.png", r"C:\pics", 600),
            item("A", r"c:\PICS", 300),
        ];
        let p = Path::new(r"C:\pics\a.png");
        assert_eq!(pick_bin_item(&items, p, 400), Some(1));
        assert_eq!(pick_bin_item(&items, p, 200), Some(1));
        assert_eq!(pick_bin_item(&items, p, 700), None);
        // extension hidden by the bin, case-insensitive folder
        assert_eq!(pick_bin_item(&items[4..], p, 0), Some(0));
        assert_eq!(
            pick_bin_item(&items, Path::new(r"C:\pics\zzz.png"), 0),
            None
        );
    }

    #[test]
    fn a_result_folder_is_fingerprinted_by_its_files() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("doc-pages");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("a.pdf"), b"abc").unwrap();
        fs::write(dir.join("b.pdf"), b"de").unwrap();
        let before = stat(&dir).unwrap();
        assert_eq!(before.0, 5);
        fs::write(dir.join("c.pdf"), b"f").unwrap();
        assert_eq!(stat(&dir).unwrap().0, 6);
        assert!(stat(&d.path().join("gone")).is_none());
    }

    #[test]
    fn undo_recycles_only_the_recorded_output() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("undo-test-out.png");
        let other = d.path().join("undo-test-other.png");
        fs::write(&out, b"abc").unwrap();
        fs::write(&other, b"keep").unwrap();
        let store: Shared = Arc::new(Mutex::new(Store::load(d.path().join("h.json"), true)));
        let id = store.lock().unwrap().record(done(&out)).unwrap();
        undo(&store, id, Lang::En).unwrap();
        assert!(!out.exists() && other.exists());
        assert!(store.lock().unwrap().get(id).unwrap().undone);
        // a second undo is refused, as is an unknown id
        assert!(undo(&store, id, Lang::En).is_err());
        assert!(undo(&store, 999, Lang::En).is_err());
        // a changed output is left alone
        fs::write(&out, b"abc").unwrap();
        let id = store.lock().unwrap().record(done(&out)).unwrap();
        fs::write(&out, b"edited later").unwrap();
        assert!(undo(&store, id, Lang::En).is_err() && out.exists());
    }

    #[test]
    fn undo_restores_the_recycled_original() {
        let d = tempfile::tempdir().unwrap();
        let orig = d.path().join("undo-test-original-qz.txt");
        let out = d.path().join("undo-test-output-qz.txt");
        fs::write(&orig, b"original").unwrap();
        fs::write(&out, b"converted").unwrap();
        let started = now_ms();
        if trash::delete(&orig).is_err() {
            return; // no Recycle Bin here
        }
        let store: Shared = Arc::new(Mutex::new(Store::load(d.path().join("h.json"), true)));
        let mut e = done(&out);
        e.input = orig.display().to_string();
        e.started = started;
        e.replaced_original = true;
        e.original_recycled = true;
        let id = store.lock().unwrap().record(e).unwrap();
        match undo(&store, id, Lang::En) {
            Ok(()) => {
                assert_eq!(fs::read(&orig).unwrap(), b"original");
                assert!(!out.exists());
            }
            // the bin may be unreachable (policy, CI); then nothing must have been deleted
            Err(msg) => assert!(out.exists() && !msg.is_empty()),
        }
    }
}
