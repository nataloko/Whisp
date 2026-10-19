//! The search panel (Ctrl+F): a search box above an owner drawn list of
//! results, filling the window in place of the editor while open.

use std::cell::Cell;
use std::path::Path;
use std::time::Instant;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, EM_SETSEL, ODS_SELECTED, SetWindowTheme};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

use super::{App, is_character_key, scale};
use crate::search::Row;

pub(super) const TIMER_SEARCH: usize = 5;
/// Opens the row in wparam, posted so that a click in the list is handled
/// before the list is hidden.
pub(super) const WM_SEARCH_OPEN: u32 = WM_APP + 4;
/// Typing filters after this pause, as in Whisp.
const SEARCH_DELAY_MS: u32 = 150;
/// Most rows shown; a last row says there are more.
const MAX_ROWS: usize = 500;
/// Match highlight: black on yellow, as in Whisp, readable in both themes.
const MATCH_BACKGROUND: COLORREF = COLORREF(0x006b_f0f9);
const MATCH_TEXT: COLORREF = COLORREF(0);

/// The panel's controls and the GDI objects that draw them.
pub(super) struct Panel {
    pub edit: HWND,
    pub list: HWND,
    /// Normal and bold list fonts, and the search box font.
    fonts: Cell<[HFONT; 3]>,
    /// Window background and search box background.
    brushes: Cell<[HBRUSH; 2]>,
    line_height: Cell<i32>,
    item_height: Cell<i32>,
}

impl Panel {
    fn create(parent: HWND, instance: HINSTANCE) -> windows::core::Result<Panel> {
        // SAFETY: plain window creation; the class names are static.
        unsafe {
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                PCWSTR::null(),
                WINDOW_STYLE(WS_CHILD.0 | ES_AUTOHSCROLL as u32),
                0,
                0,
                0,
                0,
                Some(parent),
                None,
                Some(instance),
                None,
            )?;
            // LBS_NODATA: the rows live in `App::search_rows`, the list only
            // knows how many there are.
            let style = WS_CHILD.0
                | WS_VSCROLL.0
                | (LBS_OWNERDRAWFIXED | LBS_NODATA | LBS_NOINTEGRALHEIGHT | LBS_NOTIFY) as u32;
            let list = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("LISTBOX"),
                PCWSTR::null(),
                WINDOW_STYLE(style),
                0,
                0,
                0,
                0,
                Some(parent),
                None,
                Some(instance),
                None,
            )?;
            Ok(Panel {
                edit,
                list,
                fonts: Cell::new([HFONT::default(); 3]),
                brushes: Cell::new([HBRUSH::default(); 2]),
                line_height: Cell::new(16),
                item_height: Cell::new(40),
            })
        }
    }
}

fn font(points: i32, bold: bool, dpi: u32) -> HFONT {
    let mut lf = LOGFONTW {
        lfHeight: -(points * dpi as i32 / 72),
        lfWeight: if bold {
            FW_BOLD.0 as i32
        } else {
            FW_NORMAL.0 as i32
        },
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    for (dst, src) in lf.lfFaceName.iter_mut().zip("Segoe UI".encode_utf16()) {
        *dst = src;
    }
    // SAFETY: `lf` is a filled in local.
    unsafe { CreateFontIndirectW(&lf) }
}

fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    // SAFETY: only used with messages that take no pointers, or pointers
    // valid for the call.
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
}

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

impl App {
    pub(super) fn search_panel(&self) -> Option<&Panel> {
        self.search.get()
    }

    /// Ctrl+F: shows the search panel, listing every note.
    pub(super) fn open_search(&self) {
        if self.search_open.get() {
            if let Some(panel) = self.search_panel() {
                // SAFETY: plain calls on our own controls.
                let _ = unsafe { SetFocus(Some(panel.edit)) };
                send(panel.edit, EM_SETSEL, 0, -1);
            }
            return;
        }
        self.finish_swipe();
        // Saved first, so the index sees the current note as it is.
        self.flush();
        if self.search.get().is_none() {
            let Ok(panel) = Panel::create(self.hwnd, self.instance) else {
                return;
            };
            let _ = self.search.set(panel);
        }
        let Some(panel) = self.search_panel() else {
            return;
        };
        let _ = self.index.borrow_mut().refresh(&self.dir);
        self.search_open.set(true);
        self.style_search();
        // SAFETY: plain calls on our own windows.
        unsafe {
            let _ = SetWindowTextW(panel.edit, w!(""));
            let _ = ShowWindow(self.editor().hwnd, SW_HIDE);
            let _ = ShowWindow(panel.edit, SW_SHOW);
            let _ = ShowWindow(panel.list, SW_SHOW);
            let _ = SetFocus(Some(panel.edit));
        }
        self.layout_search();
        self.run_search();
        self.update_title();
    }

    /// Esc, or after opening a result: back to the editor.
    pub(super) fn close_search(&self) {
        if !self.search_open.replace(false) {
            return;
        }
        // SAFETY: plain calls on our own windows.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SEARCH);
            if let Some(panel) = self.search_panel() {
                let _ = ShowWindow(panel.edit, SW_HIDE);
                let _ = ShowWindow(panel.list, SW_HIDE);
            }
            let _ = ShowWindow(self.editor().hwnd, SW_SHOW);
            let _ = SetFocus(Some(self.editor().hwnd));
            let _ = InvalidateRect(Some(self.hwnd), None, true);
        }
        self.search_rows.borrow_mut().clear();
        self.update_title();
    }

    /// Fonts, colours and sizes for the theme and DPI.
    pub(super) fn style_search(&self) {
        let Some(panel) = self.search_panel() else {
            return;
        };
        let dpi = self.dpi.get();
        let theme = self.theme.get();
        // SAFETY: GDI objects are created here and the old ones deleted
        // once nothing uses them any more.
        unsafe {
            let fonts = [
                font(10, false, dpi),
                font(10, true, dpi),
                font(11, false, dpi),
            ];
            let brushes = [
                CreateSolidBrush(theme.background),
                CreateSolidBrush(theme.code_background),
            ];
            send(panel.edit, WM_SETFONT, fonts[2].0 as usize, 1);
            let old_fonts = panel.fonts.replace(fonts);
            let old_brushes = panel.brushes.replace(brushes);
            for f in old_fonts.into_iter().filter(|f| !f.is_invalid()) {
                let _ = DeleteObject(f.into());
            }
            for b in old_brushes.into_iter().filter(|b| !b.is_invalid()) {
                let _ = DeleteObject(b.into());
            }

            let hdc = GetDC(Some(panel.list));
            let old = SelectObject(hdc, fonts[0].into());
            let mut tm = TEXTMETRICW::default();
            let _ = GetTextMetricsW(hdc, &mut tm);
            SelectObject(hdc, old);
            ReleaseDC(Some(panel.list), hdc);
            let line = tm.tmHeight + tm.tmExternalLeading;
            panel.line_height.set(line);
            panel.item_height.set(2 * line + scale(10, dpi));
            send(
                panel.list,
                LB_SETITEMHEIGHT,
                0,
                panel.item_height.get() as isize,
            );

            let name = if theme.dark {
                w!("DarkMode_Explorer")
            } else {
                w!("Explorer")
            };
            let _ = SetWindowTheme(panel.list, name, PCWSTR::null());
            let _ = InvalidateRect(Some(panel.edit), None, true);
            let _ = InvalidateRect(Some(panel.list), None, true);
        }
        self.layout_search();
    }

    /// The search box rectangle, as the parent paints it, and the controls.
    fn search_box(&self) -> RECT {
        let dpi = self.dpi.get();
        let mut rc = RECT::default();
        // SAFETY: `rc` is a live local.
        let _ = unsafe { GetClientRect(self.hwnd, &mut rc) };
        let pad = scale(12, dpi);
        let line = self.search_panel().map_or(20, |p| p.line_height.get());
        RECT {
            left: pad,
            top: pad,
            right: (rc.right - pad).max(pad),
            bottom: pad + line * 6 / 5 + scale(14, dpi),
        }
    }

    pub(super) fn layout_search(&self) {
        let Some(panel) = self.search_panel().filter(|_| self.search_open.get()) else {
            return;
        };
        let dpi = self.dpi.get();
        let mut rc = RECT::default();
        let b = self.search_box();
        let inner = scale(8, dpi);
        let text = panel.line_height.get() * 6 / 5;
        // SAFETY: plain calls on our own windows.
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
            let _ = MoveWindow(
                panel.edit,
                b.left + inner,
                b.top + (b.bottom - b.top - text) / 2,
                (b.right - b.left - 2 * inner).max(1),
                text,
                true,
            );
            let top = b.bottom + scale(8, dpi);
            let _ = MoveWindow(panel.list, 0, top, rc.right, (rc.bottom - top).max(0), true);
            let _ = InvalidateRect(Some(self.hwnd), None, true);
        }
    }

    /// Paints the search box behind the edit control, inside WM_ERASEBKGND.
    pub(super) fn paint_search_box(&self, hdc: HDC) {
        let Some(panel) = self.search_panel().filter(|_| self.search_open.get()) else {
            return;
        };
        let b = self.search_box();
        // SAFETY: the brush belongs to the panel and outlives the call.
        unsafe { FillRect(hdc, &b, panel.brushes.get()[1]) };
    }

    /// The text changed: search after a short pause.
    pub(super) fn search_changed(&self) {
        // SAFETY: a timer on our own window; restarting resets its delay.
        unsafe { SetTimer(Some(self.hwnd), TIMER_SEARCH, SEARCH_DELAY_MS, None) };
    }

    pub(super) fn run_search(&self) {
        let Some(panel) = self.search_panel() else {
            return;
        };
        // SAFETY: plain call.
        let _ = unsafe { KillTimer(Some(self.hwnd), TIMER_SEARCH) };
        let started = Instant::now();
        let query = window_text(panel.edit);
        let index = self.index.borrow();
        let rows = index.search(&query, MAX_ROWS);
        let report = format!(
            "whisp: search \"{query}\" {} notes, {} rows, {:.1} ms\n",
            index.len(),
            rows.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        drop(index);
        let count = rows.len();
        *self.search_rows.borrow_mut() = rows;
        send(panel.list, LB_SETCOUNT, count, 0);
        send(panel.list, LB_SETCURSEL, usize::MAX, 0);
        // SAFETY: plain calls.
        unsafe {
            let _ = InvalidateRect(Some(panel.list), None, true);
            OutputDebugStringW(&HSTRING::from(report));
        }
    }

    /// Input while the search is open: keys go to the search box, never to
    /// the editor, and horizontal swipes are ignored.
    pub(super) fn search_intercept(&self, msg: &mut MSG) -> bool {
        let Some(panel) = self.search_panel() else {
            return false;
        };
        match msg.message {
            WM_MOUSEHWHEEL => true,
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP | WM_CHAR | WM_SYSCHAR
            | WM_DEADCHAR | WM_SYSDEADCHAR | WM_UNICHAR | WM_IME_CHAR => {
                // Typing goes to the search box even after a click in the list.
                if msg.hwnd != panel.edit {
                    msg.hwnd = panel.edit;
                    // SAFETY: plain call.
                    let _ = unsafe { SetFocus(Some(panel.edit)) };
                }
                // Ctrl+Backspace makes the plain edit control type a box.
                if msg.message == WM_CHAR && msg.wParam.0 == 0x7f {
                    return true;
                }
                self.search_key(msg)
            }
            _ => false,
        }
    }

    /// Keys while the search is open. Nothing reaches the editor: note
    /// navigation and the other editor shortcuts do nothing here.
    pub(super) fn search_key(&self, msg: &MSG) -> bool {
        let Some(panel) = self.search_panel() else {
            return false;
        };
        if msg.message != WM_KEYDOWN {
            return false;
        }
        // SAFETY: plain calls.
        let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
        if down(VK_MENU) {
            return false;
        }
        let key = VIRTUAL_KEY(msg.wParam.0 as u16);
        let ctrl = down(VK_CONTROL);
        let page = (client_height(panel.list) / panel.item_height.get().max(1)).max(1);
        match key {
            VK_ESCAPE => self.close_search(),
            VK_RETURN => {
                let selected = send(panel.list, LB_GETCURSEL, 0, 0);
                self.open_row(selected.max(0) as usize);
            }
            VK_UP => self.move_selection(-1),
            VK_DOWN => self.move_selection(1),
            VK_PRIOR => self.move_selection(-page),
            VK_NEXT => self.move_selection(page),
            VK_A | VK_F if ctrl => {
                // SAFETY: plain call.
                let _ = unsafe { SetFocus(Some(panel.edit)) };
                send(panel.edit, EM_SETSEL, 0, -1);
            }
            // The search box handles these itself.
            VK_C | VK_V | VK_X | VK_Z => return false,
            // Other Ctrl + character keys would type control characters.
            _ if ctrl => return is_character_key(key),
            _ => return false,
        }
        true
    }

    fn move_selection(&self, by: i32) {
        let Some(panel) = self.search_panel() else {
            return;
        };
        let count = self.search_rows.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let current = send(panel.list, LB_GETCURSEL, 0, 0) as i32;
        let next = if current < 0 {
            if by > 0 { 0 } else { count - 1 }
        } else {
            (current + by).clamp(0, count - 1)
        };
        send(panel.list, LB_SETCURSEL, next as usize, 0);
    }

    /// A click in the list selects a row: open it once the click is done.
    pub(super) fn search_clicked(&self) {
        if let Some(panel) = self.search_panel() {
            let selected = send(panel.list, LB_GETCURSEL, 0, 0);
            if selected >= 0 {
                // SAFETY: a message to our own window.
                let _ = unsafe {
                    PostMessageW(
                        Some(self.hwnd),
                        WM_SEARCH_OPEN,
                        WPARAM(selected as usize),
                        LPARAM(0),
                    )
                };
            }
        }
    }

    /// Opens the note of row `index` and selects its match, following the
    /// navigation rules: nothing moves unless the current note is saved.
    pub(super) fn open_row(&self, index: usize) {
        let target = match self.search_rows.borrow().get(index) {
            Some(Row::Note { path, .. }) => (path.clone(), None),
            Some(Row::Match {
                path, start, end, ..
            }) => (path.clone(), Some((*start, *end))),
            Some(Row::More) | None => return,
        };
        let (path, selection) = target;
        if !self.flush() {
            return;
        }
        if !self.is_current(&path) {
            self.drop_if_blank();
            let position = self.position_of(&path);
            self.show(position);
        }
        self.close_search();
        if let Some((start, end)) = selection {
            self.editor().select(start, end);
        }
    }

    fn is_current(&self, path: &Path) -> bool {
        let notes = self.notes.borrow();
        notes
            .get(self.current.get())
            .is_some_and(|p| p.file_name() == path.file_name())
    }

    /// The index of `path` in the note list. A note made since startup
    /// (by another program, say) is added after the newest note: it is newer
    /// than every note listed at startup, so the oldest first order holds.
    fn position_of(&self, path: &Path) -> usize {
        let mut notes = self.notes.borrow_mut();
        if let Some(i) = notes.iter().position(|p| p.file_name() == path.file_name()) {
            return i;
        }
        notes.push(path.to_path_buf());
        notes.len() - 1
    }

    pub(super) fn search_color(&self, hdc: HDC, control: HWND) -> Option<LRESULT> {
        let panel = self.search_panel()?;
        let theme = self.theme.get();
        let [background, box_background] = panel.brushes.get();
        // SAFETY: hdc is the control's DC for this paint.
        unsafe {
            SetTextColor(hdc, theme.text);
            if control == panel.edit {
                SetBkColor(hdc, theme.code_background);
                Some(LRESULT(box_background.0 as isize))
            } else if control == panel.list {
                SetBkColor(hdc, theme.background);
                Some(LRESULT(background.0 as isize))
            } else {
                None
            }
        }
    }

    pub(super) fn draw_search_row(&self, item: &DRAWITEMSTRUCT) {
        let Some(panel) = self.search_panel() else {
            return;
        };
        let theme = self.theme.get();
        let dpi = self.dpi.get();
        let hdc = item.hDC;
        let rc = item.rcItem;
        let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
        let [normal, bold, _] = panel.fonts.get();
        let line = panel.line_height.get();
        let pad = scale(12, dpi);
        let top = rc.top + scale(5, dpi);
        let line_rect = |n: i32, indent: i32| RECT {
            left: rc.left + pad + indent,
            top: top + n * line,
            right: rc.right - pad,
            bottom: top + (n + 1) * line,
        };
        // SAFETY: drawing into the DC the list box gave us for this item;
        // the previous font is put back.
        unsafe {
            let brush = CreateSolidBrush(if selected {
                theme.selection
            } else {
                theme.background
            });
            FillRect(hdc, &rc, brush);
            let _ = DeleteObject(brush.into());
            SetBkMode(hdc, TRANSPARENT);
            let old = SelectObject(hdc, normal.into());
            let rows = self.search_rows.borrow();
            match rows.get(item.itemID as usize) {
                Some(Row::Note { title, excerpt, .. }) => {
                    draw_text(hdc, bold, title, line_rect(0, 0), theme.text);
                    draw_text(hdc, normal, excerpt, line_rect(1, 0), theme.quote);
                }
                Some(Row::Match { title, snippet, .. }) => {
                    let rect = match title {
                        Some(title) => {
                            draw_text(hdc, bold, title, line_rect(0, 0), theme.text);
                            line_rect(1, 0)
                        }
                        // Further matches of the same note: indented, centred.
                        None => {
                            let mut r = line_rect(0, scale(16, dpi));
                            let shift = (rc.bottom - rc.top - line) / 2 - (top - rc.top);
                            r.top += shift;
                            r.bottom += shift;
                            r
                        }
                    };
                    SelectObject(hdc, normal.into());
                    let mut x = rect.left;
                    x += draw_part(hdc, &snippet.before, x, rect, theme.quote, None);
                    x += draw_part(
                        hdc,
                        &snippet.matched,
                        x,
                        rect,
                        MATCH_TEXT,
                        Some(MATCH_BACKGROUND),
                    );
                    draw_text(
                        hdc,
                        normal,
                        &snippet.after,
                        RECT { left: x, ..rect },
                        theme.quote,
                    );
                }
                Some(Row::More) => {
                    let mut r = line_rect(0, 0);
                    let shift = (rc.bottom - rc.top - line) / 2 - (top - rc.top);
                    r.top += shift;
                    r.bottom += shift;
                    draw_text(
                        hdc,
                        normal,
                        "More results: refine the search",
                        r,
                        theme.quote,
                    );
                }
                None => {}
            }
            SelectObject(hdc, old);
        }
    }
}

/// One line of text in `rect`, cut with `…` if too long.
fn draw_text(hdc: HDC, font: HFONT, text: &str, rect: RECT, color: COLORREF) {
    if rect.right <= rect.left {
        return;
    }
    let mut units = utf16(text);
    let mut rect = rect;
    // SAFETY: drawing into a DC we were given; the font stays selected,
    // the caller restores the original.
    unsafe {
        SelectObject(hdc, font.into());
        SetTextColor(hdc, color);
        DrawTextW(
            hdc,
            &mut units,
            &mut rect,
            DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
}

/// A piece of a snippet at `x`, clipped to `rect`, with an optional
/// background. Returns its width.
fn draw_part(
    hdc: HDC,
    text: &str,
    x: i32,
    rect: RECT,
    color: COLORREF,
    background: Option<COLORREF>,
) -> i32 {
    let units = utf16(text);
    if units.is_empty() || x >= rect.right {
        return 0;
    }
    let mut size = SIZE::default();
    // SAFETY: drawing into a DC we were given, clipped to `clip`.
    unsafe {
        let _ = GetTextExtentPoint32W(hdc, &units, &mut size);
        let clip = RECT {
            left: x,
            right: (x + size.cx).min(rect.right),
            ..rect
        };
        SetTextColor(hdc, color);
        let mut options = ETO_CLIPPED;
        if let Some(background) = background {
            SetBkColor(hdc, background);
            options |= ETO_OPAQUE;
        }
        let _ = ExtTextOutW(
            hdc,
            x,
            rect.top,
            options,
            Some(&clip),
            PCWSTR(units.as_ptr()),
            units.len() as u32,
            None,
        );
    }
    size.cx
}

fn window_text(hwnd: HWND) -> String {
    // SAFETY: the buffer holds the length the window reports, plus the NUL.
    unsafe {
        let len = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let n = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
        String::from_utf16_lossy(&buf[..n.min(len)])
    }
}

fn client_height(hwnd: HWND) -> i32 {
    let mut rc = RECT::default();
    // SAFETY: `rc` is a live local.
    let _ = unsafe { GetClientRect(hwnd, &mut rc) };
    rc.bottom - rc.top
}
