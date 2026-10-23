//! Opt-in update check: one GET to the GitHub Releases API, nothing is downloaded or installed.

use std::io::Read;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::i18n::{fmt, t, Lang};

/// The only place that names the repository.
pub const REPO: &str = "kaan0d/Tessera";
const MAX_BODY: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub current: String,
    pub latest: String,
    /// Release page (or the repository's releases page when none exists).
    pub url: String,
    /// Localized one-liner for the UI ("You are up to date ...").
    pub message: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    #[serde(default)]
    browser_download_url: String,
}

fn api_url(slug: &str) -> String {
    format!("https://api.github.com/repos/{slug}/releases/latest")
}

fn releases_page(slug: &str) -> String {
    format!("https://github.com/{slug}/releases")
}

/// `v1.2.3` or `1.2.3` (pre-release and build parts allowed).
pub fn parse_version(s: &str) -> Option<semver::Version> {
    semver::Version::parse(s.trim().trim_start_matches(['v', 'V'])).ok()
}

/// True when `latest` is a newer version than `current`; unparsable input is never newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// Turns the API's JSON into the command result. `slug` decides which urls are trusted.
pub fn parse_release(
    body: &str,
    current: &str,
    slug: &str,
    lang: Lang,
) -> Result<UpdateInfo, String> {
    let bad = || t(lang, "update.bad_response").to_string();
    let r: Release = serde_json::from_str(body).map_err(|_| bad())?;
    parse_version(&r.tag_name).ok_or_else(bad)?;
    let latest = r.tag_name.trim().trim_start_matches(['v', 'V']).to_string();
    let available = is_newer(&latest, current);
    // never hand the UI a link that open_url would refuse
    let url = if allowed_url(&r.html_url, slug) {
        r.html_url
    } else {
        releases_page(slug)
    };
    let message = if available {
        fmt(
            lang,
            "update.available",
            &[("latest", &latest), ("current", current)],
        )
    } else {
        fmt(lang, "update.up_to_date", &[("current", current)])
    };
    Ok(UpdateInfo {
        available,
        current: current.into(),
        latest,
        url,
        message,
    })
}

/// 404 means no release exists yet: up to date.
pub fn no_releases(current: &str, slug: &str, lang: Lang) -> UpdateInfo {
    UpdateInfo {
        available: false,
        current: current.into(),
        latest: current.into(),
        url: releases_page(slug),
        message: t(lang, "update.none_yet").into(),
    }
}

/// GET `url` as text (at most `MAX_BODY` bytes); `None` for a 404.
fn get_text(url: &str, lang: Lang) -> Result<Option<String>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .user_agent(format!(
            "{}/{}",
            crate::app_info::APP_NAME,
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .into();
    let fail = |e: &dyn std::fmt::Display| fmt(lang, "update.failed", &[("err", &e.to_string())]);
    let mut resp = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| fail(&e))?;
    match resp.status().as_u16() {
        200 => {}
        404 => return Ok(None),
        s => return Err(fail(&format!("HTTP {s}"))),
    }
    let mut body = String::new();
    resp.body_mut()
        .with_config()
        .limit(MAX_BODY)
        .reader()
        .take(MAX_BODY)
        .read_to_string(&mut body)
        .map_err(|e| fail(&e))?;
    Ok(Some(body))
}

/// Queries the latest release of `REPO`. Blocking; run it off the UI thread.
pub fn check(lang: Lang) -> Result<UpdateInfo, String> {
    let current = env!("CARGO_PKG_VERSION");
    match get_text(&api_url(REPO), lang)? {
        Some(body) => parse_release(&body, current, REPO, lang),
        None => Ok(no_releases(current, REPO, lang)),
    }
}

/// Download links of `tessera-setup-v<latest>.exe` and its `.sha256` in a release answer.
/// Both must be release downloads of this repository.
fn installer_urls(body: &str, latest: &str, slug: &str) -> Option<(String, String)> {
    let r: Release = serde_json::from_str(body).ok()?;
    let find = |name: String| {
        r.assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
            .filter(|u| allowed_url(u, slug) && u.contains("/releases/download/"))
    };
    let exe = format!("tessera-setup-v{latest}.exe");
    Some((find(exe.clone())?, find(exe + ".sha256")?))
}

/// The hash in a `sha256sum` line (`<hex> *<name>`).
fn parse_sha256(text: &str) -> Option<String> {
    let h = text.split_whitespace().next()?;
    (h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| h.to_lowercase())
}

/// In-app update: downloads the latest installer, checks its SHA-256 and runs it as a silent
/// update of this install. Only works from the installed copy. Blocking.
pub fn install(lang: Lang) -> Result<(), String> {
    let text = |k: &str| t(lang, k).to_string();
    let dir = crate::install::installed_here().ok_or_else(|| text("update.not_installed"))?;
    let body = get_text(&api_url(REPO), lang)?.ok_or_else(|| text("update.none_yet"))?;
    let info = parse_release(&body, env!("CARGO_PKG_VERSION"), REPO, lang)?;
    if !info.available {
        return Err(info.message);
    }
    let (exe_url, sha_url) =
        installer_urls(&body, &info.latest, REPO).ok_or_else(|| text("update.no_asset"))?;
    let sha = get_text(&sha_url, lang)?
        .and_then(|s| parse_sha256(&s))
        .ok_or_else(|| text("update.no_asset"))?;
    let work = std::env::temp_dir().join("Tessera-update");
    let _ = std::fs::remove_dir_all(&work);
    let fail = |e: &dyn std::fmt::Display| fmt(lang, "update.failed", &[("err", &e.to_string())]);
    std::fs::create_dir_all(&work).map_err(|e| fail(&e))?;
    let setup = work.join(format!("tessera-setup-v{}.exe", info.latest));
    crate::ffmpeg::download(&exe_url, &setup, &mut |_| {}).map_err(|e| e.message(lang))?;
    crate::ffmpeg::verify_sha256(&setup, &sha).map_err(|e| e.message(lang))?;
    crate::install::launch_update(&setup, &dir).map_err(|e| fail(&e))
}

/// Only https links inside the project's own GitHub repository.
pub fn allowed_url(url: &str, slug: &str) -> bool {
    let prefix = format!("https://github.com/{slug}");
    let Some(rest) = url.strip_prefix(&prefix) else {
        return false;
    };
    (rest.is_empty() || rest.starts_with(['/', '?', '#']))
        && !url.chars().any(|c| c.is_control() || c == ' ' || c == '"')
}

/// Opens `url` in the default browser.
pub fn open_url(url: &str, lang: Lang) -> Result<(), String> {
    use windows::core::{w, HSTRING};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    if !allowed_url(url, REPO) {
        return Err(t(lang, "update.bad_url").into());
    }
    // SAFETY: both strings outlive the call; ShellExecuteW only reads them.
    let r = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // values above 32 mean success
    if r.0 as usize > 32 {
        Ok(())
    } else {
        Err(t(lang, "update.open_failed").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{"url":"https://api.github.com/x","tag_name":"v0.2.0","name":"Second",
        "html_url":"https://github.com/kaan0d/Tessera/releases/tag/v0.2.0","draft":false,
        "assets":[{"name":"setup.exe"}]}"#;

    #[test]
    fn semver_compare() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        // a pre-release sorts below its release
        assert!(!is_newer("1.0.0-beta.1", "1.0.0"));
        assert!(is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(!is_newer("junk", "0.1.0") && !is_newer("0.2.0", "junk"));
    }

    #[test]
    fn parses_fixture() {
        let i = parse_release(FIXTURE, "0.1.0", REPO, Lang::En).unwrap();
        assert!(i.available);
        assert_eq!((i.current.as_str(), i.latest.as_str()), ("0.1.0", "0.2.0"));
        assert!(i.url.ends_with("/releases/tag/v0.2.0"));
        let same = parse_release(FIXTURE, "0.2.0", REPO, Lang::Tr).unwrap();
        assert!(!same.available);
        assert_ne!(same.message, i.message);
    }

    #[test]
    fn rejects_bad_responses_and_foreign_urls() {
        for b in ["", "{}", "[]", r#"{"tag_name":"nightly","html_url":"x"}"#] {
            assert!(parse_release(b, "0.1.0", REPO, Lang::En).is_err(), "{b}");
        }
        let evil = r#"{"tag_name":"v9.0.0","html_url":"https://evil.example/x"}"#;
        let i = parse_release(evil, "0.1.0", REPO, Lang::En).unwrap();
        assert_eq!(i.url, "https://github.com/kaan0d/Tessera/releases");
    }

    #[test]
    fn finds_the_installer_assets() {
        let dl = "https://github.com/kaan0d/Tessera/releases/download/v0.2.0/";
        let body = |exe: &str| {
            format!(
                r#"{{"tag_name":"v0.2.0","html_url":"x","assets":[
                {{"name":"tessera-setup-v0.2.0.exe","browser_download_url":"{exe}"}},
                {{"name":"tessera-setup-v0.2.0.exe.sha256","browser_download_url":"{dl}tessera-setup-v0.2.0.exe.sha256"}}]}}"#
            )
        };
        let good = body(&format!("{dl}tessera-setup-v0.2.0.exe"));
        let (exe, sha) = installer_urls(&good, "0.2.0", REPO).unwrap();
        assert!(exe.ends_with(".exe") && sha.ends_with(".sha256"));
        // another version, a foreign host, or no assets: nothing to install
        assert!(installer_urls(&good, "0.3.0", REPO).is_none());
        assert!(installer_urls(&body("https://evil.example/a.exe"), "0.2.0", REPO).is_none());
        assert!(installer_urls(FIXTURE, "0.2.0", REPO).is_none());
    }

    #[test]
    fn reads_a_sha256sum_line() {
        let h = "B".repeat(64);
        assert_eq!(parse_sha256(&format!("{h} *a.exe
")), Some("b".repeat(64)));
        assert_eq!(parse_sha256("short"), None);
        assert_eq!(parse_sha256(&"z".repeat(64)), None);
    }

    #[test]
    fn missing_release_is_up_to_date() {
        let i = no_releases("0.1.0", REPO, Lang::En);
        assert!(!i.available && i.latest == "0.1.0");
        assert_ne!(i.message, no_releases("0.1.0", REPO, Lang::Tr).message);
    }

    #[test]
    fn open_url_allowlist() {
        let ok = |u| allowed_url(u, REPO);
        assert!(ok("https://github.com/kaan0d/Tessera"));
        assert!(ok("https://github.com/kaan0d/Tessera/releases/tag/v1"));
        assert!(!ok("http://github.com/kaan0d/Tessera/releases"));
        assert!(!ok("https://github.com/kaan0d/Tessera-evil/x"));
        assert!(!ok("https://github.com/kaan0d/other"));
        assert!(!ok("https://github.com.evil.com/kaan0d/Tessera/"));
        assert!(!ok("https://github.com/kaan0d/Tessera/\" & calc"));
        assert!(!ok("file:///C:/Windows/notepad.exe"));
        assert!(open_url("https://example.com", Lang::En).is_err());
    }
}
