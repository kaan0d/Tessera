use std::path::{Path, PathBuf};

use crate::convert::Format;

#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    InstallMenu,
    RemoveMenu,
    Convert { target: Format, files: Vec<PathBuf> },
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
            let files: Vec<PathBuf> = args[2..]
                .iter()
                .map(|f| match cwd {
                    Some(c) => c.join(f),
                    None => PathBuf::from(f),
                })
                .collect();
            if files.is_empty() {
                Cli::Normal
            } else {
                Cli::Convert { target, files }
            }
        }
        _ => Cli::Normal,
    }
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
}
