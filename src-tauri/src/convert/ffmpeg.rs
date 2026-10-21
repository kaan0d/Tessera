//! Runs ffmpeg.exe as a child process for video and the audio targets the pure-Rust
//! engine cannot write. Arguments are separate items (no shell), the output goes to a
//! temp file next to the destination and is renamed into place like every other path.

use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use super::{output, security, supported_targets, uses_ffmpeg};
use super::{Conflict, ConvertError, Format, Options};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// ffmpeg muxer name; the temp output has a `.tmp` extension, so it is always explicit.
fn muxer(t: Format) -> &'static str {
    match t {
        Format::Mp4 => "mp4",
        Format::Mkv => "matroska",
        Format::Avi => "avi",
        Format::Webm => "webm",
        Format::Mov => "mov",
        Format::Gif => "gif",
        Format::Mp3 => "mp3",
        Format::Ogg => "ogg",
        Format::Aac => "adts",
        Format::Wav => "wav",
        Format::Flac => "flac",
        _ => "null",
    }
}

/// Codec arguments per target. Only encoders present in the pinned LGPL build:
/// libopenh264 (H.264), libvpx-vp9, libopus, libvorbis, libmp3lame, native aac/mpeg4/gif.
fn codec_args(t: Format) -> &'static [&'static str] {
    const H264: &[&str] = &[
        "-map",
        "0:v:0",
        "-map",
        "0:a:0?",
        "-vf",
        "scale=trunc(iw/2)*2:trunc(ih/2)*2",
        "-c:v",
        "libopenh264",
        "-b:v",
        "6M",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-b:a",
        "192k",
    ];
    match t {
        Format::Mp4 | Format::Mkv | Format::Mov => H264,
        Format::Avi => &[
            "-map", "0:v:0", "-map", "0:a:0?", "-c:v", "mpeg4", "-q:v", "4", "-pix_fmt", "yuv420p",
            "-c:a", "libmp3lame", "-q:a", "4",
        ],
        Format::Webm => &[
            "-map", "0:v:0", "-map", "0:a:0?", "-c:v", "libvpx-vp9", "-crf", "32", "-b:v", "0",
            "-row-mt", "1", "-deadline", "good", "-cpu-used", "4", "-pix_fmt", "yuv420p", "-c:a",
            "libopus", "-b:a", "128k",
        ],
        Format::Gif => &[
            "-map", "0:v:0", "-an", "-loop", "0", "-vf",
            "fps=12,scale='min(480,iw)':-1:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse",
        ],
        Format::Mp3 => &["-vn", "-map", "0:a:0", "-c:a", "libmp3lame", "-q:a", "2"],
        Format::Ogg => &["-vn", "-map", "0:a:0", "-c:a", "libvorbis", "-q:a", "5"],
        Format::Aac => &["-vn", "-map", "0:a:0", "-c:a", "aac", "-b:a", "192k"],
        Format::Wav => &["-vn", "-map", "0:a:0", "-c:a", "pcm_s16le"],
        Format::Flac => &["-vn", "-map", "0:a:0", "-c:a", "flac"],
        _ => &[],
    }
}

/// Full argument list (without the program name).
pub fn build_args(input: &Path, output: &Path, target: Format) -> Vec<OsString> {
    let mut a: Vec<OsString> = [
        "-nostdin",
        "-y",
        "-hide_banner",
        "-nostats",
        "-progress",
        "pipe:1",
        "-i",
    ]
    .map(OsString::from)
    .into();
    a.push(input.into());
    a.extend(codec_args(target).iter().map(OsString::from));
    a.extend(["-f", muxer(target)].map(OsString::from));
    if target == Format::Mp4 {
        a.extend(["-movflags", "+faststart"].map(OsString::from));
    }
    a.push(output.into());
    a
}

/// "  Duration: 00:01:23.45, start: ..." -> microseconds. `N/A` gives None.
pub fn parse_duration_us(line: &str) -> Option<u64> {
    let rest = line.trim().strip_prefix("Duration:")?.trim_start();
    let t = rest.split(',').next()?.trim();
    let mut parts = t.split(':');
    let h: u64 = parts.next()?.parse().ok()?;
    let m: u64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !s.is_finite() || s < 0.0 {
        return None;
    }
    Some((h * 3600 + m * 60) * 1_000_000 + (s * 1e6) as u64)
}

/// "out_time_us=1234567" -> 1234567. Negative or `N/A` (ffmpeg's start-up value) gives None.
pub fn parse_progress_us(line: &str) -> Option<u64> {
    line.trim().strip_prefix("out_time_us=")?.parse().ok()
}

/// Reads a stream split on both `\n` and `\r`, handing each non-empty line to `f`.
fn for_each_line(r: impl std::io::Read, mut f: impl FnMut(&str)) {
    let mut r = BufReader::new(r);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        // ffmpeg ends lines with \n; progress updates use \r when stats are on
        match r.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                for part in String::from_utf8_lossy(&buf).split(['\r', '\n']) {
                    if !part.trim().is_empty() {
                        f(part);
                    }
                }
            }
        }
    }
}

/// Converts `input` with ffmpeg. `cancel` kills the child and removes the temp file.
/// `progress` receives 0.0..=1.0 (stays at 0 when the duration is unknown).
pub fn convert_with_ffmpeg(
    ffmpeg: &Path,
    input: &Path,
    target: Format,
    opts: &Options,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    let (src, from) = security::validate_input(input)?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !supported_targets(ext).contains(&target) || !uses_ffmpeg(from, target) {
        return Err(ConvertError::Unsupported);
    }
    if !ffmpeg.is_file() {
        return Err(ConvertError::NeedsFfmpeg);
    }
    let dest = output::plan(&src, target.ext(), &opts.output)?;
    progress(0.0);
    output::write_via_temp(&dest, |tmp| {
        run_child(ffmpeg, &src, tmp, target, cancel, &mut progress)
    })?;
    if opts.output.conflict == Conflict::ReplaceOriginal {
        output::recycle(&src)?;
    }
    progress(1.0);
    Ok(dest)
}

fn run_child(
    ffmpeg: &Path,
    src: &Path,
    tmp: &Path,
    target: Format,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(f32),
) -> Result<(), ConvertError> {
    let mut child = Command::new(ffmpeg)
        .args(build_args(src, tmp, target))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| ConvertError::Io(format!("ffmpeg başlatılamadı: {e}")))?;

    // both pipes are drained on their own threads, otherwise a full pipe stalls ffmpeg
    let duration = Arc::new(AtomicU64::new(0));
    let err_thread = {
        let (stderr, duration) = (child.stderr.take().expect("piped"), duration.clone());
        std::thread::spawn(move || {
            let mut tail: Vec<String> = Vec::new();
            for_each_line(stderr, |l| {
                if let Some(d) = parse_duration_us(l) {
                    duration.store(d, Ordering::Relaxed);
                }
                tail.push(l.trim().to_string());
                if tail.len() > 6 {
                    tail.remove(0);
                }
            });
            tail
        })
    };
    let (tx, rx) = channel::<u64>();
    let out_thread = {
        let stdout = child.stdout.take().expect("piped");
        std::thread::spawn(move || {
            for_each_line(stdout, |l| {
                if let Some(us) = parse_progress_us(l) {
                    let _ = tx.send(us);
                }
            });
        })
    };

    let mut killed = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(us) => {
                let d = duration.load(Ordering::Relaxed);
                if d > 0 {
                    progress((us as f32 / d as f32).min(0.99));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if cancel.load(Ordering::Relaxed) && !killed {
            killed = true;
            let _ = child.kill();
        }
    }
    let status = child
        .wait()
        .map_err(|e| ConvertError::Io(format!("ffmpeg beklenemedi: {e}")))?;
    let _ = out_thread.join();
    let tail = err_thread.join().unwrap_or_default();

    if killed || cancel.load(Ordering::Relaxed) && !status.success() {
        return Err(ConvertError::Cancelled);
    }
    if status.success() {
        return Ok(());
    }
    let last = tail.last().map_or("bilinmeyen hata", String::as_str);
    Err(if tail.iter().any(|l| l.contains("matches no streams")) {
        ConvertError::Encode("dosyada bu dönüşüm için gereken akış yok".into())
    } else {
        ConvertError::Encode(last.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(t: Format) -> Vec<String> {
        build_args(Path::new(r"C:\in put.mkv"), Path::new(r"C:\o.tmp"), t)
            .into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn args_are_separate_items_in_order() {
        let a = args(Format::Mp3);
        assert_eq!(&a[..3], ["-nostdin", "-y", "-hide_banner"]);
        let i = a.iter().position(|x| x == "-i").unwrap();
        assert_eq!(a[i + 1], r"C:\in put.mkv"); // spaces stay inside one item
        assert!(a.windows(2).any(|w| w == ["-c:a", "libmp3lame"]));
        assert!(a.windows(2).any(|w| w == ["-progress", "pipe:1"]));
        assert_eq!(a[a.len() - 3..], ["-f", "mp3", r"C:\o.tmp"]);
    }

    #[test]
    fn every_ffmpeg_target_has_codec_and_muxer() {
        let video = [
            Format::Mp4,
            Format::Mkv,
            Format::Avi,
            Format::Webm,
            Format::Mov,
        ];
        let other = [
            Format::Gif,
            Format::Mp3,
            Format::Ogg,
            Format::Aac,
            Format::Wav,
            Format::Flac,
        ];
        for t in video.into_iter().chain(other) {
            assert!(!codec_args(t).is_empty() && muxer(t) != "null", "{t:?}");
        }
        assert!(args(Format::Mp4).contains(&"+faststart".to_string()));
        assert!(args(Format::Webm).contains(&"libvpx-vp9".to_string()));
        assert!(args(Format::Mp4).contains(&"libopenh264".to_string()));
        assert!(!args(Format::Wav).contains(&"-c:v".to_string()));
    }

    #[test]
    fn parses_duration() {
        assert_eq!(
            parse_duration_us("  Duration: 01:02:03.50, start: 0.000000, bitrate: 128 kb/s"),
            Some(3_723_500_000)
        );
        assert_eq!(
            parse_duration_us("Duration: 00:00:02.00, start"),
            Some(2_000_000)
        );
        assert_eq!(parse_duration_us("  Duration: N/A, start: 0.0"), None);
        assert_eq!(parse_duration_us("Stream #0:0: Audio"), None);
    }

    #[test]
    fn parses_progress() {
        assert_eq!(parse_progress_us("out_time_us=1500000"), Some(1_500_000));
        assert_eq!(parse_progress_us("out_time_us=-9223372036854775807"), None);
        assert_eq!(parse_progress_us("out_time_us=N/A"), None);
        assert_eq!(parse_progress_us("out_time_ms=1500000"), None);
        assert_eq!(parse_progress_us("progress=end"), None);
    }

    #[test]
    fn splits_lines_on_cr_and_lf() {
        let mut got = vec![];
        for_each_line(&b"a\rb\n\nc"[..], |l| got.push(l.to_string()));
        assert_eq!(got, ["a", "b", "c"]);
    }

    #[test]
    fn missing_binary_and_bad_target_are_reported() {
        let d = tempfile::tempdir().unwrap();
        let wav = d.path().join("a.wav");
        std::fs::write(&wav, b"RIFF").unwrap();
        let no = AtomicBool::new(false);
        let run = |t| {
            convert_with_ffmpeg(
                &d.path().join("none.exe"),
                &wav,
                t,
                &Options::default(),
                &no,
                |_| {},
            )
        };
        assert!(matches!(run(Format::Mp3), Err(ConvertError::NeedsFfmpeg)));
        assert!(matches!(run(Format::Mp4), Err(ConvertError::Unsupported)));
        assert!(matches!(run(Format::Wav), Err(ConvertError::Unsupported)));
    }
}
