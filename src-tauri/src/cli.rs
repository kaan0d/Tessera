use std::path::{Path, PathBuf};

use crate::convert::combine::MergeKind;
use crate::convert::{Conflict, Format, Resize};
use crate::folder::Action;

#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    InstallMenu,
    RemoveMenu,
    /// Show the installer window even when the exe looks installed.
    Setup,
    /// Silent install (`--install <dir> [--portable]`), no window.
    Install {
        dir: String,
        portable: bool,
    },
    /// Remove the installed app (menu, shortcuts, uninstall entry, files). Shows a window
    /// unless `--quiet`; `--keep-data` leaves settings and history.
    Uninstall {
        quiet: bool,
        keep_data: bool,
    },
    InstallSendTo,
    RemoveSendTo,
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
    /// "Custom settings" verb: the chooser opens straight on its custom page.
    Custom {
        files: Vec<PathBuf>,
    },
    /// Folder verb: convert the matching files of a folder.
    Folder {
        action: Action,
        dir: PathBuf,
    },
    /// Bare file paths (dropped on the exe, Open with, Send To): ask what to do with them.
    Drop {
        files: Vec<PathBuf>,
    },
    Background,
    /// A headless subcommand (`convert`, `preset`, ...); never starts the app.
    Cmd(Command),
    Normal,
}

/// Arguments of `tessera convert`.
#[derive(Debug, PartialEq, Eq)]
pub struct ConvertArgs {
    pub target: Format,
    pub files: Vec<PathBuf>,
    pub resize: Option<Resize>,
    pub strip: bool,
    pub max_bytes: Option<u64>,
    pub quality: Option<u8>,
    pub out: Option<PathBuf>,
    pub conflict: Conflict,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    ListPresets,
    Formats(String),
    Convert(ConvertArgs),
    Preset {
        id: String,
        files: Vec<PathBuf>,
    },
    /// Bad command line (exit code 2); the text says what is wrong.
    Usage(String),
}

/// "8MB", "500KB", "1.5 GB", "1024" (bytes). KB/MB/GB are decimal like the presets,
/// KiB/MiB/GiB binary. Zero and junk are None.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let mult: f64 = match unit.trim() {
        "" | "b" => 1.0,
        "kb" | "k" => 1e3,
        "mb" | "m" => 1e6,
        "gb" | "g" => 1e9,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    let n: f64 = num.parse().ok()?;
    let bytes = (n * mult).round();
    (1.0..=1e13).contains(&bytes).then_some(bytes as u64)
}

/// `args` excludes the executable name. Unknown or malformed input falls back to `Normal`.
/// Relative file paths are resolved against `cwd` (a forwarded launch has another cwd).
pub fn parse(args: &[String], cwd: Option<&Path>) -> Cli {
    match args.first().map(String::as_str) {
        Some("--install-menu") => Cli::InstallMenu,
        Some("--remove-menu") => Cli::RemoveMenu,
        Some("--setup") => Cli::Setup,
        Some("--install") => match args.get(1) {
            Some(dir) if !dir.starts_with("--") => Cli::Install {
                dir: dir.clone(),
                portable: args.iter().any(|a| a == "--portable"),
            },
            _ => Cli::Normal,
        },
        Some("--uninstall") => Cli::Uninstall {
            quiet: args.iter().any(|a| a == "--quiet"),
            keep_data: args.iter().any(|a| a == "--keep-data"),
        },
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
        Some("--custom") => match files(&args[1..], cwd) {
            Some(files) => Cli::Custom { files },
            None => Cli::Normal,
        },
        Some("--folder") => {
            let Some(action) = args.get(1).and_then(|a| Action::parse(a)) else {
                return Cli::Normal;
            };
            match (args.get(2), args.len()) {
                (Some(d), 3) => Cli::Folder {
                    action,
                    dir: match cwd {
                        Some(c) => c.join(d),
                        None => PathBuf::from(d),
                    },
                },
                _ => Cli::Normal,
            }
        }
        Some("convert") => Cli::Cmd(parse_convert(&args[1..])),
        Some("preset") => Cli::Cmd(match (args.get(1), &args[args.len().min(2)..]) {
            (Some(id), rest) if !rest.is_empty() && !rest.iter().any(|a| a.starts_with("--")) => {
                Command::Preset {
                    id: id.clone(),
                    files: rest.iter().map(PathBuf::from).collect(),
                }
            }
            _ => usage("preset needs an id and at least one file"),
        }),
        Some("list-presets") => Cli::Cmd(Command::ListPresets),
        Some("formats") => Cli::Cmd(match args.get(1) {
            Some(e) if args.len() == 2 => Command::Formats(e.clone()),
            _ => usage("formats needs one extension"),
        }),
        Some("--version" | "-V") => Cli::Cmd(Command::Version),
        Some("--help" | "-h" | "help") => Cli::Cmd(Command::Help),
        Some("--install-sendto") => Cli::InstallSendTo,
        Some("--remove-sendto") => Cli::RemoveSendTo,
        Some(a) if !a.starts_with('-') => match files(args, cwd) {
            Some(files) => Cli::Drop { files },
            None => Cli::Normal,
        },
        _ => Cli::Normal,
    }
}

fn usage(msg: &str) -> Command {
    Command::Usage(msg.into())
}

fn parse_convert(args: &[String]) -> Command {
    match convert_args(args) {
        Ok(c) => Command::Convert(c),
        Err(e) => Command::Usage(e),
    }
}

fn convert_args(args: &[String]) -> Result<ConvertArgs, String> {
    let (mut files, mut to, mut resize, mut max_bytes) = (vec![], None, None, None);
    let (mut strip, mut quality, mut out) = (false, None, None);
    let mut conflict = Conflict::AutoRename;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let Some(flag) = a.strip_prefix("--") else {
            files.push(PathBuf::from(a));
            continue;
        };
        let (name, inline) = match flag.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (flag, None),
        };
        if name == "strip" && inline.is_none() {
            strip = true;
            continue;
        }
        let v = inline
            .or_else(|| it.next().cloned())
            .ok_or_else(|| format!("--{name} needs a value"))?;
        let num = |min: u32, max: u32| {
            v.parse::<u32>()
                .ok()
                .filter(|n| (min..=max).contains(n))
                .ok_or_else(|| format!("--{name} must be a number from {min} to {max}"))
        };
        let mut set_resize = |r: Resize| match resize.replace(r) {
            None => Ok(()),
            Some(_) => Err("use only one of --max-edge and --percent".to_string()),
        };
        match name {
            "to" => to = Some(v.clone()),
            "max-edge" => set_resize(Resize::MaxEdge(num(1, 16384)?))?,
            "percent" => set_resize(Resize::Percent(num(1, 400)?))?,
            "max-size" => {
                max_bytes = Some(parse_size(&v).ok_or("--max-size must look like 8MB or 500KB")?)
            }
            "quality" => quality = Some(num(1, 100)? as u8),
            "out" => out = Some(PathBuf::from(&v)),
            "conflict" => {
                conflict = match v.as_str() {
                    "rename" => Conflict::AutoRename,
                    "overwrite" => Conflict::Overwrite,
                    _ => return Err("--conflict must be rename or overwrite".into()),
                }
            }
            _ => return Err(format!("unknown option --{name}")),
        }
    }
    let to = to.ok_or("convert needs --to <ext>")?;
    let target = Format::from_ext(to.trim_start_matches('.'))
        .filter(|f| Format::ALL.contains(f))
        .ok_or_else(|| format!("unknown target format: {to}"))?;
    if files.is_empty() {
        return Err("convert needs at least one file".into());
    }
    Ok(ConvertArgs {
        target,
        files,
        resize,
        strip,
        max_bytes,
        quality,
        out,
        conflict,
    })
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
        assert_eq!(parse(&a(&["--setup"]), None), Cli::Setup);
        assert_eq!(
            parse(&a(&["--install", r"D:\Apps", "--portable"]), None),
            Cli::Install {
                dir: r"D:\Apps".into(),
                portable: true
            }
        );
        assert_eq!(parse(&a(&["--install"]), None), Cli::Normal);
        assert_eq!(
            parse(&a(&["--uninstall"]), None),
            Cli::Uninstall {
                quiet: false,
                keep_data: false
            }
        );
        assert_eq!(
            parse(&a(&["--uninstall", "--quiet", "--keep-data"]), None),
            Cli::Uninstall {
                quiet: true,
                keep_data: true
            }
        );
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
    fn bare_paths_are_a_drop() {
        assert_eq!(
            parse(&a(&[r"C:\a b.png", r"C:\c.mp3"]), None),
            Cli::Drop {
                files: vec![PathBuf::from(r"C:\a b.png"), PathBuf::from(r"C:\c.mp3")]
            }
        );
        assert_eq!(
            parse(&a(&["x.png"]), Some(Path::new(r"C:\w"))),
            Cli::Drop {
                files: vec![PathBuf::from(r"C:\w\x.png")]
            }
        );
        assert_eq!(parse(&a(&["--wat", "x.png"]), None), Cli::Normal);
        assert_eq!(parse(&a(&["--install-sendto"]), None), Cli::InstallSendTo);
        assert_eq!(parse(&a(&["--remove-sendto"]), None), Cli::RemoveSendTo);
        // subcommands and flags still win
        assert!(matches!(
            parse(&a(&["convert", "--to", "png", "a.jpg"]), None),
            Cli::Cmd(_)
        ));
        assert_eq!(parse(&[], None), Cli::Normal);
    }

    #[test]
    fn parses_folder() {
        assert_eq!(
            parse(&a(&["--folder", "jpg", r"C:\pics"]), None),
            Cli::Folder {
                action: Action::Target(Format::Jpg),
                dir: PathBuf::from(r"C:\pics")
            }
        );
        assert_eq!(
            parse(
                &a(&["--folder", "preset:web", "pics"]),
                Some(Path::new(r"C:\w"))
            ),
            Cli::Folder {
                action: Action::Preset("web".into()),
                dir: PathBuf::from(r"C:\w\pics")
            }
        );
        for bad in [
            vec!["--folder"],
            vec!["--folder", "jpg"],
            vec!["--folder", "exe", "x"],
            vec!["--folder", "jpg", "a", "b"],
        ] {
            assert_eq!(parse(&a(&bad), None), Cli::Normal, "{bad:?}");
        }
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
        assert_eq!(
            parse(&a(&["--custom", "a.png"]), Some(Path::new(r"C:\w"))),
            Cli::Custom {
                files: vec![PathBuf::from(r"C:\w\a.png")]
            }
        );
        assert_eq!(parse(&a(&["--custom"]), None), Cli::Normal);
        assert_eq!(parse(&a(&["--merge", "zip", "a"]), None), Cli::Normal);
        assert_eq!(parse(&a(&["--merge", "pdf"]), None), Cli::Normal);
    }

    fn cmd(v: &[&str]) -> Command {
        match parse(&a(v), None) {
            Cli::Cmd(c) => c,
            other => panic!("{other:?}"),
        }
    }

    fn conv(v: &[&str]) -> ConvertArgs {
        match cmd(v) {
            Command::Convert(c) => c,
            other => panic!("{other:?}"),
        }
    }

    fn usage_err(v: &[&str]) -> String {
        match cmd(v) {
            Command::Usage(m) => m,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn simple_subcommands() {
        assert_eq!(cmd(&["--version"]), Command::Version);
        assert_eq!(cmd(&["-V"]), Command::Version);
        assert_eq!(cmd(&["--help"]), Command::Help);
        assert_eq!(cmd(&["-h"]), Command::Help);
        assert_eq!(cmd(&["list-presets"]), Command::ListPresets);
        assert_eq!(cmd(&["formats", "png"]), Command::Formats("png".into()));
        usage_err(&["formats"]);
        usage_err(&["formats", "a", "b"]);
    }

    #[test]
    fn preset_subcommand() {
        assert_eq!(
            cmd(&["preset", "web", "a.png", "b.png"]),
            Command::Preset {
                id: "web".into(),
                files: vec!["a.png".into(), "b.png".into()]
            }
        );
        usage_err(&["preset"]);
        usage_err(&["preset", "web"]);
        usage_err(&["preset", "web", "--x", "a.png"]);
    }

    #[test]
    fn convert_options() {
        let c = conv(&["convert", "a.png", "b.png", "--to", "JPG"]);
        assert_eq!(c.target, Format::Jpg);
        assert_eq!(c.files.len(), 2);
        assert!(!c.strip && c.resize.is_none() && c.conflict == Conflict::AutoRename);
        let c = conv(&[
            "convert",
            "--to=webp",
            "x.png",
            "--max-edge",
            "1280",
            "--strip",
            "--max-size",
            "8MB",
            "--quality",
            "70",
            "--out",
            r"C:\o",
            "--conflict",
            "overwrite",
        ]);
        assert_eq!(c.target, Format::Webp);
        assert_eq!(c.resize, Some(Resize::MaxEdge(1280)));
        assert!(c.strip);
        assert_eq!(c.max_bytes, Some(8_000_000));
        assert_eq!(c.quality, Some(70));
        assert_eq!(c.out, Some(PathBuf::from(r"C:\o")));
        assert_eq!(c.conflict, Conflict::Overwrite);
        let c = conv(&["convert", "x.png", "--percent", "50", "--to", ".png"]);
        assert_eq!(c.resize, Some(Resize::Percent(50)));
    }

    #[test]
    fn convert_usage_errors() {
        let base = ["convert", "a.png", "--to", "jpg"];
        let with = |extra: &[&'static str]| -> Vec<&'static str> {
            base.iter().chain(extra).copied().collect()
        };
        for bad in [
            vec!["convert"],
            vec!["convert", "a.png"],
            vec!["convert", "--to", "png"],
            vec!["convert", "a.png", "--to", "exe"],
            vec!["convert", "a.png", "--to", "heic"],
            vec!["convert", "a.png", "--to"],
            with(&["--max-edge", "0"]),
            with(&["--max-edge", "x"]),
            with(&["--percent", "401"]),
            with(&["--max-edge", "9", "--percent", "9"]),
            with(&["--quality", "101"]),
            with(&["--max-size", "big"]),
            with(&["--conflict", "ask"]),
            with(&["--bogus", "1"]),
        ] {
            assert!(!usage_err(&bad).is_empty(), "{bad:?}");
        }
    }

    #[test]
    fn sizes() {
        for (s, n) in [
            ("8MB", 8_000_000),
            ("500KB", 500_000),
            ("500kb", 500_000),
            ("1.5 MB", 1_500_000),
            ("2GB", 2_000_000_000),
            ("1KiB", 1024),
            ("3MiB", 3 * 1024 * 1024),
            ("1024", 1024),
            ("10B", 10),
            (" 8 mb ", 8_000_000),
        ] {
            assert_eq!(parse_size(s), Some(n), "{s}");
        }
        for s in [
            "",
            "MB",
            "0",
            "0MB",
            "-1MB",
            "8XB",
            "1.2.3MB",
            "abc",
            "99999999TB",
        ] {
            assert_eq!(parse_size(s), None, "{s}");
        }
    }
}
