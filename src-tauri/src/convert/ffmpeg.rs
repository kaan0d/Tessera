//! Runs ffmpeg.exe as a child process for video and the audio targets the pure-Rust
//! engine cannot write. Arguments are separate items (no shell), the output goes to a
//! temp file next to the destination and is renamed into place like every other path.

use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use super::{Conflict, ConvertError, Fit, Format, Options, Quality, Resize};
use super::{allows, output, security, uses_ffmpeg};

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

/// Per-job extras on top of `Quality`: transform filters and size-derived bitrates.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extra {
    pub resize: Option<Resize>,
    pub strip: bool,
    /// Average video bitrate in bit/s (replaces the quality-based default).
    pub video_bps: Option<u64>,
    /// Audio bitrate in bit/s (replaces the quality-based default).
    pub audio_bps: Option<u64>,
    /// Video targets: no audio track in the output.
    pub no_audio: bool,
    /// One audio channel.
    pub mono: bool,
    /// Cut: start second and length in seconds (to the end when `None`).
    pub trim_start: f64,
    pub trim_len: Option<f64>,
}

/// ffmpeg scale/crop filters for a resize rule; sides come out even where the rule allows
/// (`video_filter` rounds the rest), which the chroma-subsampled encoders need.
pub fn resize_filter(r: Resize) -> String {
    match r.clamped() {
        Resize::MaxEdge(p) => {
            format!("scale='if(gt(iw,ih),min({p},iw),-2)':'if(gt(iw,ih),-2,min({p},ih))'")
        }
        Resize::Percent(p) => format!("scale=trunc(iw*{p}/200)*2:trunc(ih*{p}/200)*2"),
        Resize::Exact { w, h, fit } => match fit {
            Fit::Contain => format!("scale={w}:{h}:force_original_aspect_ratio=decrease"),
            Fit::Cover => {
                format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}")
            }
            Fit::Stretch => format!("scale={w}:{h}"),
        },
    }
}

/// Video filter chain: the resize rule, then rounding to even sides.
fn video_filter(x: &Extra) -> String {
    let even = "scale=trunc(iw/2)*2:trunc(ih/2)*2";
    match x.resize {
        Some(r) => format!("{},{even}", resize_filter(r)),
        None => even.to_string(),
    }
}

/// Codec arguments per target. Only encoders present in the pinned LGPL build:
/// libopenh264 (H.264), libvpx-vp9, libopus, libvorbis, libmp3lame, native aac/mpeg4/gif.
fn codec_args(t: Format, q: &Quality, x: &Extra) -> Vec<String> {
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let k = |bps: u64| format!("{}k", (bps / 1000).max(1));
    let abr = |default: &str| x.audio_bps.map_or(default.to_string(), k);
    let mut a = match t {
        Format::Mp4 | Format::Mkv | Format::Mov => {
            let mut a = v(&["-map", "0:v:0", "-map", "0:a:0?", "-vf"]);
            a.push(video_filter(x));
            a.extend(v(&["-c:v", "libopenh264", "-b:v"]));
            a.push(x.video_bps.map_or(format!("{}M", q.video_mbps), k));
            a.extend(v(&["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a"]));
            a.push(abr("192k"));
            a
        }
        Format::Avi => {
            let mut a = v(&["-map", "0:v:0", "-map", "0:a:0?", "-vf"]);
            a.push(video_filter(x));
            a.extend(v(&["-c:v", "mpeg4"]));
            match x.video_bps {
                Some(b) => a.extend(["-b:v".to_string(), k(b)]),
                None => a.extend(v(&["-q:v", "4"])),
            }
            a.extend(v(&["-pix_fmt", "yuv420p", "-c:a", "libmp3lame"]));
            match x.audio_bps {
                Some(b) => a.extend(["-b:a".to_string(), k(b)]),
                None => a.extend(v(&["-q:a", "4"])),
            }
            a
        }
        Format::Webm => {
            let mut a = v(&["-map", "0:v:0", "-map", "0:a:0?", "-vf"]);
            a.push(video_filter(x));
            a.extend(v(&["-c:v", "libvpx-vp9"]));
            match x.video_bps {
                Some(b) => a.extend(["-b:v".to_string(), k(b)]),
                None => a.extend(v(&["-crf", "32", "-b:v", "0"])),
            }
            a.extend(v(&[
                "-row-mt",
                "1",
                "-deadline",
                "good",
                "-cpu-used",
                "4",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "libopus",
                "-b:a",
            ]));
            a.push(abr("128k"));
            a
        }
        Format::Gif => {
            let mut a = v(&["-map", "0:v:0", "-an", "-loop", "0", "-vf"]);
            let size = match x.resize {
                Some(r) => resize_filter(r),
                None => format!("scale='min({},iw)':-1:flags=lanczos", q.gif_width),
            };
            a.push(format!(
                "fps={},{size},split[a][b];[a]palettegen[p];[b][p]paletteuse",
                q.gif_fps
            ));
            a
        }
        Format::Mp3 | Format::Ogg => {
            let (codec, qv) = if t == Format::Mp3 {
                ("libmp3lame", q.mp3_q)
            } else {
                ("libvorbis", q.ogg_q)
            };
            let mut a = v(&["-vn", "-map", "0:a:0", "-c:a", codec]);
            match x.audio_bps {
                Some(b) => {
                    // MPEG-1 layer III and libvorbis at 44.1 kHz start around 32 kbit/s;
                    // 22.05 kHz goes lower
                    if b < 32_000 {
                        a.extend(v(&["-ar", "22050"]));
                    }
                    a.extend(["-b:a".to_string(), k(b)]);
                }
                None => a.extend(["-q:a".to_string(), qv.to_string()]),
            }
            a
        }
        Format::Aac => {
            let mut a = v(&["-vn", "-map", "0:a:0", "-c:a", "aac", "-b:a"]);
            a.push(abr("192k"));
            a
        }
        Format::Wav => v(&["-vn", "-map", "0:a:0", "-c:a", "pcm_s16le"]),
        Format::Flac => v(&["-vn", "-map", "0:a:0", "-c:a", "flac"]),
        _ => return vec![],
    };
    if x.no_audio && t.is_video() {
        if let Some(i) = a
            .windows(2)
            .position(|w| w[0] == "-map" && w[1] == "0:a:0?")
        {
            a.drain(i..i + 2);
        }
        a.push("-an".to_string());
    }
    if x.mono && !(t.is_video() && x.no_audio) && !matches!(t, Format::Gif) {
        a.extend(v(&["-ac", "1"]));
    }
    if x.strip {
        a.extend(v(&[
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-fflags",
            "+bitexact",
            "-flags:v",
            "+bitexact",
            "-flags:a",
            "+bitexact",
        ]));
    }
    a
}

/// Full argument list (without the program name).
pub fn build_args(
    input: &Path,
    output: &Path,
    target: Format,
    q: &Quality,
    x: &Extra,
) -> Vec<OsString> {
    let mut a = head_args(input, x);
    a.extend(codec_args(target, q, x).into_iter().map(OsString::from));
    a.extend(["-f", muxer(target)].map(OsString::from));
    if target == Format::Mp4 {
        a.extend(["-movflags", "+faststart"].map(OsString::from));
    }
    a.push(output.into());
    a
}

fn head_args(input: &Path, x: &Extra) -> Vec<OsString> {
    let mut a: Vec<OsString> = [
        "-nostdin",
        "-y",
        "-hide_banner",
        "-nostats",
        "-progress",
        "pipe:1",
        // the input is one local file; a crafted playlist must not pull in anything else
        "-protocol_whitelist",
        "file",
    ]
    .map(OsString::from)
    .into();
    // input options: seeking before -i skips straight to the cut
    if x.trim_start > 0.0 {
        a.extend(["-ss".into(), format!("{:.3}", x.trim_start).into()]);
    }
    if let Some(len) = x.trim_len {
        a.extend(["-t".into(), format!("{len:.3}").into()]);
    }
    a.push("-i".into());
    a.push(input.into());
    a
}

/// First picture of a HEIC/AVIF file as PNG (ffmpeg stitches the tile grid and applies
/// the rotation).
fn still_args(input: &Path, output: &Path) -> Vec<OsString> {
    let mut a = head_args(input, &Extra::default());
    a.extend(
        [
            "-frames:v",
            "1",
            "-update",
            "1",
            "-map_metadata",
            "-1",
            "-c:v",
            "png",
            "-f",
            "image2",
        ]
        .map(OsString::from),
    );
    a.push(output.into());
    a
}

/// What `ffmpeg -i file` reports.
#[derive(Debug, Default, PartialEq)]
pub struct Probe {
    pub duration_us: Option<u64>,
    pub has_audio: bool,
    /// Whole-file bitrate in bit/s ("bitrate: 1511 kb/s").
    pub bitrate_bps: Option<u64>,
}

pub fn parse_probe(stderr: &str) -> Probe {
    let mut p = Probe::default();
    for l in stderr.lines() {
        if let Some(d) = parse_duration_us(l) {
            p.duration_us = Some(d);
            p.bitrate_bps = l
                .split("bitrate:")
                .nth(1)
                .and_then(|r| r.trim().strip_suffix("kb/s"))
                .and_then(|k| k.trim().parse::<u64>().ok())
                .map(|k| k * 1000);
        }
        if l.contains("Stream #") && l.contains(": Audio:") {
            p.has_audio = true;
        }
    }
    p
}

fn probe(ffmpeg: &Path, src: &Path) -> Result<Probe, ConvertError> {
    let out = Command::new(ffmpeg)
        .current_dir(ffmpeg.parent().unwrap_or(Path::new(".")))
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(src)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| ConvertError::FfmpegStart(e.to_string()))?;
    Ok(parse_probe(&String::from_utf8_lossy(&out.stderr)))
}

/// Encodes per file when the result is over the size limit.
const RETRIES: usize = 7;
/// Longest side a video steps down to, one retry after another, when a size limit is not met
/// at full size (an encoder starved of bits at 720p or more overshoots its target).
const SHRINK_EDGES: [u32; 3] = [1280, 854, 640];
/// Share of the byte budget left for the payload: container overhead and rate-control error.
const HEADROOM: f64 = 0.90;
/// A source with fewer bits than this still gets this much video bitrate (bit/s).
const MIN_COPY_BPS: u64 = 250_000;
/// Below this a video is not worth producing (bit/s).
const MIN_VIDEO_BPS: f64 = 30_000.0;
/// Below this an audio track is not worth producing (bit/s).
const MIN_AUDIO_BPS: f64 = 24_000.0;
/// libvorbis floors: stereo below the first fails to open, mono below the second.
const OGG_STEREO_MIN_BPS: f64 = 48_000.0;
const OGG_MONO_MIN_BPS: f64 = 32_000.0;

/// Bitrates that should land `target` under `max_bytes` for a clip of `duration_us`.
/// Never above the quality defaults (a big budget keeps normal quality). Audio gets a fixed
/// share when there is a video; `TargetSize` when what is left is unusably small.
pub fn plan_bitrates(
    max_bytes: u64,
    duration_us: u64,
    target: Format,
    has_audio: bool,
    q: &Quality,
) -> Result<Extra, ConvertError> {
    if duration_us == 0 {
        return Err(ConvertError::TargetSize);
    }
    let total = max_bytes as f64 * 8.0 * 1e6 / duration_us as f64 * HEADROOM;
    let mut x = Extra::default();
    if target.is_video() {
        let audio = if !has_audio {
            0.0
        } else if total >= 300_000.0 {
            128_000.0
        } else {
            48_000.0
        };
        let video = total - audio;
        if video < MIN_VIDEO_BPS {
            return Err(ConvertError::TargetSize);
        }
        x.video_bps = Some(video.min(f64::from(q.video_mbps) * 1e6) as u64);
        x.audio_bps = has_audio.then_some(audio as u64);
    } else if matches!(target, Format::Mp3 | Format::Ogg | Format::Aac) {
        if total < MIN_AUDIO_BPS || (target == Format::Ogg && total < OGG_MONO_MIN_BPS) {
            return Err(ConvertError::TargetSize);
        }
        // libvorbis refuses stereo this small; one channel still sounds fine
        x.mono = target == Format::Ogg && total < OGG_STEREO_MIN_BPS;
        x.audio_bps = Some(total.min(192_000.0) as u64);
    }
    Ok(x)
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

/// Lowers the bitrates of `x` by `f` (< 1) for a retry; `TargetSize` when they become unusable.
fn shrink_bitrates(x: &mut Extra, f: f64) -> Result<(), ConvertError> {
    let cut = |b: &mut Option<u64>, min: f64| -> Result<(), ConvertError> {
        if let Some(v) = b {
            let n = *v as f64 * f;
            if n < min {
                return Err(ConvertError::TargetSize);
            }
            *v = n as u64;
        }
        Ok(())
    };
    cut(&mut x.video_bps, MIN_VIDEO_BPS)?;
    cut(&mut x.audio_bps, MIN_AUDIO_BPS)
}

/// Converts `input` with ffmpeg. `cancel` kills the child and removes the temp file.
/// `progress` receives 0.0..=1.0 (stays at 0 when the duration is unknown).
///
/// HEIC/AVIF sources: ffmpeg writes the first picture to a temporary PNG, then the
/// pure-Rust pipeline applies orientation/resize/size limit and encodes the target.
pub fn convert_with_ffmpeg(
    ffmpeg: &Path,
    input: &Path,
    target: Format,
    opts: &Options,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    let opts = &opts.clamped();
    let (src, from) = security::validate_input(input)?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !allows(ext, from, target, opts) || !(uses_ffmpeg(from, target) || opts.ffmpeg_only(from)) {
        return Err(ConvertError::Unsupported);
    }
    if !ffmpeg.is_file() {
        return Err(ConvertError::NeedsFfmpeg);
    }
    let dest = output::plan(&src, target.ext(), &opts.output)?;
    progress(0.0);
    if opts.already_fits(&src, from, target) {
        let done = opts.copy_as_result(&src, &dest)?;
        progress(1.0);
        return Ok(done);
    }
    if matches!(from, Format::Heic | Format::Avif) {
        output::write_atomic(&dest, |w| {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let png = dest.with_file_name(format!(".still.{nanos}.png"));
            let r = run_child(ffmpeg, still_args(&src, &png), None, cancel, &mut progress)
                .and_then(|()| super::encode_decoded(&png, target, true, opts, w));
            let _ = std::fs::remove_file(&png);
            r
        })?;
    } else {
        let mut x = Extra {
            resize: opts.resize,
            strip: opts.strip_metadata,
            no_audio: opts.no_audio,
            mono: opts.mono,
            audio_bps: opts.audio_kbps.map(|k| u64::from(k) * 1000),
            ..Extra::default()
        };
        x.trim_start = f64::from(opts.trim_start.unwrap_or(0.0));
        if let Some(end) = opts.trim_end {
            // an end at or before the start keeps nothing
            let len = f64::from(end) - x.trim_start;
            if len <= 0.0 {
                return Err(ConvertError::Unsupported);
            }
            x.trim_len = Some(len);
        }
        if x.trim_start > 0.0 {
            // a start past the end would leave an empty file that looks like a success
            if let Some(d) = probe(ffmpeg, &src)?.duration_us
                && (x.trim_start * 1e6) as u64 >= d
            {
                return Err(ConvertError::TrimOutside);
            }
        }
        if let Some(max) = opts.max_bytes
            && (target.is_video() || matches!(target, Format::Mp3 | Format::Ogg | Format::Aac))
        {
            let p = probe(ffmpeg, &src)?;
            let mut d = p.duration_us.ok_or(ConvertError::TargetSize)?;
            // only the kept part counts against the size limit
            d = kept_us(d, x.trim_start, x.trim_len);
            let b = plan_bitrates(max, d, target, p.has_audio && !opts.no_audio, &opts.quality)?;
            (x.video_bps, x.audio_bps) = (b.video_bps, b.audio_bps);
            x.mono |= b.mono;
        }
        // a re-encode never needs more bits than the source had: small clips stay small
        if target.is_video()
            && let Some(src_bps) = probe(ffmpeg, &src)?.bitrate_bps
        {
            let cap = src_bps.max(MIN_COPY_BPS);
            let cur = x
                .video_bps
                .unwrap_or(u64::from(opts.quality.video_mbps) * 1_000_000);
            x.video_bps = Some(cur.min(cap));
        }
        let (mut q, mut shrunk) = (opts.quality, 0);
        output::write_via_temp(&dest, |tmp| {
            for attempt in 0..RETRIES {
                let args = build_args(&src, tmp, target, &q, &x);
                run_child(
                    ffmpeg,
                    args,
                    Some((x.trim_start, x.trim_len)),
                    cancel,
                    &mut progress,
                )?;
                let Some(max) = opts.max_bytes else {
                    return Ok(());
                };
                let size = std::fs::metadata(tmp)?.len();
                if size <= max {
                    return Ok(());
                }
                if attempt + 1 == RETRIES {
                    break;
                }
                // a GIF has no bitrate: fewer frames and a smaller picture instead
                if target == Format::Gif {
                    if q.gif_fps <= 5 && q.gif_width <= 120 {
                        break;
                    }
                    q.gif_fps = (f32::from(q.gif_fps) * 0.75).max(5.0) as u8;
                    q.gif_width = (f32::from(q.gif_width) * 0.8).max(120.0) as u16;
                    continue;
                }
                if x.video_bps.is_none() && x.audio_bps.is_none() {
                    break;
                }
                // rate control overshot: a video first gets smaller at the same bitrate (unless
                // the user chose its size), then the bitrate goes down
                if target.is_video()
                    && opts.resize.is_none()
                    && let Some(&edge) = SHRINK_EDGES.get(shrunk)
                {
                    x.resize = Some(Resize::MaxEdge(edge));
                    shrunk += 1;
                    continue;
                }
                shrink_bitrates(&mut x, max as f64 / size as f64 * 0.9)?;
            }
            Err(ConvertError::TargetSize)
        })?;
    }
    if opts.output.conflict == Conflict::ReplaceOriginal {
        output::recycle(&src)?;
    }
    progress(1.0);
    Ok(dest)
}

/// Lossy WebP of `img` (already oriented/resized, no metadata) through libwebp, as bytes.
/// The picture goes in as a fast-compressed temporary PNG; both temp files are removed.
pub fn encode_webp(
    ffmpeg: &Path,
    img: &image::DynamicImage,
    quality: u8,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<u8>, ConvertError> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let base = std::env::temp_dir().join(format!("tessera-webp-{}-{nanos}", std::process::id()));
    let (png, webp) = (base.with_extension("png"), base.with_extension("webp"));
    let run = || -> Result<Vec<u8>, ConvertError> {
        let file = std::fs::File::create(&png)?;
        img.write_with_encoder(PngEncoder::new_with_quality(
            std::io::BufWriter::new(file),
            CompressionType::Fast,
            FilterType::NoFilter,
        ))?;
        let mut a = head_args(&png, &Extra::default());
        a.extend(
            [
                "-frames:v",
                "1",
                "-map_metadata",
                "-1",
                "-c:v",
                "libwebp",
                "-quality",
            ]
            .map(OsString::from),
        );
        a.push(quality.clamp(1, 100).to_string().into());
        a.extend(["-f", "webp"].map(OsString::from));
        a.push(webp.clone().into());
        let never = AtomicBool::new(false);
        run_child(ffmpeg, a, None, cancel.unwrap_or(&never), &mut |_| {})?;
        let bytes = std::fs::read(&webp)?;
        if bytes.is_empty() {
            return Err(ConvertError::EmptyOutput);
        }
        Ok(bytes)
    };
    let r = run();
    let _ = std::fs::remove_file(&png);
    let _ = std::fs::remove_file(&webp);
    r
}

/// Length in microseconds of the part that is kept after a cut: `total` minus the start,
/// shortened to `len` seconds when given.
pub fn kept_us(total: u64, start: f64, len: Option<f64>) -> u64 {
    total
        .saturating_sub((start * 1e6) as u64)
        .min(len.map_or(u64::MAX, |l| (l * 1e6) as u64))
}

/// `cut` is (start, length) in seconds: ffmpeg reports the input's full length, but with the
/// cut before `-i` its progress counts only the kept part, so that is what 100% means.
fn run_child(
    ffmpeg: &Path,
    args: Vec<OsString>,
    cut: Option<(f64, Option<f64>)>,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(f32),
) -> Result<(), ConvertError> {
    let mut child = Command::new(ffmpeg)
        // not the caller's folder: nothing in there should be picked up by the child
        .current_dir(ffmpeg.parent().unwrap_or(Path::new(".")))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| ConvertError::FfmpegStart(e.to_string()))?;

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
                let total = duration.load(Ordering::Relaxed);
                let d = cut.map_or(total, |(start, len)| kept_us(total, start, len));
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
        .map_err(|e| ConvertError::FfmpegWait(e.to_string()))?;
    let _ = out_thread.join();
    let tail = err_thread.join().unwrap_or_default();

    if killed || cancel.load(Ordering::Relaxed) && !status.success() {
        return Err(ConvertError::Cancelled);
    }
    if status.success() {
        return Ok(());
    }
    Err(if tail.iter().any(|l| l.contains("matches no streams")) {
        ConvertError::NoStream
    } else {
        ConvertError::Encode(tail.last().cloned().unwrap_or_default())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(t: Format) -> Vec<String> {
        build_args(
            Path::new(r"C:\in put.mkv"),
            Path::new(r"C:\o.tmp"),
            t,
            &Quality::default(),
            &Extra::default(),
        )
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
            assert!(
                !codec_args(t, &Quality::default(), &Extra::default()).is_empty()
                    && muxer(t) != "null",
                "{t:?}"
            );
        }
        assert!(args(Format::Mp4).contains(&"+faststart".to_string()));
        assert!(args(Format::Webm).contains(&"libvpx-vp9".to_string()));
        assert!(args(Format::Mp4).contains(&"libopenh264".to_string()));
        assert!(!args(Format::Wav).contains(&"-c:v".to_string()));
    }

    #[test]
    fn quality_reaches_args() {
        let q = Quality {
            mp3_q: 7,
            ogg_q: 9,
            video_mbps: 20,
            gif_fps: 25,
            gif_width: 800,
            ..Quality::default()
        };
        let a = |t| codec_args(t, &q, &Extra::default()).join(" ");
        assert!(a(Format::Mp3).ends_with("-q:a 7"));
        assert!(a(Format::Ogg).ends_with("-q:a 9"));
        assert!(a(Format::Mp4).contains("-b:v 20M"));
        assert!(a(Format::Gif).contains("fps=25,scale='min(800,iw)'"));
    }

    #[test]
    fn resize_and_strip_reach_args() {
        let ex = |r| Extra {
            resize: Some(r),
            ..Extra::default()
        };
        let a = |t, x: &Extra| codec_args(t, &Quality::default(), x).join(" ");
        let m = a(Format::Mp4, &ex(Resize::MaxEdge(720)));
        assert!(m.contains("scale='if(gt(iw,ih),min(720,iw),-2)':'if(gt(iw,ih),-2,min(720,ih))',scale=trunc(iw/2)*2"));
        assert!(a(Format::Webm, &ex(Resize::Percent(50))).contains("scale=trunc(iw*50/200)*2"));
        let cover = Resize::Exact {
            w: 100,
            h: 50,
            fit: Fit::Cover,
        };
        assert!(a(Format::Avi, &ex(cover)).contains("increase,crop=100:50"));
        assert!(a(Format::Gif, &ex(Resize::MaxEdge(300))).contains("fps=12,scale='if(gt"));
        assert!(!a(Format::Mp3, &ex(Resize::MaxEdge(300))).contains("scale"));
        let strip = Extra {
            strip: true,
            ..Extra::default()
        };
        for t in [Format::Mp4, Format::Mp3, Format::Gif, Format::Wav] {
            let s = a(t, &strip);
            assert!(
                s.contains("-map_metadata -1") && s.contains("-fflags +bitexact"),
                "{t:?}"
            );
        }
        assert!(!a(Format::Mp4, &Extra::default()).contains("map_metadata"));
    }

    #[test]
    fn bitrates_follow_duration_and_size() {
        let q = Quality::default();
        // 1 MB over 100 s: 80 kbit/s total, 90 % usable -> 72k; 48k audio, 24k video is too low
        assert!(matches!(
            plan_bitrates(1_000_000, 100_000_000, Format::Mp4, true, &q),
            Err(ConvertError::TargetSize)
        ));
        // 10 MB over 100 s: 800 kbit/s * 0.9 = 720k, 128k audio
        let x = plan_bitrates(10_000_000, 100_000_000, Format::Mp4, true, &q).unwrap();
        assert_eq!((x.video_bps, x.audio_bps), (Some(592_000), Some(128_000)));
        // no audio track: everything goes to video
        let x = plan_bitrates(10_000_000, 100_000_000, Format::Webm, false, &q).unwrap();
        assert_eq!((x.video_bps, x.audio_bps), (Some(720_000), None));
        // a huge budget never exceeds the quality default
        let x = plan_bitrates(u64::MAX / 1000, 1_000_000, Format::Mp4, true, &q).unwrap();
        assert_eq!(x.video_bps, Some(6_000_000));
        // audio targets
        let x = plan_bitrates(1_000_000, 100_000_000, Format::Mp3, true, &q).unwrap();
        assert_eq!(x.audio_bps, Some(72_000));
        let x = plan_bitrates(100_000_000, 100_000_000, Format::Ogg, true, &q).unwrap();
        assert_eq!(x.audio_bps, Some(192_000));
        assert!(plan_bitrates(100_000, 100_000_000, Format::Mp3, true, &q).is_err());
        assert!(plan_bitrates(1000, 0, Format::Mp3, true, &q).is_err());
        // bitrate flags land in the args
        let a = codec_args(
            Format::Mp3,
            &q,
            &Extra {
                audio_bps: Some(28_000),
                ..Extra::default()
            },
        )
        .join(" ");
        assert!(a.contains("-ar 22050") && a.ends_with("-b:a 28k"));
        let a = codec_args(
            Format::Webm,
            &q,
            &Extra {
                video_bps: Some(300_000),
                audio_bps: Some(64_000),
                ..Extra::default()
            },
        )
        .join(" ");
        assert!(a.contains("-b:v 300k") && !a.contains("-crf") && a.ends_with("-b:a 64k"));
    }

    #[test]
    fn trim_comes_before_input_and_mono_after() {
        let x = Extra {
            mono: true,
            trim_start: 2.5,
            trim_len: Some(10.0),
            ..Extra::default()
        };
        let a: Vec<String> = build_args(
            Path::new("in.mp4"),
            Path::new("out.mp3"),
            Format::Mp3,
            &Quality::default(),
            &x,
        )
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
        let pos = |k: &str| a.iter().position(|s| s == k).unwrap();
        assert_eq!(
            (a[pos("-ss") + 1].as_str(), a[pos("-t") + 1].as_str()),
            ("2.500", "10.000")
        );
        assert!(pos("-ss") < pos("-i") && pos("-t") < pos("-i") && pos("-ac") > pos("-i"));
        // no cut: no seek flags; a GIF stays without -ac
        let a = build_args(
            Path::new("in.mp4"),
            Path::new("out.gif"),
            Format::Gif,
            &Quality::default(),
            &Extra {
                mono: true,
                ..Extra::default()
            },
        );
        assert!(!a.iter().any(|s| s == "-ss" || s == "-ac"));
    }

    #[test]
    fn probe_reads_duration_and_audio() {
        let p = parse_probe(
            "  Duration: 00:00:06.02, start: 0.0
  Stream #0:0: Video: h264
  Stream #0:1: Audio: aac
",
        );
        assert_eq!(
            p,
            Probe {
                duration_us: Some(6_020_000),
                has_audio: true,
                bitrate_bps: None
            }
        );
        let p = parse_probe(
            "  Duration: 00:00:10.00, start: 0.000000, bitrate: 1511 kb/s
",
        );
        assert_eq!(p.bitrate_bps, Some(1_511_000));
        let p = parse_probe(
            "  Duration: N/A
  Stream #0:0: Video: h264
",
        );
        assert_eq!(p, Probe::default());
    }

    #[test]
    fn no_audio_drops_the_audio_track_of_video_targets() {
        let x = Extra {
            no_audio: true,
            ..Extra::default()
        };
        for t in [Format::Mp4, Format::Webm, Format::Avi] {
            let args = codec_args(t, &Quality::default(), &x);
            assert!(args.contains(&"-an".to_string()), "{t:?}");
            assert!(!args.contains(&"0:a:0?".to_string()), "{t:?}");
        }
        let kept = codec_args(Format::Mp4, &Quality::default(), &Extra::default());
        assert!(kept.contains(&"0:a:0?".to_string()));
    }

    #[test]
    fn retry_lowers_bitrates_until_unusable() {
        let mut x = Extra {
            video_bps: Some(100_000),
            audio_bps: Some(48_000),
            ..Extra::default()
        };
        shrink_bitrates(&mut x, 0.5).unwrap();
        assert_eq!((x.video_bps, x.audio_bps), (Some(50_000), Some(24_000)));
        assert!(shrink_bitrates(&mut x, 0.5).is_err());
    }

    #[test]
    fn kept_length_follows_the_cut() {
        assert_eq!(kept_us(60_000_000, 0.0, None), 60_000_000);
        assert_eq!(kept_us(60_000_000, 10.0, None), 50_000_000);
        assert_eq!(kept_us(60_000_000, 10.0, Some(5.0)), 5_000_000);
        assert_eq!(kept_us(60_000_000, 58.0, Some(30.0)), 2_000_000);
        assert_eq!(kept_us(60_000_000, 90.0, None), 0);
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
