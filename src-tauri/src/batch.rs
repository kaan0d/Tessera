//! Explorer starts one process per selected file for classic verbs, so a merge arrives as N
//! separate `--merge <kind> <file>` calls. `Collector` gathers them: once no file has
//! arrived for `quiet_ms`, the batch is released as one sorted, de-duplicated list.
//! Time is passed in (milliseconds), so tests use a fake clock.

use std::cmp::Ordering;
use std::path::PathBuf;

use crate::convert::combine::MergeKind;

pub const QUIET_MS: u64 = 700;

struct Batch {
    kind: MergeKind,
    files: Vec<PathBuf>,
    last: u64,
}

pub struct Collector {
    quiet_ms: u64,
    batches: Vec<Batch>,
}

impl Collector {
    pub fn new(quiet_ms: u64) -> Collector {
        Collector {
            quiet_ms,
            batches: vec![],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }

    /// Adds files at time `now`. True when the collector was empty before, i.e. the caller
    /// has to start a flusher.
    pub fn push(&mut self, kind: MergeKind, files: Vec<PathBuf>, now: u64) -> bool {
        let was_empty = self.batches.is_empty();
        let i = match self.batches.iter().position(|b| b.kind == kind) {
            Some(i) => i,
            None => {
                self.batches.push(Batch {
                    kind,
                    files: vec![],
                    last: now,
                });
                self.batches.len() - 1
            }
        };
        let b = &mut self.batches[i];
        b.last = now;
        b.files.extend(files);
        was_empty
    }

    /// Batches that have been quiet long enough, removed from the collector.
    pub fn take_ready(&mut self, now: u64) -> Vec<(MergeKind, Vec<PathBuf>)> {
        let quiet = self.quiet_ms;
        let (ready, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.batches)
            .into_iter()
            .partition(|b| now.saturating_sub(b.last) >= quiet);
        self.batches = keep;
        ready
            .into_iter()
            .map(|b| (b.kind, order(b.files)))
            .collect()
    }
}

/// Natural order by file name ("img2" before "img10"), de-duplicated (case-insensitive).
fn order(mut files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    files.retain(|f| seen.insert(f.to_string_lossy().to_lowercase()));
    files.sort_by(|a, b| {
        let name = |p: &PathBuf| {
            p.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().to_lowercase())
        };
        natural(&name(a), &name(b)).then_with(|| a.cmp(b))
    });
    files
}

fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let num = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.next_if(char::is_ascii_digit) {
                        s.push(c);
                    }
                    let t = s.trim_start_matches('0').to_string();
                    (t.len(), t)
                };
                let (na, nb) = (num(&mut a), num(&mut b));
                if na != nb {
                    return na.cmp(&nb);
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn waits_for_a_quiet_window_then_sorts_and_dedups() {
        let mut c = Collector::new(QUIET_MS);
        assert!(c.push(MergeKind::Pdf, vec![p(r"C:\a\img10.png")], 1000));
        assert!(!c.push(MergeKind::Pdf, vec![p(r"C:\a\img2.png")], 1300));
        assert!(!c.push(
            MergeKind::Pdf,
            vec![p(r"C:\a\IMG2.png"), p(r"C:\a\img1.png")],
            1900
        ));
        // 699 ms after the last arrival: still collecting
        assert!(c.take_ready(2599).is_empty());
        let r = c.take_ready(2600);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, MergeKind::Pdf);
        assert_eq!(
            r[0].1,
            [
                p(r"C:\a\img1.png"),
                p(r"C:\a\img2.png"),
                p(r"C:\a\img10.png")
            ]
        );
        assert!(c.is_empty());
        // a new push after a flush asks for a flusher again
        assert!(c.push(MergeKind::Audio, vec![p("x.mp3")], 5000));
    }

    #[test]
    fn kinds_are_separate_and_a_late_file_extends_the_window() {
        let mut c = Collector::new(100);
        c.push(MergeKind::Pdf, vec![p("a.png")], 0);
        c.push(MergeKind::Audio, vec![p("b.mp3")], 50);
        let r = c.take_ready(100);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, MergeKind::Pdf);
        c.push(MergeKind::Audio, vec![p("c.mp3")], 120);
        assert!(c.take_ready(200).is_empty());
        let r = c.take_ready(220);
        assert_eq!(r[0].1, [p("b.mp3"), p("c.mp3")]);
    }

    #[test]
    fn natural_order() {
        assert_eq!(natural("a2", "a10"), Ordering::Less);
        assert_eq!(natural("a02", "a2"), Ordering::Equal);
        assert_eq!(natural("b", "a10"), Ordering::Greater);
        assert_eq!(natural("a", "a1"), Ordering::Less);
    }
}
