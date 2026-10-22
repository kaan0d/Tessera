//! "Convert all images in this folder": the Explorer folder verbs run `--folder <spec> <dir>`.
//! `scan` picks the files (pure, tested); `lib.rs` turns the list into normal queue jobs.

use std::collections::HashSet;
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::convert::Format;
use crate::watch::ignored_name;

/// Files taken from one folder; more than this are left alone (with a notification).
pub const MAX_FILES: usize = 2000;
/// Deepest sub-folder level read when the folder verb is recursive.
const MAX_DEPTH: usize = 8;
const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;
const REPARSE_POINT: u32 = 0x400;

/// What a folder verb does to every matching file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `--folder jpg <dir>`
    Target(Format),
    /// `--folder preset:web <dir>`
    Preset(String),
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        match s.strip_prefix("preset:") {
            Some(id) if !id.is_empty() => Some(Action::Preset(id.to_string())),
            Some(_) => None,
            None => Format::from_ext(s)
                .filter(|f| Format::ALL.contains(f) && f.ext() == s)
                .map(Action::Target),
        }
    }

    /// Command-line spelling, the inverse of `parse`.
    pub fn arg(&self) -> String {
        match self {
            Action::Target(t) => t.ext().to_string(),
            Action::Preset(id) => format!("preset:{id}"),
        }
    }
}

/// A source belongs to a folder verb for `target` when it is the same family: images for
/// image targets, audio for audio targets (a video is not "audio in this folder").
pub fn same_family(src: Format, target: Format) -> bool {
    src.kind() == target.kind() || (target == Format::Pdf && src.is_image())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub files: Vec<PathBuf>,
    /// More matches existed than `cap`.
    pub truncated: bool,
}

/// Files of `dir` (and sub-folders when `recursive`) that `want` accepts, sorted, at most
/// `cap`. Skipped: hidden/system entries, temp-looking names, links to other folders and
/// anything `own` says we wrote ourselves.
pub fn scan(
    dir: &Path,
    recursive: bool,
    cap: usize,
    want: impl Fn(&Path) -> bool,
    own: impl Fn(&Path) -> bool,
) -> Scan {
    let mut found = vec![];
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let Ok(m) = e.metadata() else { continue };
            let name = e.file_name();
            if m.file_attributes() & HIDDEN_OR_SYSTEM != 0 || ignored_name(&name.to_string_lossy())
            {
                continue;
            }
            if m.is_dir() {
                if recursive && depth < MAX_DEPTH && m.file_attributes() & REPARSE_POINT == 0 {
                    stack.push((path, depth + 1));
                }
            } else if m.is_file()
                && m.file_attributes() & REPARSE_POINT == 0
                && want(&path)
                && !own(&path)
            {
                found.push(path);
            }
        }
    }
    found.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    let truncated = found.len() > cap;
    found.truncate(cap);
    Scan {
        files: found,
        truncated,
    }
}

/// Lowercased paths of every output the history knows (successful, not undone).
pub fn history_outputs(entries: &[crate::history::Entry]) -> HashSet<String> {
    entries
        .iter()
        .filter(|e| e.ok && !e.undone)
        .filter_map(|e| e.output.as_ref())
        .map(|o| o.replace('/', "\\").to_lowercase())
        .collect()
}

pub fn is_listed(outputs: &HashSet<String>, p: &Path) -> bool {
    outputs.contains(&p.to_string_lossy().replace('/', "\\").to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }

    fn images(p: &Path) -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .and_then(Format::from_ext)
            .is_some_and(|f| f.is_image())
    }

    fn names(s: &Scan) -> Vec<String> {
        s.files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn parses_actions() {
        assert_eq!(Action::parse("jpg"), Some(Action::Target(Format::Jpg)));
        assert_eq!(
            Action::parse("preset:web"),
            Some(Action::Preset("web".into()))
        );
        for bad in ["", "preset:", "jpeg", "heic", "exe", "JPG"] {
            assert_eq!(Action::parse(bad), None, "{bad}");
        }
        for a in [Action::Target(Format::Flac), Action::Preset("c1".into())] {
            assert_eq!(Action::parse(&a.arg()), Some(a));
        }
    }

    #[test]
    fn families() {
        assert!(same_family(Format::Png, Format::Jpg));
        assert!(same_family(Format::Heic, Format::Jpg));
        assert!(same_family(Format::Jpg, Format::Pdf));
        assert!(same_family(Format::Mp3, Format::Wav));
        assert!(!same_family(Format::Mp4, Format::Wav));
        assert!(!same_family(Format::Pdf, Format::Png));
        assert!(!same_family(Format::Mp4, Format::Gif));
    }

    #[test]
    fn scan_picks_sorted_supported_visible_files() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        for n in [
            "b.PNG",
            "a.jpg",
            "c.txt",
            ".hidden.png",
            "d.png.tmp",
            "~$e.png",
        ] {
            touch(&r.join(n));
        }
        touch(&r.join("sub").join("deep.png"));
        let s = scan(r, false, 100, images, |_| false);
        assert_eq!(names(&s), ["a.jpg", "b.PNG"]);
        assert!(!s.truncated);
        let s = scan(r, true, 100, images, |_| false);
        assert_eq!(names(&s), ["a.jpg", "b.PNG", "deep.png"]);
    }

    #[test]
    fn scan_skips_own_outputs_system_files_and_caps() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        for i in 0..5 {
            touch(&r.join(format!("p{i}.png")));
        }
        let own = |p: &Path| p.ends_with("p1.png");
        let s = scan(r, false, 3, images, own);
        assert_eq!(names(&s), ["p0.png", "p2.png", "p3.png"]);
        assert!(s.truncated);
        let s = scan(r, false, 4, images, own);
        assert!(!s.truncated && s.files.len() == 4);
        // system attribute (0x4) via attrib
        std::process::Command::new("attrib")
            .arg("+S")
            .arg(r.join("p0.png"))
            .output()
            .unwrap();
        let s = scan(r, false, 10, images, |_| false);
        assert_eq!(names(&s), ["p1.png", "p2.png", "p3.png", "p4.png"]);
    }

    #[test]
    fn history_outputs_match_case_insensitively() {
        let e = |out: &str, ok, undone| crate::history::Entry {
            output: Some(out.into()),
            ok,
            undone,
            ..Default::default()
        };
        let set = history_outputs(&[
            e(r"C:\A\Out.jpg", true, false),
            e(r"C:\A\fail.jpg", false, false),
            e(r"C:\A\undone.jpg", true, true),
        ]);
        assert!(is_listed(&set, Path::new("c:/a/OUT.JPG")));
        assert!(!is_listed(&set, Path::new(r"C:\A\fail.jpg")));
        assert!(!is_listed(&set, Path::new(r"C:\A\undone.jpg")));
    }
}
