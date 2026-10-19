//! The keyboard shortcuts window (Ctrl+/): a read only, scrollable list of
//! the keys in `shortcuts::SECTIONS`, drawn by the window itself.

use std::cell::Cell;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::SetScrollInfo;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use super::dialog;
use super::theme::Theme;
use super::{App, scale};
use crate::shortcuts::SECTIONS;

const CLASS: PCWSTR = w!("WhispKeysWindow");
const STYLE: WINDOW_STYLE =
    WINDOW_STYLE(WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_THICKFRAME.0 | WS_VSCROLL.0);
/// Width of the client area and the margin, at 96 dpi.
const CLIENT_WIDTH: i32 = 520;
const MARGIN: i32 = 24;
/// Space above a section heading that is not the first.
const SECTION_GAP: i32 = 14;

pub(super) struct KeysWindow {
    pub hwnd: HWND,
    /// Normal and bold message fonts.
    fonts: Cell<[HFONT; 2]>,
    dpi: Cell<u32>,
}

pub(super) fn register(instance: HINSTANCE) -> windows::core::Result<()> {
    dialog::register(instance, CLASS, Some(keys_proc))
}

unsafe extern "system" fn keys_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(result) = super::app().and_then(|app| app.keys_message(hwnd, msg, wparam, lparam)) {
        return result;
    }
    // SAFETY: forwarding the message as received.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl KeysWindow {
    /// Height of one row for the current fonts and DPI.
    fn row_height(&self, hdc: HDC) -> i32 {
        let mut tm = TEXTMETRICW::default();
        // SAFETY: `tm` is a live local; the old font is put back.
        unsafe {
            let old = SelectObject(hdc, self.fonts.get()[0].into());
            let _ = GetTextMetricsW(hdc, &mut tm);
            SelectObject(hdc, old);
        }
        tm.tmHeight + tm.tmExternalLeading + scale(10, self.dpi.get())
    }

    /// Height of everything the list draws, margins included.
    fn content_height(&self, hdc: HDC) -> i32 {
        let dpi = self.dpi.get();
        let row = self.row_height(hdc);
        let mut y = scale(MARGIN, dpi);
        for (i, section) in SECTIONS.iter().enumerate() {
            if i > 0 {
                y += scale(SECTION_GAP, dpi);
            }
            y += row * (1 + section.entries.len() as i32) + scale(4, dpi);
        }
        y + scale(MARGIN, dpi)
    }

    /// Draws the list into `hdc`, scrolled by `scroll` pixels.
    fn draw(&self, hdc: HDC, theme: &Theme, client: &RECT, scroll: i32) {
        let dpi = self.dpi.get();
        let [normal, bold] = self.fonts.get();
        let row = self.row_height(hdc);
        let margin = scale(MARGIN, dpi);
        let (left, right) = (margin, client.right - margin);
        let draw_text = |text: &str, x0: i32, x1: i32, y: i32, flags: DRAW_TEXT_FORMAT| {
            let mut units: Vec<u16> = text.encode_utf16().collect();
            let mut rc = RECT {
                left: x0,
                top: y,
                right: x1,
                bottom: y + row,
            };
            // SAFETY: `units` and `rc` are live locals.
            unsafe {
                DrawTextW(
                    hdc,
                    &mut units,
                    &mut rc,
                    flags | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
                )
            };
        };
        // SAFETY: plain calls on a live DC; the old font is put back.
        unsafe {
            SetBkMode(hdc, TRANSPARENT);
            let old = SelectObject(hdc, normal.into());
            let mut y = margin - scroll;
            for (i, section) in SECTIONS.iter().enumerate() {
                if i > 0 {
                    y += scale(SECTION_GAP, dpi);
                }
                SelectObject(hdc, bold.into());
                SetTextColor(hdc, theme.text);
                draw_text(section.title, left, right, y, DT_LEFT);
                y += row + scale(4, dpi);
                SelectObject(hdc, normal.into());
                for entry in section.entries {
                    // The keys take what they need on the right; the rest
                    // is for the description.
                    let keys = entry.keys.join(" or ");
                    let mut size = SIZE::default();
                    let units: Vec<u16> = keys.encode_utf16().collect();
                    let _ = GetTextExtentPoint32W(hdc, &units, &mut size);
                    let split = (right - size.cx - scale(16, dpi)).max(left);
                    SetTextColor(hdc, theme.text);
                    draw_text(entry.action, left, split, y, DT_LEFT);
                    SetTextColor(hdc, theme.dim());
                    draw_text(&keys, split, right, y, DT_RIGHT);
                    y += row;
                }
            }
            SelectObject(hdc, old);
        }
    }

    /// The scroll bar for the content and the client height.
    fn update_scroll(&self) {
        // SAFETY: plain calls on our own window; `info` is a live local.
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let hdc = GetDC(Some(self.hwnd));
            let content = self.content_height(hdc);
            ReleaseDC(Some(self.hwnd), hdc);
            let mut info = SCROLLINFO {
                cbSize: size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE,
                nMin: 0,
                nMax: content - 1,
                nPage: rc.bottom.max(0) as u32,
                ..Default::default()
            };
            SetScrollInfo(self.hwnd, SB_VERT, &info, true);
            // Setting the range may have moved the position.
            info.fMask = SIF_POS;
            let _ = GetScrollInfo(self.hwnd, SB_VERT, &mut info);
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// Scrolls to `pos` pixels, within the range.
    fn scroll_to(&self, pos: i32) {
        let info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_POS,
            nPos: pos,
            ..Default::default()
        };
        // SAFETY: plain calls on our own window; `info` is a live local.
        unsafe {
            SetScrollInfo(self.hwnd, SB_VERT, &info, true);
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// The scroll bar's position, and its page and largest position.
    fn scroll_state(&self) -> (i32, i32, i32) {
        let mut info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_ALL,
            ..Default::default()
        };
        // SAFETY: plain call; `info` is a live local.
        let _ = unsafe { GetScrollInfo(self.hwnd, SB_VERT, &mut info) };
        let page = info.nPage as i32;
        (info.nPos, page, (info.nMax - page + 1).max(0))
    }

    fn row_pixels(&self) -> i32 {
        // SAFETY: the DC is released at once.
        unsafe {
            let hdc = GetDC(Some(self.hwnd));
            let row = self.row_height(hdc);
            ReleaseDC(Some(self.hwnd), hdc);
            row
        }
    }

    fn paint(&self, theme: &Theme) {
        let mut ps = PAINTSTRUCT::default();
        let mut rc = RECT::default();
        // SAFETY: BeginPaint and EndPaint pair up; the bitmap and DC made
        // here are deleted before returning. Drawing goes to a bitmap first
        // so scrolling does not flicker.
        unsafe {
            let hdc = BeginPaint(self.hwnd, &mut ps);
            let _ = GetClientRect(self.hwnd, &mut rc);
            let memory = CreateCompatibleDC(Some(hdc));
            let bitmap = CreateCompatibleBitmap(hdc, rc.right.max(1), rc.bottom.max(1));
            let old = SelectObject(memory, bitmap.into());
            let brush = CreateSolidBrush(theme.background);
            FillRect(memory, &rc, brush);
            let _ = DeleteObject(brush.into());
            self.draw(memory, theme, &rc, self.scroll_state().0);
            let _ = BitBlt(hdc, 0, 0, rc.right, rc.bottom, Some(memory), 0, 0, SRCCOPY);
            SelectObject(memory, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            let _ = EndPaint(self.hwnd, &ps);
        }
    }
}

impl App {
    /// The shortcuts window, if it is open.
    pub(super) fn keys_hwnd(&self) -> Option<HWND> {
        self.keys.borrow().as_ref().map(|k| k.hwnd)
    }

    /// Ctrl+/: opens the keyboard shortcuts window, or shows the open one.
    pub(super) fn open_keys(&self) {
        if let Some(hwnd) = self.keys_hwnd() {
            dialog::bring_forward(hwnd);
            return;
        }
        self.finish_swipe();
        self.hide_suggestions();
        let mut rc = RECT::default();
        // SAFETY: `rc` is a live local. Created where the owner is, so the
        // window starts on the owner's monitor and gets its DPI.
        let hwnd = unsafe {
            let _ = GetWindowRect(self.hwnd, &mut rc);
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                CLASS,
                w!("Keyboard Shortcuts"),
                STYLE | WS_CLIPCHILDREN,
                rc.left,
                rc.top,
                scale(CLIENT_WIDTH, 96),
                scale(400, 96),
                Some(self.hwnd),
                None,
                Some(self.instance),
                None,
            )
        };
        let Ok(hwnd) = hwnd else {
            self.show_status("Could not open the shortcuts");
            return;
        };
        let keys = KeysWindow {
            hwnd,
            fonts: Cell::new([HFONT::default(); 2]),
            // SAFETY: plain call on the window just created.
            dpi: Cell::new(unsafe { GetDpiForWindow(hwnd) }),
        };
        *self.keys.borrow_mut() = Some(keys);
        let guard = self.keys.borrow();
        let Some(keys) = guard.as_ref() else {
            return;
        };
        keys.restyle(&self.theme.get());
        // The height of the content, or as much as the screen allows.
        // SAFETY: the DC is released at once; plain calls.
        let (width, height) = unsafe {
            let hdc = GetDC(Some(hwnd));
            let content = keys.content_height(hdc);
            ReleaseDC(Some(hwnd), hdc);
            let dpi = keys.dpi.get();
            let (w, h) = dialog::outer_size(STYLE, scale(CLIENT_WIDTH, dpi), content, dpi);
            let work = dialog::work_area(self.hwnd);
            (w, h.min((work.bottom - work.top) * 85 / 100))
        };
        dialog::centre_over(self.hwnd, hwnd, width, height);
        // SAFETY: plain calls on the window just created.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetFocus(Some(hwnd));
        }
        keys.update_scroll();
    }

    /// Esc or the close button: closes the shortcuts window.
    pub(super) fn close_keys(&self) {
        if let Some(hwnd) = self.keys_hwnd() {
            // SAFETY: our own window; WM_DESTROY drops the state.
            let _ = unsafe { DestroyWindow(hwnd) };
        }
    }

    /// The theme changed.
    pub(super) fn restyle_keys(&self) {
        if let Some(keys) = self.keys.borrow().as_ref() {
            keys.restyle(&self.theme.get());
        }
    }

    /// Messages of the shortcuts window.
    pub(super) fn keys_message(
        &self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        if msg == WM_DESTROY {
            let taken = self.keys.borrow_mut().take();
            if let Some(keys) = taken.filter(|k| k.hwnd == hwnd) {
                dialog::delete_objects(keys.fonts.get().into_iter().map(HGDIOBJ::from));
            }
            return None;
        }
        let guard = self.keys.borrow();
        let keys = guard.as_ref().filter(|k| k.hwnd == hwnd)?;
        match msg {
            WM_ERASEBKGND => return Some(LRESULT(1)),
            WM_PAINT => keys.paint(&self.theme.get()),
            WM_SIZE => keys.update_scroll(),
            WM_VSCROLL => {
                let (pos, page, max) = keys.scroll_state();
                let row = keys.row_pixels();
                let to = match (wparam.0 & 0xffff) as i32 {
                    x if x == SB_LINEUP.0 => pos - row,
                    x if x == SB_LINEDOWN.0 => pos + row,
                    x if x == SB_PAGEUP.0 => pos - page,
                    x if x == SB_PAGEDOWN.0 => pos + page,
                    x if x == SB_TOP.0 => 0,
                    x if x == SB_BOTTOM.0 => max,
                    // The 16 bit track position is enough for this list.
                    x if x == SB_THUMBTRACK.0 || x == SB_THUMBPOSITION.0 => {
                        ((wparam.0 >> 16) & 0xffff) as i32
                    }
                    _ => pos,
                };
                keys.scroll_to(to.clamp(0, max));
            }
            WM_MOUSEWHEEL => {
                let delta = i32::from((wparam.0 >> 16) as u16 as i16);
                let (pos, _, max) = keys.scroll_state();
                // Three rows per notch.
                keys.scroll_to((pos - delta * 3 * keys.row_pixels() / 120).clamp(0, max));
            }
            WM_KEYDOWN => {
                let (pos, page, max) = keys.scroll_state();
                let row = keys.row_pixels();
                let to = match VIRTUAL_KEY(wparam.0 as u16) {
                    VK_UP => pos - row,
                    VK_DOWN => pos + row,
                    VK_PRIOR => pos - page,
                    VK_NEXT => pos + page,
                    VK_HOME => 0,
                    VK_END => max,
                    _ => return None,
                };
                keys.scroll_to(to.clamp(0, max));
            }
            WM_DPICHANGED => {
                keys.dpi.set((wparam.0 & 0xffff) as u32);
                // SAFETY: for WM_DPICHANGED, lparam points to the suggested window RECT.
                unsafe {
                    let rc = *(lparam.0 as *const RECT);
                    let flags = SWP_NOZORDER | SWP_NOACTIVATE;
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        rc.left,
                        rc.top,
                        rc.right - rc.left,
                        rc.bottom - rc.top,
                        flags,
                    );
                }
                keys.restyle(&self.theme.get());
            }
            WM_CLOSE => {
                drop(guard);
                self.close_keys();
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

impl KeysWindow {
    /// New fonts for the DPI, the title bar for the theme, and a repaint.
    fn restyle(&self, theme: &Theme) {
        let old = self.fonts.replace(dialog::message_fonts(self.dpi.get()));
        dialog::delete_objects(old.into_iter().map(HGDIOBJ::from));
        theme.apply_to_frame(self.hwnd);
        dialog::refresh_frame(self.hwnd);
        self.update_scroll();
    }
}
