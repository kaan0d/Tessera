use std::path::{Path, PathBuf};

use crate::convert::combine::MergeKind;
use crate::convert::Format;

#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    InstallMenu,
    RemoveMenu,
    Convert {
        target: Format,
        files: Vec<PathBuf>,
    },
    Preset {
        id: String,
        files: Vec<PathBuf>,
    },
    Merge {
        kind: MergeKind,
        files: Vec<PathBuf>,
    },
    Background,
    Normal,
}

/// `args` excludes the executable name. Unknown or malformed input falls back to `Normal`.
/// Relative file paths are resolved against `cwd` (a forwarded launch has another cwd).
pub fn parse(args: &[String], cwd: Option<&Path>) -> Cli {
    match args.first().map(String::as_str) {
        Some("--install-menu") => Cli::InstallMenu,
        Some("--remove-menu") => Cli::RemoveMenu,
        Some("--background") => Cli::Background,
        Some("--convert") => {
            let Some(target) = args.get(1).and_then(|t| Format::from_ext(t)) else {
                return Cli::Normal;
            };
            match files(&args[2..], cwd) {
                Some(files) => Cli::Convert { target, files },
                None => Cli::Normal,
            }
        }
        Some("--preset") => {
            let Some(id) = args.get(1).filter(|i| !i.is_empty()) else {
                return Cli::Normal;
            };
            match files(&args[2..], cwd) {
                Some(files) => Cli::Preset {
                    id: id.clone(),
                    files,
                },
                None => Cli::Normal,
            }
        }
        Some("--merge") => {
            let Some(kind) = args.get(1).and_then(|k| MergeKind::from_arg(k)) else {
                return Cli::Normal;
            };
            match files(&args[2..], cwd) {
                Some(files) => Cli::Merge { kind, files },
                None => Cli::Normal,
            }
        }
        _ => Cli::Normal,
    }
}

/// Paths from the command line; None when there are none.
fn files(args: &[String], cwd: Option<&Path>) -> Option<Vec<PathBuf>> {
    let files: Vec<PathBuf> = args
        .iter()
        .map(|f| match cwd {
            Some(c) => c.join(f),
            None => PathBuf::from(f),
        })
        .collect();
    (!files.is_empty()).then_some(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_flags() {
        assert_eq!(parse(&a(&[]), None), Cli::Normal);
        assert_eq!(parse(&a(&["--install-menu"]), None), Cli::InstallMenu);
        assert_eq!(parse(&a(&["--remove-menu"]), None), Cli::RemoveMenu);
        assert_eq!(parse(&a(&["--background"]), None), Cli::Background);
        assert_eq!(parse(&a(&["--bogus"]), None), Cli::Normal);
    }

    #[test]
    fn parses_convert() {
        assert_eq!(
            parse(&a(&["--convert", "jpg", r"C:\a b.png", r"C:\c.png"]), None),
            Cli::Convert {
                target: Format::Jpg,
                files: vec![PathBuf::from(r"C:\a b.png"), PathBuf::from(r"C:\c.png")]
            }
        );
        assert_eq!(parse(&a(&["--convert", "jpg"]), None), Cli::Normal);
        assert_eq!(parse(&a(&["--convert", "exe", "x"]), None), Cli::Normal);
        let Cli::Convert { files, .. } =
            parse(&a(&["--convert", "png", "x.jpg"]), Some(Path::new(r"C:\w")))
        else {
            panic!()
        };
        assert_eq!(files, vec![PathBuf::from(r"C:\w\x.jpg")]);
    }

    #[test]
    fn parses_preset_and_merge() {
        assert_eq!(
            parse(&a(&["--preset", "whatsapp", r"C:\a.png"]), None),
            Cli::Preset {
                id: "whatsapp".into(),
                files: vec![PathBuf::from(r"C:\a.png")]
            }
        );
        assert_eq!(parse(&a(&["--preset", "web"]), None), Cli::Normal);
        assert_eq!(parse(&a(&["--preset"]), None), Cli::Normal);
        assert_eq!(
            parse(&a(&["--merge", "pdf", "a.png"]), Some(Path::new(r"C:\w"))),
            Cli::Merge {
                kind: MergeKind::Pdf,
                files: vec![PathBuf::from(r"C:\w\a.png")]
            }
        );
        assert_eq!(
            parse(&a(&["--merge", "audio", "a.mp3", "b.mp3"]), None),
            Cli::Merge {
                kind: MergeKind::Audio,
                files: vec![PathBuf::from("a.mp3"), PathBuf::from("b.mp3")]
            }
        );
        assert_eq!(parse(&a(&["--merge", "zip", "a"]), None), Cli::Normal);
        assert_eq!(parse(&a(&["--merge", "pdf"]), None), Cli::Normal);
    }
}
