//! What the preferences and keyboard shortcuts windows share: window class
//! registration, the system message font, sizing and placement.

use std::ffi::c_void;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, SystemParametersInfoForDpi};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use super::scale;

/// Registers a window class for one of the two windows. The background is
/// painted by the window itself, in the theme's colour.
pub(super) fn register(
    instance: HINSTANCE,
    name: PCWSTR,
    proc: WNDPROC,
) -> windows::core::Result<()> {
    // SAFETY: plain calls; the class name is static.
    unsafe {
        // Resource 1, embedded by build.rs, in the large and small sizes.
        let icon = |x, y| {
            LoadImageW(
                Some(instance),
                // MAKEINTRESOURCE(1): an ID, not a pointer.
                PCWSTR(std::ptr::without_provenance(1)),
                IMAGE_ICON,
                GetSystemMetrics(x),
                GetSystemMetrics(y),
                LR_DEFAULTCOLOR,
            )
            .map(|h| HICON(h.0))
            .unwrap_or_default()
        };
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: proc,
            hInstance: instance,
            lpszClassName: name,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: icon(SM_CXICON, SM_CYICON),
            hIconSm: icon(SM_CXSMICON, SM_CYSMICON),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
    }
    Ok(())
}

/// The system message font (what dialogs use) for `dpi`, in normal and
/// bold. The caller deletes them.
pub(super) fn message_fonts(dpi: u32) -> [HFONT; 2] {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    // SAFETY: `metrics` is a properly sized local; the fonts are created
    // from filled in LOGFONTs.
    unsafe {
        let ok = SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            metrics.cbSize,
            Some(&mut metrics as *mut _ as *mut c_void),
            0,
            dpi,
        )
        .is_ok();
        let mut normal = metrics.lfMessageFont;
        if !ok {
            // Segoe UI 9 pt, the usual message font.
            normal = LOGFONTW {
                lfHeight: -scale(12, dpi),
                ..Default::default()
            };
            for (dst, src) in normal.lfFaceName.iter_mut().zip("Segoe UI".encode_utf16()) {
                *dst = src;
            }
        }
        let mut bold = normal;
        bold.lfWeight = FW_BOLD.0 as i32;
        [CreateFontIndirectW(&normal), CreateFontIndirectW(&bold)]
    }
}

/// Deletes fonts or brushes made for a window, skipping empty handles.
pub(super) fn delete_objects(objects: impl IntoIterator<Item = HGDIOBJ>) {
    for object in objects.into_iter().filter(|o| !o.is_invalid()) {
        // SAFETY: the objects were created here and are no longer in use.
        let _ = unsafe { DeleteObject(object) };
    }
}

/// The window size that has a client area of `width` by `height`.
pub(super) fn outer_size(style: WINDOW_STYLE, width: i32, height: i32, dpi: u32) -> (i32, i32) {
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    // SAFETY: `rc` is a live local.
    let _ = unsafe { AdjustWindowRectExForDpi(&mut rc, style, false, WINDOW_EX_STYLE(0), dpi) };
    (rc.right - rc.left, rc.bottom - rc.top)
}

/// The work area of the monitor `hwnd` is on.
pub(super) fn work_area(hwnd: HWND) -> RECT {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: plain calls; `info` is a properly sized local.
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            return info.rcWork;
        }
    }
    RECT {
        left: 0,
        top: 0,
        right: 1024,
        bottom: 768,
    }
}

/// Puts `hwnd`, `width` by `height` in size, over the middle of `owner`,
/// inside the work area.
pub(super) fn centre_over(owner: HWND, hwnd: HWND, width: i32, height: i32) {
    let mut rc = RECT::default();
    // SAFETY: `rc` is a live local; plain calls.
    unsafe {
        let _ = GetWindowRect(owner, &mut rc);
        let work = work_area(owner);
        let x = (rc.left + (rc.right - rc.left - width) / 2)
            .clamp(work.left, (work.right - width).max(work.left));
        let y = (rc.top + (rc.bottom - rc.top - height) / 2)
            .clamp(work.top, (work.bottom - height).max(work.top));
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Whether `hwnd` is `root` or one of its children, or something `root` or
/// its children own, such as a combo box's drop down list.
pub(super) fn belongs(root: HWND, mut hwnd: HWND) -> bool {
    for _ in 0..4 {
        if hwnd == root {
            return true;
        }
        // SAFETY: plain call; an invalid window gives an error.
        match unsafe { GetParent(hwnd) } {
            Ok(parent) => hwnd = parent,
            Err(_) => return false,
        }
    }
    false
}

/// Shows and focuses a window that is already open.
pub(super) fn bring_forward(hwnd: HWND) {
    // SAFETY: plain calls on a live window.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

/// The dark or light title bar of a window shown in a new theme.
pub(super) fn refresh_frame(hwnd: HWND) {
    // SAFETY: plain call; makes the title bar repaint in the new colours.
    unsafe {
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, flags);
    }
}

/// The height of `text` wrapped to `width` in `font`.
pub(super) fn text_height(hdc: HDC, font: HFONT, text: &str, width: i32) -> i32 {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: width.max(1),
        bottom: 0,
    };
    // SAFETY: `units` and `rc` are live locals; the old font is put back.
    unsafe {
        let old = SelectObject(hdc, font.into());
        DrawTextW(
            hdc,
            &mut units,
            &mut rc,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(hdc, old);
    }
    rc.bottom - rc.top
}
