//! Screen recording core: ffmpeg's gdigrab writes an H.264 MP4 of one screen area, no sound.
//! Pure parts (rectangles, DPI, arguments, limits) are tested; `start` spawns the child.
//! The windows around it (area picker, control bar, red frame) are in `capture_ui.rs`.

use std::ffi::OsString;
use std::io::Write;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::convert::Format;
use crate::ffmpeg::FfError;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Smallest recorded side in physical pixels.
pub const MIN_SIDE: u32 = 64;
/// A recording does not start with less free space than this (bytes).
pub const MIN_FREE: u64 = 1 << 30;
/// ffmpeg stops the file itself at this size (bytes).
pub const MAX_BYTES: u64 = 2 << 30;
/// How long `q` gets to finish the file before the process tree is killed.
const STOP_WAIT: Duration = Duration::from_secs(3);
/// Red frame: this many physical pixels of gap between the recorded area and the frame, and the
/// frame's own thickness. The gap keeps the frame out of the picture.
pub const FRAME_GAP: u32 = 3;
pub const FRAME_W: u32 = 2;

/// Pixels on the virtual desktop (physical), the unit gdigrab works in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn right(&self) -> i32 {
        self.x + self.w as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h as i32
    }
}

/// What the user can change (Settings > Advanced).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureSettings {
    /// Where recordings go; None is `Videos\Tessera`.
    pub dir: Option<PathBuf>,
    /// 10, 15 or 30.
    pub fps: u8,
    pub cursor: bool,
    /// Longest recording, 1..=30 minutes.
    pub max_minutes: u8,
    /// "mp4", "gif" or "webm".
    pub format: String,
    /// Global shortcut that starts and stops a recording (off by default).
    pub hotkey: bool,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        CaptureSettings {
            dir: None,
            fps: 15,
            cursor: true,
            max_minutes: 10,
            format: "mp4".into(),
            hotkey: false,
        }
    }
}

/// The global shortcut, fixed: Ctrl+Alt+R.
pub const HOTKEY: &str = "Ctrl+Alt+R";

impl CaptureSettings {
    pub fn normalized(mut self) -> Self {
        self.fps = match self.fps {
            0..=12 => 10,
            13..=22 => 15,
            _ => 30,
        };
        self.max_minutes = self.max_minutes.clamp(1, 30);
        self.format = self.format.trim().to_lowercase();
        if !["mp4", "gif", "webm"].contains(&self.format.as_str()) {
            self.format = "mp4".into();
        }
        if self.dir.as_ref().is_some_and(|d| d.as_os_str().is_empty()) {
            self.dir = None;
        }
        self
    }

    pub fn target(&self) -> Format {
        match self.format.as_str() {
            "gif" => Format::Gif,
            "webm" => Format::Webm,
            _ => Format::Mp4,
        }
    }

    pub fn max_secs(&self) -> u64 {
        u64::from(self.max_minutes.clamp(1, 30)) * 60
    }

    pub fn out_dir(&self) -> PathBuf {
        self.dir.clone().unwrap_or_else(default_dir)
    }
}

/// `Videos\Tessera`.
pub fn default_dir() -> PathBuf {
    videos_dir().join(crate::app_info::APP_NAME)
}

fn videos_dir() -> PathBuf {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    // SAFETY: the returned buffer is copied and then freed as the API requires.
    let known = unsafe {
        SHGetKnownFolderPath(&FOLDERID_Videos, KF_FLAG_DEFAULT, None)
            .ok()
            .and_then(|p| {
                let s = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as *const _));
                s
            })
    };
    known.map(PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Videos")
    })
}

/// `capture-2026-11-08-142530.mp4` style stem.
pub fn file_stem() -> String {
    // SAFETY: GetLocalTime only fills the struct it is given.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "capture-{:04}-{:02}-{:02}-{:02}{:02}{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// Free bytes for the current user on the drive of `dir` (or its nearest existing parent).
pub fn free_bytes(dir: &Path) -> Option<u64> {
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let existing = dir.ancestors().find(|p| p.is_dir())?;
    let wide = windows::core::HSTRING::from(existing.as_os_str());
    let mut free = 0u64;
    // SAFETY: `free` outlives the call; the other outputs are not wanted.
    unsafe { GetDiskFreeSpaceExW(&wide, Some(&mut free), None, None).ok()? };
    Some(free)
}

/// Rounds down to an even number (yuv420p needs even sides).
pub fn even_down(n: u32) -> u32 {
    n & !1
}

/// A picked area in a window's CSS pixels to desktop pixels. `origin` is the window's top-left
/// on the desktop (physical) and `scale` physical pixels per CSS pixel on that screen. Negative
/// sizes (a drag up and to the left) become positive.
pub fn to_physical(origin: (i32, i32), scale: f64, sel: (f64, f64, f64, f64)) -> Rect {
    let (x, y, w, h) = sel;
    let (x0, x1) = (x.min(x + w), x.max(x + w));
    let (y0, y1) = (y.min(y + h), y.max(y + h));
    let px = |v: f64| (v * scale).round() as i32;
    Rect {
        x: origin.0 + px(x0),
        y: origin.1 + px(y0),
        w: (px(x1) - px(x0)).max(0) as u32,
        h: (px(y1) - px(y0)).max(0) as u32,
    }
}

/// The scale to use for a window: the one the system reports, unless the page's real width says
/// otherwise (a webview that has not caught up with a monitor change).
pub fn effective_scale(reported: f64, monitor_w: u32, inner_css_w: f64) -> f64 {
    if inner_css_w >= 1.0 {
        let real = f64::from(monitor_w) / inner_css_w;
        if (real - reported).abs() > 0.02 {
            return real;
        }
    }
    reported
}

/// `r` cut to `bounds`, sides even, at least `MIN_SIDE`; None when nothing usable is left.
pub fn fit_rect(r: Rect, bounds: Rect) -> Option<Rect> {
    let x0 = r.x.max(bounds.x);
    let y0 = r.y.max(bounds.y);
    let x1 = r.right().min(bounds.right());
    let y1 = r.bottom().min(bounds.bottom());
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (w, h) = (even_down((x1 - x0) as u32), even_down((y1 - y0) as u32));
    (w >= MIN_SIDE && h >= MIN_SIDE).then_some(Rect { x: x0, y: y0, w, h })
}

/// "x,y,w,h" (x and y may be negative on a second monitor to the left or above).
pub fn parse_region(s: &str) -> Option<Rect> {
    let v: Vec<i32> = s
        .split(',')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    let [x, y, w, h] = v[..] else { return None };
    (w > 0 && h > 0).then_some(Rect {
        x,
        y,
        w: w as u32,
        h: h as u32,
    })
}

/// Video bitrate (bit/s) that keeps screen text readable: about 0.15 bit per pixel and frame,
/// between 1 and 12 Mbit/s.
pub fn bitrate_bps(w: u32, h: u32, fps: u8) -> u64 {
    (u64::from(w) * u64::from(h) * u64::from(fps) * 15 / 100).clamp(1_000_000, 12_000_000)
}

/// ffmpeg arguments (without the program name) for recording `r` into `out`.
pub fn build_args(r: Rect, s: &CaptureSettings, max_secs: u64, out: &Path) -> Vec<OsString> {
    let s = s.clone().normalized();
    let mut a: Vec<OsString> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "gdigrab",
        "-framerate",
    ]
    .map(OsString::from)
    .into();
    a.push(s.fps.to_string().into());
    a.extend(["-draw_mouse", if s.cursor { "1" } else { "0" }].map(OsString::from));
    a.extend(["-offset_x".into(), r.x.to_string().into()]);
    a.extend(["-offset_y".into(), r.y.to_string().into()]);
    a.extend(["-video_size".into(), format!("{}x{}", r.w, r.h).into()]);
    a.extend(["-i", "desktop"].map(OsString::from));
    // output side: the safety limits end the file cleanly on their own
    a.extend(["-t".into(), max_secs.to_string().into()]);
    a.extend(["-fs".into(), MAX_BYTES.to_string().into()]);
    a.extend(["-an", "-c:v", "libopenh264", "-b:v"].map(OsString::from));
    a.push(format!("{}k", bitrate_bps(r.w, r.h, s.fps) / 1000).into());
    a.extend(
        [
            "-pix_fmt",
            "yuv420p",
            "-f",
            "mp4",
            "-movflags",
            "+faststart",
        ]
        .map(OsString::from),
    );
    a.push(out.into());
    a
}

/// Where the control bar goes: outside the red frame, below the area, else above, right, left.
/// `bounds` is the screen the area is on; None when it fits nowhere.
pub fn place_bar(region: Rect, bar: (u32, u32), bounds: Rect) -> Option<(i32, i32)> {
    let gap = (FRAME_GAP + FRAME_W + 8) as i32;
    let (bw, bh) = (bar.0 as i32, bar.1 as i32);
    let cx = (region.x + region.w as i32 / 2 - bw / 2)
        .clamp(bounds.x, (bounds.right() - bw).max(bounds.x));
    let cy = region.y;
    let cands = [
        (cx, region.bottom() + gap),
        (cx, region.y - gap - bh),
        (region.right() + gap, cy),
        (region.x - gap - bw, cy),
    ];
    cands.into_iter().find(|&(x, y)| {
        x >= bounds.x && y >= bounds.y && x + bw <= bounds.right() && y + bh <= bounds.bottom()
    })
}

/// Fallback when the bar fits outside nowhere (a whole-screen recording): bottom centre inside
/// the area. The bar window is excluded from capture, so it does not show up in the video.
pub fn place_bar_inside(region: Rect, bar: (u32, u32)) -> (i32, i32) {
    let (bw, bh) = (bar.0 as i32, bar.1 as i32);
    let x = (region.x + (region.w as i32 - bw) / 2).max(region.x);
    let y = (region.bottom() - bh - 24).max(region.y);
    (x, y)
}

/// The red frame's window: `r` grown by gap and thickness on every side, and the hole (relative
/// to that window) that is cut out so the window never covers the recorded area.
pub fn frame_window(r: Rect) -> (Rect, Rect) {
    let g = FRAME_GAP as i32;
    let m = g + FRAME_W as i32;
    let outer = Rect {
        x: r.x - m,
        y: r.y - m,
        w: r.w + 2 * m as u32,
        h: r.h + 2 * m as u32,
    };
    let hole = Rect {
        x: FRAME_W as i32,
        y: FRAME_W as i32,
        w: r.w + 2 * g as u32,
        h: r.h + 2 * g as u32,
    };
    (outer, hole)
}

fn error(key: &'static str, detail: impl std::fmt::Display) -> FfError {
    FfError::with(key, detail)
}

/// All recording children belong to one job object: when this process ends in any way, Windows
/// ends them too, so no ffmpeg keeps grabbing the screen.
fn job() -> Option<usize> {
    use windows::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    static JOB: OnceLock<Option<usize>> = OnceLock::new();
    *JOB.get_or_init(|| {
        // SAFETY: plain job object calls; the handle is kept for the life of the process.
        unsafe {
            let h = CreateJobObjectW(None, None).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .ok()?;
            Some(h.0 as usize)
        }
    })
}

fn join_job(child: &Child) {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::AssignProcessToJobObject;
    if let Some(j) = job() {
        // SAFETY: both handles are valid; failure only means no orphan protection.
        unsafe {
            let _ = AssignProcessToJobObject(
                HANDLE(j as *mut _),
                HANDLE(child.as_raw_handle() as *mut _),
            );
        }
    }
}

/// A running recording.
pub struct Recording {
    child: Child,
    stdin: Option<ChildStdin>,
    started: Instant,
    part: PathBuf,
    dest: PathBuf,
    tail: Arc<Mutex<Vec<String>>>,
    pub rect: Rect,
    done: bool,
}

/// Starts recording `r` into `<dir>\<stem>.mp4` (written as `.part` and renamed when it is
/// finished). Refuses with too little free space.
pub fn start(
    ffmpeg: &Path,
    r: Rect,
    s: &CaptureSettings,
    dir: &Path,
    stem: &str,
) -> Result<Recording, FfError> {
    if r.w < MIN_SIDE || r.h < MIN_SIDE {
        return Err(error("cap.bad_region", ""));
    }
    std::fs::create_dir_all(dir).map_err(|e| error("cap.dir", e))?;
    if free_bytes(dir).is_some_and(|f| f < MIN_FREE) {
        return Err(error("cap.low_disk", ""));
    }
    let dest = dir.join(format!("{stem}.mp4"));
    let part = dir.join(format!("{stem}.mp4.part"));
    let mut child = Command::new(ffmpeg)
        .current_dir(ffmpeg.parent().unwrap_or(Path::new(".")))
        .args(build_args(r, s, s.max_secs(), &part))
        // ffmpeg.exe is not DPI aware; without this gdigrab reads scaled coordinates
        .env("__COMPAT_LAYER", "HighDpiAware")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| error("cap.start", e))?;
    join_job(&child);
    let tail = Arc::new(Mutex::new(Vec::<String>::new()));
    if let Some(stderr) = child.stderr.take() {
        let tail = tail.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for l in std::io::BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
            {
                let mut t = tail.lock().unwrap();
                t.push(l.trim().to_string());
                if t.len() > 4 {
                    t.remove(0);
                }
            }
        });
    }
    let stdin = child.stdin.take();
    Ok(Recording {
        child,
        stdin,
        started: Instant::now(),
        part,
        dest,
        tail,
        rect: r,
        done: false,
    })
}

impl Recording {
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Bytes written so far.
    pub fn size(&self) -> u64 {
        std::fs::metadata(&self.part).map_or(0, |m| m.len())
    }

    /// ffmpeg has ended on its own (time or size limit reached, or it failed).
    pub fn has_ended(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_some()
    }

    fn last_error(&self) -> String {
        self.tail
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }

    /// Asks ffmpeg to finish with `q`; kills the process tree after `STOP_WAIT`. True when it
    /// ended by itself.
    fn finish_child(&mut self) -> bool {
        if let Some(mut si) = self.stdin.take() {
            let _ = si.write_all(b"q");
            let _ = si.flush();
        }
        let t0 = Instant::now();
        while t0.elapsed() < STOP_WAIT {
            if self.child.try_wait().ok().flatten().is_some() {
                self.done = true;
                return true;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        crate::convert::office::kill_tree(self.child.id());
        let _ = self.child.wait();
        self.done = true;
        false
    }

    /// Ends the recording and moves the file into place. A file ffmpeg had to be killed for has
    /// no index at the end and is removed.
    pub fn stop(mut self) -> Result<PathBuf, FfError> {
        let clean = self.finish_child();
        std::thread::sleep(Duration::from_millis(50)); // let the stderr reader drain
        if !clean {
            let _ = std::fs::remove_file(&self.part);
            return Err(error("cap.killed", ""));
        }
        if self.size() == 0 {
            let _ = std::fs::remove_file(&self.part);
            return Err(error("cap.failed", self.last_error()));
        }
        std::fs::rename(&self.part, &self.dest).map_err(|e| error("cap.failed", e))?;
        Ok(self.dest.clone())
    }

    /// Ends the recording and throws the file away.
    pub fn cancel(mut self) {
        crate::convert::office::kill_tree(self.child.id());
        let _ = self.child.wait();
        self.done = true;
        let _ = std::fs::remove_file(&self.part);
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        if !self.done {
            crate::convert::office::kill_tree(self.child.id());
            let _ = self.child.wait();
            let _ = std::fs::remove_file(&self.part);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: fn(i32, i32, u32, u32) -> Rect = |x, y, w, h| Rect { x, y, w, h };

    #[test]
    fn dpi_conversion_matrix() {
        // 100/125/150/200 percent: a 400x300 CSS box at (100, 50) on a screen at origin
        for (scale, want) in [
            (1.0, R(100, 50, 400, 300)),
            (1.25, R(125, 63, 500, 375)),
            (1.5, R(150, 75, 600, 450)),
            (2.0, R(200, 100, 800, 600)),
        ] {
            assert_eq!(
                to_physical((0, 0), scale, (100.0, 50.0, 400.0, 300.0)),
                want,
                "{scale}"
            );
        }
        // second screen (1920x1080 at x = 2560, 125 %): the origin is added after scaling
        assert_eq!(
            to_physical((2560, 0), 1.25, (80.0, 40.0, 160.0, 80.0)),
            R(2660, 50, 200, 100)
        );
        // a screen to the left: negative origin
        assert_eq!(
            to_physical((-1920, 0), 1.0, (10.0, 20.0, 30.0, 40.0)),
            R(-1910, 20, 30, 40)
        );
        // dragged up and to the left
        assert_eq!(
            to_physical((0, 0), 1.0, (100.0, 100.0, -40.0, -30.0)),
            R(60, 70, 40, 30)
        );
    }

    #[test]
    fn mixed_dpi_uses_the_real_page_width() {
        // reported 1.0 but the 2560 px screen shows a 2048 px wide page: 125 % in truth
        assert!((effective_scale(1.0, 2560, 2048.0) - 1.25).abs() < 1e-9);
        // consistent values and an unknown width keep what the system said
        assert_eq!(effective_scale(1.25, 2560, 2048.0), 1.25);
        assert_eq!(effective_scale(1.5, 1920, 0.0), 1.5);
        // rounding of a fractional scale stays inside the tolerance
        assert_eq!(effective_scale(1.75, 2560, 1463.0), 1.75);
    }

    #[test]
    fn fit_rounds_even_clamps_and_enforces_minimum() {
        let screen = R(0, 0, 2560, 1440);
        assert_eq!(
            fit_rect(R(10, 10, 301, 201), screen),
            Some(R(10, 10, 300, 200))
        );
        // hanging over the right edge
        assert_eq!(fit_rect(R(2500, 0, 200, 100), screen), None);
        assert_eq!(
            fit_rect(R(2400, 0, 400, 100), screen),
            Some(R(2400, 0, 160, 100))
        );
        assert_eq!(fit_rect(R(0, 0, 63, 200), screen), None);
        assert_eq!(fit_rect(R(5000, 0, 200, 200), screen), None);
        // a screen with negative coordinates
        let left = R(-1920, 0, 1920, 1080);
        assert_eq!(
            fit_rect(R(-1000, 100, 640, 360), left),
            Some(R(-1000, 100, 640, 360))
        );
    }

    #[test]
    fn region_text_parses() {
        assert_eq!(parse_region("10,20,300,200"), Some(R(10, 20, 300, 200)));
        assert_eq!(
            parse_region(" -1920, 0 ,640,360"),
            Some(R(-1920, 0, 640, 360))
        );
        for bad in ["", "1,2,3", "1,2,3,4,5", "a,b,c,d", "0,0,0,10", "0,0,10,-5"] {
            assert_eq!(parse_region(bad), None, "{bad}");
        }
    }

    #[test]
    fn args_are_separate_items_with_offsets_and_limits() {
        let s = CaptureSettings::default();
        let out = Path::new(r"C:\My Videos\a.mp4.part");
        let a: Vec<String> = build_args(R(-1920, 40, 640, 360), &s, 600, out)
            .iter()
            .map(|x| x.to_string_lossy().into_owned())
            .collect();
        let has = |w: &[&str]| a.windows(w.len()).any(|x| x == w);
        assert!(has(&["-f", "gdigrab", "-framerate", "15"]));
        assert!(has(&[
            "-draw_mouse",
            "1",
            "-offset_x",
            "-1920",
            "-offset_y",
            "40"
        ]));
        assert!(has(&["-video_size", "640x360", "-i", "desktop"]));
        assert!(has(&["-t", "600", "-fs", "2147483648"]));
        assert!(has(&["-c:v", "libopenh264", "-b:v", "1000k"]));
        assert!(has(&["-pix_fmt", "yuv420p", "-f", "mp4"]));
        assert!(!a.contains(&"-nostdin".to_string()), "stdin carries q");
        assert_eq!(a.last().unwrap(), r"C:\My Videos\a.mp4.part");
        // -i comes after the input options and before the output ones
        let pos = |k: &str| a.iter().position(|x| x == k).unwrap();
        assert!(pos("-offset_x") < pos("-i") && pos("-i") < pos("-t") && pos("-t") < pos("-c:v"));
        let no_cursor = CaptureSettings {
            cursor: false,
            fps: 30,
            ..s
        };
        let a = build_args(R(0, 0, 100, 100), &no_cursor, 60, out)
            .iter()
            .map(|x| x.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(a.contains("-framerate 30 -draw_mouse 0"));
    }

    #[test]
    fn bitrate_scales_with_size_and_stays_in_range() {
        assert_eq!(bitrate_bps(64, 64, 10), 1_000_000);
        assert_eq!(bitrate_bps(640, 360, 15), 1_000_000);
        assert_eq!(bitrate_bps(1920, 1080, 15), 4_665_600);
        assert_eq!(bitrate_bps(2560, 1440, 30), 12_000_000);
        assert_eq!(bitrate_bps(7680, 4320, 30), 12_000_000);
    }

    #[test]
    fn settings_are_clamped() {
        let s = CaptureSettings {
            fps: 99,
            max_minutes: 0,
            format: " GIF ".into(),
            dir: Some(PathBuf::new()),
            ..CaptureSettings::default()
        }
        .normalized();
        assert_eq!((s.fps, s.max_minutes, s.format.as_str()), (30, 1, "gif"));
        assert!(s.dir.is_none());
        assert_eq!(s.target(), Format::Gif);
        assert_eq!(s.max_secs(), 60);
        let s = CaptureSettings {
            fps: 1,
            max_minutes: 200,
            format: "exe".into(),
            ..CaptureSettings::default()
        }
        .normalized();
        assert_eq!((s.fps, s.max_minutes, s.format.as_str()), (10, 30, "mp4"));
        assert_eq!(s.max_secs(), 1800);
        // an old settings file without the block reads as defaults
        let d: CaptureSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(d, CaptureSettings::default());
        assert!(!d.hotkey);
    }

    #[test]
    fn bar_stays_outside_the_frame_and_inside_the_screen() {
        let screen = R(0, 0, 1920, 1080);
        let bar = (300, 48);
        let region = R(500, 300, 640, 360);
        // below, centred
        assert_eq!(place_bar(region, bar, screen), Some((670, 673)));
        // no room below: above
        let low = R(500, 700, 640, 340);
        let (_, y) = place_bar(low, bar, screen).unwrap();
        assert!(y + 48 <= 700 - (FRAME_GAP + FRAME_W) as i32);
        // whole screen: nowhere
        assert_eq!(place_bar(screen, bar, screen), None);
        // full height, narrow: beside
        let tall = R(0, 0, 400, 1080);
        let (x, _) = place_bar(tall, bar, screen).unwrap();
        assert!(x >= 400 + (FRAME_GAP + FRAME_W) as i32);
        // never over the recorded area or its frame
        for r in [region, low, tall] {
            let (x, y) = place_bar(r, bar, screen).unwrap();
            let (outer, _) = frame_window(r);
            let overlap =
                x < outer.right() && x + 300 > outer.x && y < outer.bottom() && y + 48 > outer.y;
            assert!(!overlap, "{r:?}");
        }
        // inside fallback sits within the region
        let (x, y) = place_bar_inside(screen, bar);
        assert!(x >= 0 && y >= 0 && x + 300 <= 1920 && y + 48 <= 1080);
    }

    #[test]
    fn frame_surrounds_the_area_without_touching_it() {
        let r = R(100, 100, 640, 360);
        let (outer, hole) = frame_window(r);
        assert_eq!(outer, R(95, 95, 650, 370));
        // the hole, in screen coordinates, leaves the gap around the area
        let (hx, hy) = (outer.x + hole.x, outer.y + hole.y);
        assert_eq!((hx, hy), (97, 97));
        assert_eq!(hx + hole.w as i32, r.right() + FRAME_GAP as i32);
        assert_eq!(hy + hole.h as i32, r.bottom() + FRAME_GAP as i32);
        assert!(hx < r.x && hy < r.y);
    }

    /// Two real seconds of the desktop, when ffmpeg is installed (skipped otherwise).
    #[test]
    fn records_two_seconds_when_ffmpeg_is_there() {
        let exe = crate::ffmpeg::exe_path();
        if !crate::ffmpeg::is_working(&exe) {
            eprintln!("skipped: no ffmpeg at {}", exe.display());
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let s = CaptureSettings::default();
        let rec = start(&exe, R(0, 0, 320, 240), &s, d.path(), "t").unwrap();
        std::thread::sleep(Duration::from_secs(2));
        let out = rec.stop().unwrap();
        assert_eq!(out, d.path().join("t.mp4"));
        assert!(!d.path().join("t.mp4.part").exists());
        let probe = Command::new(&exe)
            .args(["-hide_banner", "-i"])
            .arg(&out)
            .stdin(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .unwrap();
        let p = crate::convert::ffmpeg::parse_probe(&String::from_utf8_lossy(&probe.stderr));
        let secs = p.duration_us.unwrap() as f64 / 1e6;
        assert!((1.4..=3.0).contains(&secs), "duration {secs}");
        // a cancelled one leaves nothing
        let rec = start(&exe, R(0, 0, 320, 240), &s, d.path(), "c").unwrap();
        std::thread::sleep(Duration::from_millis(500));
        rec.cancel();
        assert!(!d.path().join("c.mp4").exists() && !d.path().join("c.mp4.part").exists());
    }
}
