//! Tesseract as the second OCR engine, for languages Windows has no recognizer for. Downloaded
//! only when the user asks (Settings > OCR): a trimmed Tesseract 5.5.3 bundle from the project's
//! own release (SHA-256 verified before anything is extracted) plus one `tessdata_fast` file per
//! language, each pinned by hash. Lives in `<data>\tesseract\`; run as a separate process, never
//! linked. The bundle is the official installer's `tesseract.exe` and the DLLs it loads, with the
//! GPL `libjbig-0.dll` swapped for the MIT stub in `tools/libjbig-stub`.

use std::fs::{self, File};
use std::io::{self, Cursor, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use image::{DynamicImage, ImageFormat, RgbImage};

use crate::convert::ocr::{Line, OcrPage, Word};
use crate::convert::{ConvertError, Options};
use crate::ffmpeg::{FfError, Fetch, download_capped, verify_sha256};
use crate::settings::data_dir;

pub const VERSION: &str = "5.5.3";
/// The bundle on the project's release. There is no second source: the official installer
/// holds the GPL JBIG library, so it cannot be used as is.
pub const BUNDLE_URL: &str = "https://github.com/kaan0d/Tessera/releases/download/tesseract-5.5.3/tesseract-5.5.3-win64-tessera.zip";
pub const BUNDLE_SHA256: &str = "2ff4a233891bd35d728a6728de0144d602db3cbc209953321545978a74111770";
pub const BUNDLE_SIZE: u64 = 17_571_857;
const MAX_ZIP: u64 = 60 << 20;
const MAX_FILE: u64 = 80 << 20;
const MAX_UNPACKED: u64 = 160 << 20;
const MAX_TRAINEDDATA: u64 = 40 << 20;
const TIMEOUT: Duration = Duration::from_secs(120);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `tessdata_fast` at a fixed commit: the hash below is of the file at exactly this commit.
const TESSDATA_COMMIT: &str = "87416418657359cb625c412a48b6e1d6d41c29bd";
/// More languages than this at once only slow the engine down.
const MAX_AT_ONCE: usize = 3;

/// One downloadable language: Tesseract code, BCP-47 tag, SHA-256 and size of its file.
pub struct Lang {
    pub code: &'static str,
    pub tag: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

/// The app's twelve languages. Order is the order Tesseract gets them in when none is chosen.
pub const LANGS: [Lang; 12] = [
    Lang { code: "eng", tag: "en", sha256: "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2", size: 4_113_088 },
    Lang { code: "tur", tag: "tr", sha256: "7393381111e1152420fc4092cb44eef4237580d21b92bf30d7d221aad192c6b7", size: 4_550_554 },
    Lang { code: "deu", tag: "de", sha256: "19d219bbb6672c869d20a9636c6816a81eb9a71796cb93ebe0cb1530e2cdb22d", size: 1_525_436 },
    Lang { code: "fra", tag: "fr", sha256: "ced037562e8c80c13122dece28dd477d399af80911a28791a66a63ac1e3445ca", size: 1_130_365 },
    Lang { code: "spa", tag: "es", sha256: "6f2e04d02774a18f01bed44b1111f2cd7f3ba7ac9dc4373cd3f898a40ea6b464", size: 2_294_433 },
    Lang { code: "ita", tag: "it", sha256: "b8f89e1e785118dac4d51ae042c029a64edb5c3ee42ef73027a6d412748d8827", size: 2_701_314 },
    Lang { code: "por", tag: "pt", sha256: "c4932b937207a9514b7514d518b931a99938c02a28a5a5a553f8599ed58b7deb", size: 1_982_756 },
    Lang { code: "rus", tag: "ru", sha256: "e16e5e036cce1d9ec2b00063cf8b54472625b9e14d893a169e2b0dedeb4df225", size: 3_861_738 },
    Lang { code: "ind", tag: "id", sha256: "69786901da87ab8766c1ea7fbb10b28f2110c14da3f6c8f2735df131fba95d88", size: 1_122_661 },
    Lang { code: "jpn", tag: "ja", sha256: "1f5de9236d2e85f5fdf4b3c500f2d4926f8d9449f28f5394472d9e8d83b91b4d", size: 2_471_260 },
    Lang { code: "kor", tag: "ko", sha256: "6b85e11d9bbf07863b97b3523b1b112844c43e713df8b66418a081fd1060b3b2", size: 1_677_415 },
    Lang { code: "chi_sim", tag: "zh-Hans", sha256: "a5fcb6f0db1e1d6d8522f39db4e848f05984669172e584e8d76b6b3141e1f730", size: 2_469_156 },
];

pub fn lang_info(code: &str) -> Option<&'static Lang> {
    LANGS.iter().find(|l| l.code == code)
}

pub fn lang_url(code: &str) -> String {
    format!("https://github.com/tesseract-ocr/tessdata_fast/raw/{TESSDATA_COMMIT}/{code}.traineddata")
}

/// Tesseract code for a BCP-47 tag (`tr`, `en-US`, `zh-Hans-CN`), None when there is none.
/// Traditional Chinese is not offered.
pub fn code_for_tag(tag: &str) -> Option<&'static str> {
    let lower = tag.to_ascii_lowercase();
    let mut parts = lower.split('-');
    let primary = parts.next()?;
    if primary == "zh" && parts.any(|p| matches!(p, "hant" | "tw" | "hk" | "mo")) {
        return None;
    }
    LANGS
        .iter()
        .find(|l| l.tag.split('-').next() == Some(primary))
        .map(|l| l.code)
}

pub fn tag_for_code(code: &str) -> Option<&'static str> {
    lang_info(code).map(|l| l.tag)
}

pub fn dir() -> PathBuf {
    data_dir().join("tesseract")
}

pub fn exe_path() -> PathBuf {
    dir().join("tesseract.exe")
}

fn tessdata_in(dir: &Path) -> PathBuf {
    dir.join("tessdata")
}

/// Codes of the languages whose file is present, in `LANGS` order.
pub fn installed_langs() -> Vec<&'static str> {
    installed_in(&dir())
}

fn installed_in(dir: &Path) -> Vec<&'static str> {
    LANGS
        .iter()
        .filter(|l| {
            tessdata_in(dir)
                .join(format!("{}.traineddata", l.code))
                .is_file()
        })
        .map(|l| l.code)
        .collect()
}

/// The engine is downloaded (languages are separate).
pub fn engine_present() -> bool {
    exe_path().is_file()
}

/// True if `exe` runs and identifies itself (catches a missing or corrupt file).
pub fn is_working(exe: &Path) -> bool {
    Command::new(exe)
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .arg("--version")
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| {
            o.status.success()
                && (o.stdout.starts_with(b"tesseract v") || o.stderr.starts_with(b"tesseract v"))
        })
}

/// Unpacks the bundle's files (flat: `.exe`, `.dll`, `.txt`) into `to`. A name with a path in
/// it, a traversal or an oversized file rejects the whole archive.
pub fn extract_all(zip_path: &Path, to: &Path) -> Result<(), FfError> {
    let bad = |e: &dyn std::fmt::Display| FfError::with("ff.archive", e);
    let mut z =
        zip::ZipArchive::new(File::open(zip_path).map_err(|e| bad(&e))?).map_err(|e| bad(&e))?;
    fs::create_dir_all(to).map_err(|e| bad(&e))?;
    let mut total = 0u64;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| bad(&e))?;
        let name = e.enclosed_name().ok_or_else(|| FfError::new("ff.unsafe"))?;
        if !e.is_file() {
            continue;
        }
        let mut comps = name.components();
        let (Some(std::path::Component::Normal(file)), None) = (comps.next(), comps.next()) else {
            return Err(FfError::new("ff.unsafe"));
        };
        let ext = Path::new(file)
            .extension()
            .map(|x| x.to_string_lossy().to_ascii_lowercase());
        if !matches!(ext.as_deref(), Some("exe" | "dll" | "txt")) {
            continue;
        }
        let mut out = File::create(to.join(file)).map_err(|e| bad(&e))?;
        let n = io::copy(&mut (&mut e).take(MAX_FILE + 1), &mut out).map_err(|e| bad(&e))?;
        total += n;
        if n > MAX_FILE || total > MAX_UNPACKED {
            return Err(FfError::new("ff.exe_big"));
        }
        out.sync_all().map_err(|e| bad(&e))?;
    }
    if !to.join("tesseract.exe").is_file() {
        return Err(FfError::new("tess.missing"));
    }
    Ok(())
}

fn default_fetch(url: &str, dest: &Path, p: &mut dyn FnMut(f32)) -> Result<(), FfError> {
    download_capped(url, dest, MAX_ZIP, BUNDLE_SIZE, p)
}

/// Downloads the pinned bundle and installs it, keeping the languages already there.
pub fn install_engine(progress: &mut dyn FnMut(f32)) -> Result<(), FfError> {
    install_engine_from(BUNDLE_URL, BUNDLE_SHA256, &dir(), progress, &default_fetch)
}

pub fn install_engine_from(
    url: &str,
    sha256: &str,
    dir: &Path,
    progress: &mut dyn FnMut(f32),
    fetch: Fetch,
) -> Result<(), FfError> {
    let parent = dir.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| FfError::with("ff.mkdir", e))?;
    let zip = parent.join("tesseract.zip.part");
    let work = parent.join("tesseract.work");
    let _ = fs::remove_dir_all(&work);
    let result = (|| {
        fetch(url, &zip, &mut |f| progress(f * 90.0))?;
        progress(92.0);
        verify_sha256(&zip, sha256)?;
        extract_all(&zip, &work)?;
        progress(96.0);
        if !is_working(&work.join("tesseract.exe")) {
            return Err(FfError::new("tess.run"));
        }
        // languages survive an engine update
        let _ = fs::rename(tessdata_in(dir), tessdata_in(&work));
        match fs::remove_dir_all(dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(FfError::with("ff.write", e));
            }
            _ => {}
        }
        fs::rename(&work, dir).map_err(|e| FfError::with("ff.write", e))?;
        progress(100.0);
        Ok(())
    })();
    let _ = fs::remove_file(&zip);
    let _ = fs::remove_dir_all(&work);
    result
}

/// Downloads one language into the installed engine's tessdata folder.
pub fn install_lang(code: &str, progress: &mut dyn FnMut(f32)) -> Result<(), FfError> {
    let lang = lang_info(code).ok_or_else(|| FfError::with("ff.start", code))?;
    install_lang_from(&lang_url(code), lang, &dir(), progress, &|u, d, p| {
        download_capped(u, d, MAX_TRAINEDDATA, lang.size, p)
    })
}

pub fn install_lang_from(
    url: &str,
    lang: &Lang,
    dir: &Path,
    progress: &mut dyn FnMut(f32),
    fetch: Fetch,
) -> Result<(), FfError> {
    if !dir.join("tesseract.exe").is_file() {
        return Err(FfError::new("tess.missing"));
    }
    let td = tessdata_in(dir);
    fs::create_dir_all(&td).map_err(|e| FfError::with("ff.mkdir", e))?;
    let part = td.join(format!("{}.traineddata.part", lang.code));
    let result = (|| {
        fetch(url, &part, &mut |f| progress(f * 98.0))?;
        verify_sha256(&part, lang.sha256)?;
        fs::rename(&part, td.join(format!("{}.traineddata", lang.code)))
            .map_err(|e| FfError::with("ff.write", e))?;
        progress(100.0);
        Ok(())
    })();
    let _ = fs::remove_file(&part);
    result
}

pub fn remove_lang(code: &str) -> io::Result<()> {
    let Some(l) = lang_info(code) else {
        return Ok(());
    };
    match fs::remove_file(tessdata_in(&dir()).join(format!("{}.traineddata", l.code))) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Deletes the engine and every language.
pub fn remove() -> io::Result<()> {
    match fs::remove_dir_all(dir()) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Languages Tesseract should read: the wanted tag's, or (no tag) the installed ones.
/// Empty when that language is not installed.
fn wanted(lang: Option<&str>, installed: &[&'static str]) -> Vec<&'static str> {
    match lang {
        Some(tag) => code_for_tag(tag)
            .filter(|c| installed.contains(c))
            .into_iter()
            .collect(),
        // shortcut: no tag reads every installed language (up to three), not the user's Windows
        // languages; fine until someone installs more than they use
        None => installed.iter().copied().take(MAX_AT_ONCE).collect(),
    }
}

/// Recognizes `img` with the downloaded engine. `OcrNoLanguage` when there is no engine or not
/// that language, so the caller can report what is missing.
pub fn recognize(
    img: &DynamicImage,
    lang: Option<&str>,
    opts: &Options,
) -> Result<OcrPage, ConvertError> {
    let dir = dir();
    if !dir.join("tesseract.exe").is_file() {
        return Err(ConvertError::OcrNoLanguage);
    }
    let codes = wanted(lang, &installed_in(&dir));
    if codes.is_empty() {
        return Err(ConvertError::OcrNoLanguage);
    }
    run_in(&dir, &codes, img, opts)
}

/// The picture as PNG bytes, transparency over white (Tesseract reads transparent as black).
fn png_bytes(img: &DynamicImage) -> Result<Vec<u8>, ConvertError> {
    let rgba = img.to_rgba8();
    let mut rgb = RgbImage::new(rgba.width(), rgba.height());
    for (o, p) in rgb.pixels_mut().zip(rgba.pixels()) {
        let a = p[3] as u32;
        let over = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        o.0 = [over(p[0]), over(p[1]), over(p[2])];
    }
    let mut out = Cursor::new(vec![]);
    DynamicImage::ImageRgb8(rgb)
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|e| ConvertError::OcrFailed(e.to_string()))?;
    Ok(out.into_inner())
}

/// Ends the process and anything it started.
fn kill_tree(pid: u32) {
    crate::convert::office::kill_tree(pid);
}

fn run_in(
    dir: &Path,
    codes: &[&str],
    img: &DynamicImage,
    opts: &Options,
) -> Result<OcrPage, ConvertError> {
    if img.width() == 0 || img.height() == 0 {
        return Err(ConvertError::Corrupt);
    }
    let png = png_bytes(img)?;
    // The picture goes in on stdin and the data folder is given relative to the working
    // directory: Tesseract reads narrow (ANSI) paths, which break on a non-ASCII user name.
    let mut child = Command::new(dir.join("tesseract.exe"))
        .current_dir(dir)
        .args(["-", "stdout", "--tessdata-dir", "tessdata", "-l"])
        .arg(codes.join("+"))
        .args(["--psm", "3", "-c", "tessedit_create_tsv=1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| ConvertError::OcrFailed(e.to_string()))?;
    let mut stdin = child.stdin.take().expect("piped");
    let feeder = std::thread::spawn(move || {
        let _ = stdin.write_all(&png);
    });
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut buf = vec![];
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        let cancelled = opts
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed));
        if cancelled || started.elapsed() > TIMEOUT {
            kill_tree(child.id());
            let _ = child.wait();
            let _ = feeder.join();
            let _ = reader.join();
            return Err(if cancelled {
                ConvertError::Cancelled
            } else {
                ConvertError::OcrFailed("timed out".into())
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = feeder.join();
    let out = reader.join().unwrap_or_default();
    if !status.success() {
        return Err(ConvertError::OcrFailed(format!(
            "Tesseract exited with {}",
            status.code().unwrap_or(-1)
        )));
    }
    Ok(parse_tsv(&String::from_utf8_lossy(&out)))
}

/// Word rows (level 5) of Tesseract's TSV, grouped into lines. Line endings are CRLF or LF.
pub fn parse_tsv(tsv: &str) -> OcrPage {
    let mut lines: Vec<((u32, u32, u32), Line)> = vec![];
    for row in tsv.lines().skip(1) {
        let f: Vec<&str> = row.splitn(12, '\t').collect();
        if f.len() < 12 || f[0] != "5" {
            continue;
        }
        let text = f[11].trim();
        let num = |i: usize| f[i].trim().parse::<f32>().ok();
        let (Some(x), Some(y), Some(w), Some(h)) = (num(6), num(7), num(8), num(9)) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let id = |i: usize| f[i].trim().parse::<u32>().unwrap_or(0);
        let key = (id(2), id(3), id(4));
        let word = Word { text: text.to_string(), x, y, w, h };
        match lines.last_mut() {
            Some((k, line)) if *k == key => {
                line.text.push(' ');
                line.text.push_str(text);
                line.words.push(word);
            }
            _ => lines.push((key, Line { text: text.to_string(), words: vec![word] })),
        }
    }
    OcrPage {
        lines: lines.into_iter().map(|(_, l)| l).collect(),
        scale: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn fake_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, data) in entries {
            w.start_file(*name, SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn tags_and_codes_map_both_ways() {
        for (tag, code) in [
            ("tr", "tur"),
            ("tr-TR", "tur"),
            ("en-US", "eng"),
            ("EN", "eng"),
            ("zh-Hans", "chi_sim"),
            ("zh-CN", "chi_sim"),
            ("pt-BR", "por"),
            ("id", "ind"),
            ("ja-JP", "jpn"),
        ] {
            assert_eq!(code_for_tag(tag), Some(code), "{tag}");
        }
        for none in ["zh-Hant", "zh-TW", "zh-HK", "xx", "ar", "tur"] {
            assert_eq!(code_for_tag(none), None, "{none}");
        }
        for l in &LANGS {
            assert_eq!(code_for_tag(l.tag), Some(l.code));
            assert_eq!(tag_for_code(l.code), Some(l.tag));
            assert!(crate::convert::ocr::valid_lang(l.tag));
        }
    }

    #[test]
    fn pins_are_well_formed() {
        for l in &LANGS {
            assert_eq!(l.sha256.len(), 64, "{}", l.code);
            assert!(l.sha256.bytes().all(|b| b.is_ascii_hexdigit()), "{}", l.code);
            assert!(l.size > 100_000 && l.size < MAX_TRAINEDDATA);
        }
        assert_eq!(BUNDLE_SHA256.len(), 64);
        assert!(BUNDLE_URL.starts_with("https://github.com/kaan0d/Tessera/releases/download/"));
        assert!(lang_url("tur").contains(TESSDATA_COMMIT) && lang_url("tur").ends_with("/tur.traineddata"));
    }

    #[test]
    fn language_choice() {
        let have = ["eng", "tur", "deu", "fra", "spa"];
        assert_eq!(wanted(Some("tr-TR"), &have), ["tur"]);
        assert!(wanted(Some("ru"), &have).is_empty());
        assert!(wanted(Some("xx"), &have).is_empty());
        assert_eq!(wanted(None, &have), ["eng", "tur", "deu"]);
        assert!(wanted(None, &[]).is_empty());
    }

    #[test]
    fn tsv_becomes_lines_of_words_with_crlf_or_lf() {
        let rows = [
            "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext",
            "1\t1\t0\t0\t0\t0\t0\t0\t1000\t300\t-1\t",
            "4\t1\t1\t1\t1\t0\t35\t31\t515\t43\t-1\t",
            "5\t1\t1\t1\t1\t1\t35\t31\t129\t43\t93.2\tHello",
            "5\t1\t1\t1\t1\t2\t186\t31\t213\t43\t93.1\tG\u{fc}n",
            "5\t1\t1\t1\t2\t1\t33\t160\t184\t54\t91.5\t2026",
            "5\t1\t1\t1\t2\t2\t240\t161\t109\t53\t-1\t ",
        ];
        for nl in ["\r\n", "\n"] {
            let page = parse_tsv(&rows.join(nl));
            assert_eq!(page.lines.len(), 2, "{nl:?}");
            assert_eq!(page.lines[0].text, "Hello G\u{fc}n");
            assert_eq!(page.lines[1].text, "2026");
            let w = &page.lines[0].words[1];
            assert_eq!((w.x, w.y, w.w, w.h), (186.0, 31.0, 213.0, 43.0));
            assert_eq!(page.scale, 1.0);
        }
        assert!(parse_tsv("").lines.is_empty());
        assert!(parse_tsv("only header\n5\tbad\n").lines.is_empty());
    }

    #[test]
    fn extracts_flat_files_and_refuses_paths() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("b.zip");
        fake_zip(
            &z,
            &[("tesseract.exe", b"MZ"), ("libx.dll", b"MZ"), ("NOTICE.txt", b"n"), ("junk.bin", b"j")],
        );
        let out = d.path().join("out");
        extract_all(&z, &out).unwrap();
        assert!(out.join("tesseract.exe").is_file() && out.join("libx.dll").is_file());
        assert!(!out.join("junk.bin").exists());
        fake_zip(&z, &[("tesseract.exe", b"MZ"), ("../evil.dll", b"x")]);
        assert_eq!(extract_all(&z, &d.path().join("o2")).unwrap_err().key(), "ff.unsafe");
        fake_zip(&z, &[("sub/tesseract.exe", b"MZ")]);
        assert_eq!(extract_all(&z, &d.path().join("o3")).unwrap_err().key(), "ff.unsafe");
        fake_zip(&z, &[("libx.dll", b"MZ")]);
        assert_eq!(extract_all(&z, &d.path().join("o4")).unwrap_err().key(), "tess.missing");
    }

    #[test]
    fn engine_install_checks_hash_before_unpacking() {
        let d = tempfile::tempdir().unwrap();
        let fetch = |_: &str, dest: &Path, p: &mut dyn FnMut(f32)| -> Result<(), FfError> {
            fake_zip(dest, &[("tesseract.exe", b"MZfake")]);
            p(1.0);
            Ok(())
        };
        let dir = d.path().join("tesseract");
        let e = install_engine_from("u", &"0".repeat(64), &dir, &mut |_| {}, &fetch).unwrap_err();
        assert_eq!(e.key(), "ff.hash");
        assert!(!dir.exists() && !d.path().join("tesseract.work").exists());
        // right hash: unpacks, then fails only at "does it run" (the exe is fake)
        let z = d.path().join("ref.zip");
        fake_zip(&z, &[("tesseract.exe", b"MZfake")]);
        let sha = crate::ffmpeg::sha256_file(&z).unwrap();
        let e = install_engine_from("u", &sha, &dir, &mut |_| {}, &fetch).unwrap_err();
        assert_eq!(e.key(), "tess.run");
        assert!(!dir.exists());
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1); // only ref.zip left
    }

    #[test]
    fn language_install_verifies_and_needs_the_engine() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("tesseract");
        let data: &[u8] = b"fake traineddata";
        let f = d.path().join("f.bin");
        fs::write(&f, data).unwrap();
        let sha: &'static str = Box::leak(crate::ffmpeg::sha256_file(&f).unwrap().into_boxed_str());
        let lang = Lang { code: "tur", tag: "tr", sha256: sha, size: 16 };
        let fetch = |_: &str, dest: &Path, p: &mut dyn FnMut(f32)| -> Result<(), FfError> {
            fs::write(dest, data).unwrap();
            p(1.0);
            Ok(())
        };
        let e = install_lang_from("u", &lang, &dir, &mut |_| {}, &fetch).unwrap_err();
        assert_eq!(e.key(), "tess.missing");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("tesseract.exe"), b"MZ").unwrap();
        install_lang_from("u", &lang, &dir, &mut |_| {}, &fetch).unwrap();
        assert_eq!(installed_in(&dir), ["tur"]);
        assert_eq!(fs::read(dir.join("tessdata").join("tur.traineddata")).unwrap(), data);
        // a wrong hash installs nothing
        let other = Lang { code: "deu", tag: "de", sha256: "0000000000000000000000000000000000000000000000000000000000000000", size: 16 };
        let e = install_lang_from("u", &other, &dir, &mut |_| {}, &fetch).unwrap_err();
        assert_eq!(e.key(), "ff.hash");
        assert_eq!(installed_in(&dir), ["tur"]);
        assert_eq!(fs::read_dir(dir.join("tessdata")).unwrap().count(), 1);
    }

    /// Needs a real engine: TESSERA_TEST_TESSERACT = folder with tesseract.exe and tessdata\
    /// (eng + tur); TESSERA_TEST_PNG = a picture reading "Hello ... 2026". Run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn reads_a_picture_with_the_real_engine() {
        let dir = PathBuf::from(std::env::var("TESSERA_TEST_TESSERACT").unwrap());
        let img = image::open(std::env::var("TESSERA_TEST_PNG").unwrap()).unwrap();
        let page = run_in(&dir, &["eng", "tur"], &img, &Options::default()).unwrap();
        let text = page.text();
        assert!(text.contains("Hello") && text.contains("2026"), "{text:?}");
        assert!(page.lines.iter().all(|l| !l.words.is_empty()));
    }
}
