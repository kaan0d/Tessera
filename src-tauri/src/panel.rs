//! The mini progress panel: a frameless, non-activating window in a screen corner.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, WebviewWindow};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST, MA_NOACTIVATE, SW_HIDE, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    WM_MOUSEACTIVATE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

/// Distance of the visible card from the work-area edge, logical px.
const MARGIN: f64 = 16.0;
/// Transparent padding the UI draws around the card (room for its CSS shadow), per side.
const PAD: f64 = 12.0;
const CARD_W: f64 = 360.0;

/// Window size in logical px for `n` rows: the card (66 + 44 per row, 6 rows max) plus padding.
pub fn window_size(n: usize) -> (f64, f64) {
    (
        CARD_W + 2.0 * PAD,
        66.0 + 44.0 * n.clamp(1, 6) as f64 + 2.0 * PAD,
    )
}

/// Top-left of the window in physical px so the visible card sits `MARGIN` from the
/// corner of `work` (x, y, w, h). `size` is the window's physical size.
pub fn position(
    corner: Corner,
    work: (i32, i32, u32, u32),
    size: (u32, u32),
    scale: f64,
) -> (i32, i32) {
    let (wx, wy, ww, wh) = work;
    let inset = ((MARGIN - PAD) * scale).round() as i32;
    let (w, h) = (size.0 as i32, size.1 as i32);
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => wx + inset,
        Corner::TopRight | Corner::BottomRight => wx + ww as i32 - w - inset,
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => wy + inset,
        Corner::BottomLeft | Corner::BottomRight => wy + wh as i32 - h - inset,
    };
    (x, y)
}

fn hwnd(w: &WebviewWindow) -> Option<HWND> {
    w.hwnd().ok().map(|h| HWND(h.0))
}

/// Clicks inside the card are delivered, but never activate the window.
unsafe extern "system" fn no_activate(
    h: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_MOUSEACTIVATE {
            return LRESULT(MA_NOACTIVATE as isize);
        }
        DefSubclassProc(h, msg, wp, lp)
    }
}

/// One-time setup: no taskbar or Alt-Tab entry, topmost, never takes focus.
pub fn init(w: &WebviewWindow) {
    let _ = w.set_focusable(false);
    let _ = w.set_shadow(false);
    if let Some(h) = hwnd(w) {
        // SAFETY: style read-modify-write and a subclass on a window this process owns.
        unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            let add = WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0;
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex | add as isize);
            let _ = SetWindowSubclass(h, Some(no_activate), 1, 0);
        }
    }
}

/// Sizes for `n` rows, moves to `corner` of the work area and shows, topmost and without
/// activating, in one `SetWindowPos`, so no step can pull focus from the app being typed in.
/// Re-asserted on every call. Limit: Windows keeps exclusive-fullscreen games above any
/// topmost window; the panel is not visible over those.
pub fn show(app: &AppHandle, w: &WebviewWindow, n: usize, corner: Corner) {
    let Some(h) = hwnd(w) else { return };
    let monitor = w
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let scale = m.scale_factor();
    let (lw, lh) = window_size(n);
    let size = ((lw * scale).round() as u32, (lh * scale).round() as u32);
    let a = m.work_area();
    let (x, y) = position(
        corner,
        (a.position.x, a.position.y, a.size.width, a.size.height),
        size,
        scale,
    );
    // SAFETY: plain window positioning on a window this process owns.
    unsafe {
        let _ = SetWindowPos(
            h,
            Some(HWND_TOPMOST),
            x,
            y,
            size.0 as i32,
            size.1 as i32,
            SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOOWNERZORDER,
        );
    }
}

/// Hides through Win32 too: `show` bypasses tauri, whose own `hide` then sees nothing to do.
pub fn hide(w: &WebviewWindow) {
    if let Some(h) = hwnd(w) {
        // SAFETY: plain ShowWindow on a window this process owns.
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: (i32, i32, u32, u32) = (0, 0, 1920, 1040); // taskbar at the bottom

    #[test]
    fn sizes_follow_row_count() {
        assert_eq!(window_size(1), (384.0, 134.0));
        assert_eq!(window_size(6), (384.0, 354.0));
        assert_eq!(window_size(20), window_size(6));
    }

    #[test]
    fn card_sits_16_from_each_corner() {
        let size = (384, 134);
        assert_eq!(
            position(Corner::TopRight, WORK, size, 1.0),
            (1920 - 384 - 4, 4)
        );
        assert_eq!(position(Corner::TopLeft, WORK, size, 1.0), (4, 4));
        assert_eq!(
            position(Corner::BottomRight, WORK, size, 1.0),
            (1532, 1040 - 134 - 4)
        );
        assert_eq!(position(Corner::BottomLeft, WORK, size, 1.0), (4, 902));
    }

    #[test]
    fn scaling_and_offset_work_areas() {
        // 150% DPI, second monitor to the left, taskbar on top (work area starts at y=48)
        let work = (-2880, 48, 2880, 1552);
        let size = (576, 201);
        assert_eq!(
            position(Corner::TopLeft, work, size, 1.5),
            (-2880 + 6, 48 + 6)
        );
        assert_eq!(
            position(Corner::BottomRight, work, size, 1.5),
            (-576 - 6, 48 + 1552 - 201 - 6)
        );
    }
}
