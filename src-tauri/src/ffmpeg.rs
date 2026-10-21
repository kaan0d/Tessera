//! ffmpeg download: one pinned LGPL win64 build, SHA-256 verified before anything is
//! extracted, only `bin/ffmpeg.exe` kept. Lives in %APPDATA%\<app>\ffmpeg\.
//! Pure functions plus a small shared status; the app wiring is in lib.rs.

use std::fs::{self, File};
use std::io::{self, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::settings::settings_path;

/// BtbN/FFmpeg-Builds, release `autobuild-2026-10-20-13-08`, ffmpeg n8.1.3, LGPL (no x264/x265/fdk).
/// The hash is of this exact asset, so the host is not trusted. Encoders used by the runner:
/// libopenh264, libvpx-vp9, libopus, libvorbis, libmp3lame, aac, mpeg4, gif, flac, pcm_s16le.
pub const URL: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-20-13-08/ffmpeg-n8.1.3-18-g46d899d7a0-win64-lgpl-8.1.zip";
pub const SHA256: &str = "26634a5e06429c4f51ed08ba518923f928c5e4a49ed0ee62f701e035227b91a7";
/// Used for the percent when the server sends no length.
const EXPECTED_SIZE: u64 = 170_543_601;
const MAX_ZIP: u64 = 400 << 20;
const MAX_EXE: u64 = 400 << 20;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn ffmpeg_dir() -> PathBuf {
    settings_path().with_file_name("ffmpeg")
}

pub fn exe_path() -> PathBuf {
    ffmpeg_dir().join("ffmpeg.exe")
}

#[derive(Serialize)]
pub struct FfmpegStatus {
    pub ready: bool,
    pub downloading: bool,
    pub percent: f32,
    pub error: Option<String>,
}

/// Download state shared between the worker thread and the UI poll.
#[derive(Default)]
pub struct Download {
    active: AtomicBool,
    permille: AtomicU32,
    error: Mutex<Option<String>>,
}

impl Download {
    /// False if a download is already running.
    pub fn begin(&self) -> bool {
        if self.active.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.permille.store(0, Ordering::Relaxed);
        *self.error.lock().unwrap() = None;
        true
    }

    pub fn set_percent(&self, p: f32) {
        self.permille
            .store((p.clamp(0.0, 100.0) * 10.0) as u32, Ordering::Relaxed);
    }

    pub fn finish(&self, error: Option<String>) {
        *self.error.lock().unwrap() = error;
        self.active.store(false, Ordering::SeqCst);
    }

    pub fn status(&self, ready: bool) -> FfmpegStatus {
        FfmpegStatus {
            ready,
            downloading: self.active.load(Ordering::SeqCst),
            percent: self.permille.load(Ordering::Relaxed) as f32 / 10.0,
            error: self.error.lock().unwrap().clone(),
        }
    }
}

/// True if `exe` runs and identifies itself as ffmpeg (catches a missing or corrupt file).
// ponytail: not a hash check; a tampered but working exe passes. Re-hash the exe if that matters.
pub fn is_working(exe: &Path) -> bool {
    Command::new(exe)
        .arg("-version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success() && o.stdout.starts_with(b"ffmpeg version"))
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let got = sha256_file(path).map_err(|e| format!("Dosya okunamadı: {e}"))?;
    if got.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err("İndirilen dosya doğrulanamadı (SHA-256 uyuşmuyor)".into())
    }
}

/// Streams `url` into `dest`, reporting 0.0..=1.0.
fn download(url: &str, dest: &Path, progress: &mut dyn FnMut(f32)) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(30 * 60)))
        .build()
        .into();
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("İndirme başlatılamadı: {e}"))?;
    let total = resp
        .body()
        .content_length()
        .filter(|&n| n > 0)
        .unwrap_or(EXPECTED_SIZE);
    if total > MAX_ZIP {
        return Err("İndirilecek dosya beklenenden büyük".into());
    }
    let mut body = resp.into_body().into_with_config().limit(MAX_ZIP).reader();
    let mut out = File::create(dest).map_err(|e| format!("Geçici dosya oluşturulamadı: {e}"))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = body
            .read(&mut buf)
            .map_err(|e| format!("İndirme kesildi: {e}"))?;
        if n == 0 {
            break;
        }
        io::Write::write_all(&mut out, &buf[..n]).map_err(|e| format!("Diske yazılamadı: {e}"))?;
        done += n as u64;
        progress((done as f32 / total as f32).min(1.0));
    }
    out.sync_all().map_err(|e| format!("Diske yazılamadı: {e}"))
}

/// Extracts only `*/bin/ffmpeg.exe` from the zip into `dir/ffmpeg.exe` (atomic rename).
/// Any entry with a traversal or absolute path rejects the whole archive.
pub fn extract_exe(zip_path: &Path, dir: &Path) -> Result<(), String> {
    let bad = |e: &dyn std::fmt::Display| format!("Arşiv açılamadı: {e}");
    let mut z =
        zip::ZipArchive::new(File::open(zip_path).map_err(|e| bad(&e))?).map_err(|e| bad(&e))?;
    let mut found = None;
    for i in 0..z.len() {
        let e = z.by_index(i).map_err(|e| bad(&e))?;
        let name = e.enclosed_name().ok_or("Arşivde güvensiz dosya yolu var")?;
        let in_bin = name
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|p| p == "bin");
        if e.is_file() && in_bin && name.file_name().is_some_and(|n| n == "ffmpeg.exe") {
            found = Some(i);
        }
    }
    let i = found.ok_or("Arşivde ffmpeg.exe bulunamadı")?;
    let mut entry = z.by_index(i).map_err(|e| bad(&e))?;
    fs::create_dir_all(dir).map_err(|e| bad(&e))?;
    let part = dir.join("ffmpeg.exe.part");
    let result = (|| -> Result<(), String> {
        let mut out = File::create(&part).map_err(|e| bad(&e))?;
        let n = io::copy(&mut (&mut entry).take(MAX_EXE + 1), &mut out).map_err(|e| bad(&e))?;
        if n > MAX_EXE {
            return Err("ffmpeg.exe beklenenden büyük".into());
        }
        out.sync_all().map_err(|e| bad(&e))?;
        drop(out);
        fs::rename(&part, dir.join("ffmpeg.exe")).map_err(|e| bad(&e))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

/// One full attempt: download, verify the hash, extract, check the result runs.
pub fn install_once(
    url: &str,
    sha256: &str,
    dir: &Path,
    progress: &mut dyn FnMut(f32),
) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("Klasör oluşturulamadı: {e}"))?;
    let tmp = dir.join("ffmpeg.zip.part");
    let result = (|| {
        download(url, &tmp, &mut |f| progress(f * 97.0))?;
        progress(98.0);
        verify_sha256(&tmp, sha256)?;
        extract_exe(&tmp, dir)?;
        if !is_working(&dir.join("ffmpeg.exe")) {
            return Err("ffmpeg çalıştırılamadı".into());
        }
        progress(100.0);
        Ok(())
    })();
    let _ = fs::remove_file(&tmp);
    result
}

/// The pinned build, up to three attempts with a growing pause. Updates `dl` as it goes.
pub fn install_with_retries(dl: &Download) -> Result<(), String> {
    let mut last = String::new();
    for pause in [0u64, 3, 10] {
        std::thread::sleep(Duration::from_secs(pause));
        dl.set_percent(0.0);
        match install_once(URL, SHA256, &ffmpeg_dir(), &mut |p| dl.set_percent(p)) {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
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
    fn pinned_source_is_well_formed() {
        assert!(
            URL.starts_with("https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-")
        );
        assert!(URL.ends_with("-win64-lgpl-8.1.zip") && !URL.contains("gpl-shared"));
        assert!(URL.contains("/ffmpeg-n8.1.3-18-g46d899d7a0-"));
        assert!(SHA256.len() == 64 && SHA256.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(SHA256, SHA256.to_ascii_lowercase());
    }

    #[test]
    fn hash_verification_accepts_and_rejects() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.bin");
        fs::write(&f, b"abc").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_file(&f).unwrap(), abc);
        verify_sha256(&f, abc).unwrap();
        verify_sha256(&f, &abc.to_uppercase()).unwrap();
        assert!(verify_sha256(&f, &"0".repeat(64)).is_err());
        assert!(verify_sha256(&d.path().join("yok"), abc).is_err());
    }

    #[test]
    fn extracts_only_ffmpeg_exe() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("f.zip");
        fake_zip(
            &z,
            &[
                ("ffmpeg-x/bin/ffprobe.exe", b"probe"),
                ("ffmpeg-x/bin/ffmpeg.exe", b"MZfake"),
                ("ffmpeg-x/LICENSE.txt", b"lic"),
            ],
        );
        let out = d.path().join("out");
        extract_exe(&z, &out).unwrap();
        assert_eq!(fs::read(out.join("ffmpeg.exe")).unwrap(), b"MZfake");
        assert_eq!(fs::read_dir(&out).unwrap().count(), 1); // no .part, no other files
    }

    #[test]
    fn rejects_traversal_and_missing_exe() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("evil.zip");
        fake_zip(
            &z,
            &[("x/bin/ffmpeg.exe", b"MZ"), ("../../evil.txt", b"pwn")],
        );
        let out = d.path().join("out");
        assert!(extract_exe(&z, &out).unwrap_err().contains("güvensiz"));
        assert!(!out.join("ffmpeg.exe").exists());
        fake_zip(&z, &[(r"..\evil.txt", b"x"), ("x/bin/ffmpeg.exe", b"MZ")]);
        assert!(extract_exe(&z, &out).is_err());
        fake_zip(&z, &[("x/ffmpeg.exe", b"MZ")]); // not under bin/
        assert!(extract_exe(&z, &out).unwrap_err().contains("bulunamadı"));
    }

    #[test]
    fn status_tracks_download() {
        let dl = Download::default();
        assert!(dl.begin() && !dl.begin());
        dl.set_percent(42.5);
        let s = dl.status(false);
        assert!(s.downloading && (s.percent - 42.5).abs() < 0.11 && s.error.is_none());
        dl.finish(Some("hata".into()));
        let s = dl.status(false);
        assert!(!s.downloading && s.error.as_deref() == Some("hata"));
        assert!(dl.begin() && dl.status(false).error.is_none());
    }
}
