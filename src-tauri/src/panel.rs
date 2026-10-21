//! The mini progress panel: a frameless, non-activating window in a screen corner.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, LogicalSize, PhysicalPosition, WebviewWindow};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE, SW_SHOWNOACTIVATE,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
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

/// One-time setup: no taskbar or Alt-Tab entry, never takes focus.
pub fn init(w: &WebviewWindow) {
    let _ = w.set_focusable(false);
    let _ = w.set_shadow(false);
    if let Some(h) = hwnd(w) {
        // SAFETY: plain style read-modify-write on a window this process owns.
        unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            SetWindowLongPtrW(
                h,
                GWL_EXSTYLE,
                ex | (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) as isize,
            );
        }
    }
}

/// Sizes for `n` rows, moves to `corner` of the work area and shows without activating.
pub fn show(app: &AppHandle, w: &WebviewWindow, n: usize, corner: Corner) {
    let (lw, lh) = window_size(n);
    let _ = w.set_size(LogicalSize::new(lw, lh));
    let monitor = w
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    if let (Some(m), Ok(size)) = (monitor, w.outer_size()) {
        let a = m.work_area();
        let (x, y) = position(
            corner,
            (a.position.x, a.position.y, a.size.width, a.size.height),
            (size.width, size.height),
            m.scale_factor(),
        );
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
    match hwnd(w) {
        // SAFETY: shows without stealing focus.
        Some(h) => unsafe {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        },
        None => {
            let _ = w.show();
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
