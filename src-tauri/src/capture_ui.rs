//! The windows and state around a screen recording: one area picker per screen, then a small
//! control bar and a red frame while ffmpeg records. Everything that decides a position or a
//! size is a pure function in `capture.rs`; this file only opens windows and moves them.
//!
//! The bar and the frame are also excluded from screen capture by Windows, so even when they
//! have to sit over the recorded area (a whole-screen recording) they never reach the video.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, HGDIOBJ, RGN_DIFF, SetWindowRgn,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowDisplayAffinity, SetWindowPos,
    WDA_EXCLUDEFROMCAPTURE,
};

use crate::capture::{self, Rect, Recording};
use crate::convert::{Conflict, Format, OutDir};
use crate::i18n::{self, fmt, t};
use crate::{AppState, log, notify, panel, settings};

const SEL_PREFIX: &str = "capture-sel-";
const BAR: &str = "capture-bar";
const FRAME: &str = "capture-frame";
/// Control bar size in CSS pixels.
const BAR_SIZE: (f64, f64) = (300.0, 48.0);
/// Below this much free space a running recording is stopped (bytes).
const STOP_FREE: u64 = 200 << 20;

struct Active {
    rec: Recording,
    target: Format,
    dir: PathBuf,
}

#[derive(Default)]
struct Inner {
    selecting: bool,
    rec: Option<Active>,
}

/// Picker and recording state, shared by the tray, the shortcut, the windows and the watcher.
#[derive(Default)]
pub struct CaptureState {
    inner: Mutex<Inner>,
}

impl CaptureState {
    pub fn is_recording(&self) -> bool {
        self.inner.lock().unwrap().rec.is_some()
    }
}

#[derive(Serialize)]
pub struct Status {
    secs: u64,
}

fn state(app: &AppHandle) -> tauri::State<'_, AppState> {
    app.state::<AppState>()
}

fn hwnd(w: &WebviewWindow) -> Option<HWND> {
    w.hwnd().ok().map(|h| HWND(h.0))
}

/// Puts `w` at `r` (desktop pixels) in one call, topmost; `activate` lets it take the focus.
fn place(w: &WebviewWindow, r: Rect, activate: bool) {
    if let Some(h) = hwnd(w) {
        let flags = if activate {
            SWP_SHOWWINDOW
        } else {
            SWP_SHOWWINDOW | SWP_NOACTIVATE
        };
        // SAFETY: plain window positioning on a window this process owns.
        unsafe {
            let _ = SetWindowPos(
                h,
                Some(HWND_TOPMOST),
                r.x,
                r.y,
                r.w as i32,
                r.h as i32,
                flags,
            );
        }
    }
}

/// Keeps the window out of screenshots and screen recordings.
fn exclude_from_capture(w: &WebviewWindow) {
    if let Some(h) = hwnd(w) {
        // SAFETY: plain call on a window this process owns; failing only leaves it capturable.
        unsafe {
            let _ = SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE);
        }
    }
}

/// Cuts `hole` (relative to the window) out of the window's shape.
fn punch(w: &WebviewWindow, size: (u32, u32), hole: Rect) {
    if let Some(h) = hwnd(w) {
        // SAFETY: the regions are ours until SetWindowRgn takes the combined one; the two
        // helpers are deleted here.
        unsafe {
            let outer = CreateRectRgn(0, 0, size.0 as i32, size.1 as i32);
            let inner = CreateRectRgn(hole.x, hole.y, hole.right(), hole.bottom());
            let ring = CreateRectRgn(0, 0, 0, 0);
            CombineRgn(Some(ring), Some(outer), Some(inner), RGN_DIFF);
            let _ = DeleteObject(HGDIOBJ(outer.0));
            let _ = DeleteObject(HGDIOBJ(inner.0));
            SetWindowRgn(h, Some(ring), true);
        }
    }
}

fn monitor_rect(m: &tauri::Monitor) -> Rect {
    Rect {
        x: m.position().x,
        y: m.position().y,
        w: m.size().width,
        h: m.size().height,
    }
}

fn fail(app: &AppHandle, msg: &str) {
    log::write("error", &format!("screen recording: {msg}"));
    notify(
        app,
        t(i18n::lang(), "notify.failed_title"),
        &fmt(i18n::lang(), "cap.failed", &[("detail", msg)]),
    );
}

/// Starts the area picker (tray, shortcut, `--capture`). A recording or picker already open, or
/// no ffmpeg, answers with a notification instead.
pub fn begin(app: &AppHandle) {
    let lang = i18n::lang();
    let ready = state(app).settings.lock().unwrap().ffmpeg_ready;
    {
        let st = state(app);
        let mut g = st.capture.inner.lock().unwrap();
        if g.selecting || g.rec.is_some() {
            drop(g);
            notify(app, t(lang, "notify.failed_title"), t(lang, "cap.busy"));
            return;
        }
        if !ready {
            drop(g);
            notify(app, t(lang, "notify.failed_title"), t(lang, "cap.no_ffmpeg"));
            return;
        }
        g.selecting = true;
    }
    let app = app.clone();
    // windows are built off the event loop thread (building from it can deadlock)
    std::thread::spawn(move || {
        if let Err(e) = open_pickers(&app) {
            abort_select(&app);
            fail(&app, &e.to_string());
        }
    });
}

fn open_pickers(app: &AppHandle) -> tauri::Result<()> {
    let monitors = app.available_monitors()?;
    let cursor = app.cursor_position().ok();
    let mut focus = None;
    for (i, m) in monitors.iter().enumerate() {
        let label = format!("{SEL_PREFIX}{i}");
        let w = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("capture.html".into()))
            .title(crate::app_info::APP_NAME)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .visible(false)
            .data_directory(settings::data_dir().join("webview"))
            .build()?;
        let r = monitor_rect(m);
        place(&w, r, true);
        let under_cursor = cursor.is_some_and(|c| {
            (c.x as i32) >= r.x
                && (c.x as i32) < r.right()
                && (c.y as i32) >= r.y
                && (c.y as i32) < r.bottom()
        });
        if under_cursor || focus.is_none() {
            focus = Some(w);
        }
    }
    if let Some(w) = focus {
        let _ = w.set_focus();
    }
    Ok(())
}

fn close_pickers(app: &AppHandle) {
    for (label, w) in app.webview_windows() {
        if label.starts_with(SEL_PREFIX) {
            // destroy: a close request is turned into a hide by the app
            let _ = w.destroy();
        }
    }
}

/// Esc or a closed picker: back to idle.
pub fn abort_select(app: &AppHandle) {
    close_pickers(app);
    state(app).capture.inner.lock().unwrap().selecting = false;
}

#[tauri::command]
pub fn capture_abort_select(app: AppHandle) {
    abort_select(&app);
}

/// The picker reports a box in its CSS pixels; `inner_w` is the page's width, which exposes a
/// webview whose scale has not caught up with the screen.
#[tauri::command]
pub fn capture_pick(
    monitor: usize,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    inner_w: f64,
    app: AppHandle,
) -> Result<(), String> {
    let lang = i18n::lang();
    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    let m = monitors
        .get(monitor)
        .ok_or_else(|| t(lang, "cap.bad_region").to_string())?;
    let bounds = monitor_rect(m);
    let reported = m.scale_factor();
    let scale = capture::effective_scale(reported, bounds.w, inner_w);
    if scale != reported {
        log::write(
            "info",
            &format!("capture: screen {monitor} reports scale {reported}, page shows {scale}"),
        );
    }
    let rect = capture::to_physical((bounds.x, bounds.y), scale, (x, y, w, h));
    let rect = capture::fit_rect(rect, bounds)
        .ok_or_else(|| t(lang, "cap.bad_region").to_string())?;
    close_pickers(&app);
    std::thread::spawn(move || start_recording(&app, rect, bounds, scale));
    Ok(())
}

fn start_recording(app: &AppHandle, rect: Rect, bounds: Rect, scale: f64) {
    let (cs, ready) = {
        let st = state(app);
        let s = st.settings.lock().unwrap();
        (s.capture.clone().normalized(), s.ffmpeg_ready)
    };
    let lang = i18n::lang();
    let dir = cs.out_dir();
    let started = if ready {
        capture::start(&crate::ffmpeg::exe_path(), rect, &cs, &dir, &capture::file_stem())
    } else {
        Err(crate::ffmpeg::FfError::new("cap.no_ffmpeg"))
    };
    let rec = match started {
        Ok(r) => r,
        Err(e) => {
            state(app).capture.inner.lock().unwrap().selecting = false;
            let msg = e.message(lang);
            log::write("error", &format!("screen recording did not start: {msg}"));
            notify(app, t(lang, "notify.failed_title"), &msg);
            return;
        }
    };
    {
        let st = state(app);
        let mut g = st.capture.inner.lock().unwrap();
        g.selecting = false;
        g.rec = Some(Active {
            rec,
            target: cs.target(),
            dir: dir.clone(),
        });
    }
    // indicator first: nothing records without it
    if let Err(e) = show_indicators(app, rect, bounds, scale) {
        cancel(app);
        fail(app, &e.to_string());
        return;
    }
    refresh_tray(app);
    watch(app.clone(), dir);
}

fn show_indicators(app: &AppHandle, rect: Rect, bounds: Rect, scale: f64) -> tauri::Result<()> {
    let (outer, hole) = capture::frame_window(rect);
    let frame = WebviewWindowBuilder::new(app, FRAME, WebviewUrl::App("capture-frame.html".into()))
        .title(crate::app_info::APP_NAME)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .closable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .focused(false)
        .visible(false)
        .data_directory(settings::data_dir().join("webview"))
        .build()?;
    panel::init(&frame);
    exclude_from_capture(&frame);
    place(&frame, outer, false);
    punch(&frame, (outer.w, outer.h), hole);
    let _ = frame.set_ignore_cursor_events(true);

    let size = (
        (BAR_SIZE.0 * scale).round() as u32,
        (BAR_SIZE.1 * scale).round() as u32,
    );
    let (x, y) = capture::place_bar(rect, size, bounds)
        .unwrap_or_else(|| capture::place_bar_inside(rect, size));
    let bar = WebviewWindowBuilder::new(app, BAR, WebviewUrl::App("capture-bar.html".into()))
        .title(crate::app_info::APP_NAME)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .closable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .focused(false)
        .visible(false)
        .data_directory(settings::data_dir().join("webview"))
        .build()?;
    panel::init(&bar);
    exclude_from_capture(&bar);
    place(
        &bar,
        Rect {
            x,
            y,
            w: size.0,
            h: size.1,
        },
        false,
    );
    Ok(())
}

fn hide_indicators(app: &AppHandle) {
    for label in [BAR, FRAME] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.destroy();
        }
    }
}

fn refresh_tray(app: &AppHandle) {
    let lang = i18n::lang();
    let _ = crate::set_tray_menu(app, lang);
    if let Some(tray) = app.tray_by_id(crate::TRAY_ID) {
        let on = state(app).capture.is_recording();
        let tip = if on {
            format!("{} - {}", crate::app_info::APP_NAME, t(lang, "cap.recording"))
        } else {
            crate::app_info::APP_NAME.to_string()
        };
        let _ = tray.set_tooltip(Some(tip));
    }
}

#[derive(Clone, Copy)]
enum Why {
    Limit,
    Disk,
}

/// Ends the recording when ffmpeg stops by itself (time or size limit) or the disk runs low.
fn watch(app: AppHandle, dir: PathBuf) {
    std::thread::spawn(move || {
        let mut last_disk = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(400));
            let why = {
                let st = state(&app);
                let mut g = st.capture.inner.lock().unwrap();
                let Some(a) = g.rec.as_mut() else { return };
                if a.rec.has_ended() || a.rec.size() >= capture::MAX_BYTES {
                    Some(Why::Limit)
                } else if last_disk.elapsed() >= Duration::from_secs(1) {
                    last_disk = Instant::now();
                    capture::free_bytes(&dir)
                        .is_some_and(|f| f < STOP_FREE)
                        .then_some(Why::Disk)
                } else {
                    None
                }
            };
            if let Some(w) = why {
                finish(&app, Some(w));
                return;
            }
        }
    });
}

fn take(app: &AppHandle) -> Option<Active> {
    let a = state(app).capture.inner.lock().unwrap().rec.take();
    if a.is_some() {
        hide_indicators(app);
        refresh_tray(app);
    }
    a
}

/// Stops, saves, and tells the user. `why` is set when a limit ended it, not the user.
fn finish(app: &AppHandle, why: Option<Why>) {
    let Some(a) = take(app) else { return };
    let lang = i18n::lang();
    match a.rec.stop() {
        Ok(path) => saved(app, path, a.target, &a.dir, why),
        Err(e) => {
            let msg = e.message(lang);
            log::write("error", &format!("screen recording failed: {msg}"));
            notify(app, t(lang, "notify.failed_title"), &msg);
        }
    }
}

fn saved(app: &AppHandle, path: PathBuf, target: Format, dir: &std::path::Path, why: Option<Why>) {
    let lang = i18n::lang();
    if target == Format::Mp4 {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let key = match why {
            None => "cap.saved_body",
            Some(Why::Limit) => "cap.saved_limit",
            Some(Why::Disk) => "cap.saved_disk",
        };
        notify(
            app,
            t(lang, "cap.saved_title"),
            &fmt(lang, key, &[("name", &name)]),
        );
        return;
    }
    // GIF and WEBM go through the normal queue (progress panel, history); the MP4 stays
    let st = state(app);
    let s = st.settings.lock().unwrap().clone();
    let mut opts = s.to_options();
    opts.output.dir = OutDir::Fixed(dir.to_path_buf());
    opts.output.conflict = Conflict::AutoRename;
    opts.output.template = "{name}".into();
    st.queue
        .enqueue(app, target, vec![path], &opts, s.ffmpeg_ready);
}

/// Stop button, tray item, shortcut.
pub fn stop(app: &AppHandle) {
    let app = app.clone();
    // stopping waits for ffmpeg: not on the event loop thread
    std::thread::spawn(move || finish(&app, None));
}

fn cancel(app: &AppHandle) {
    if let Some(a) = take(app) {
        a.rec.cancel();
    }
}

/// Shortcut: starts the picker, or stops a running recording.
pub fn toggle(app: &AppHandle) {
    if state(app).capture.is_recording() {
        stop(app);
    } else {
        begin(app);
    }
}

/// App exit: a running recording is finished properly (its file stays playable).
pub fn shutdown(app: &AppHandle) {
    finish(app, None);
}

#[tauri::command]
pub fn capture_stop(app: AppHandle) {
    stop(&app);
}

#[tauri::command]
pub fn capture_cancel(app: AppHandle) {
    std::thread::spawn(move || cancel(&app));
}

/// Seconds recorded so far (the control bar's timer); None when nothing records.
#[tauri::command]
pub fn capture_status(app: AppHandle) -> Option<Status> {
    state(&app)
        .capture
        .inner
        .lock()
        .unwrap()
        .rec
        .as_ref()
        .map(|a| Status {
            secs: a.rec.elapsed().as_secs(),
        })
}

/// Registers or removes the global shortcut; a failure (taken by another program) is told to
/// the user.
pub fn apply_hotkey(app: &AppHandle, on: bool) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let gs = app.global_shortcut();
    let _ = gs.unregister(capture::HOTKEY);
    if !on {
        return;
    }
    if let Err(e) = gs.register(capture::HOTKEY) {
        let lang = i18n::lang();
        let msg = fmt(
            lang,
            "cap.hotkey_failed",
            &[("key", capture::HOTKEY), ("err", &e.to_string())],
        );
        log::write("error", &msg);
        notify(app, t(lang, "notify.failed_title"), &msg);
    }
}
