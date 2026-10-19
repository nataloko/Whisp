//! The `::` expansions and their suggestion popup.
//!
//! The popup is an owner drawn list box, a child of the main window placed
//! above the editor. A child window never activates, and every click on it
//! is handled in the message loop before the list box sees it, so it never
//! takes focus either: typing stays in the editor. The editors have
//! WS_CLIPSIBLINGS, so they do not paint over it.

use std::cell::Cell;

use windows::Win32::Foundation::*;
use windows::Win32::Globalization::{
    DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Security::Cryptography::ProcessPrng;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Controls::DRAWITEMSTRUCT;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use super::{App, scale};
use crate::expand::{self, Env, Expansion};
use crate::notes;

/// Posted after a selection change, so the popup follows the token once
/// RichEdit has finished the keystroke.
pub(super) const WM_SUGGEST: u32 = WM_APP + 5;
/// Popup size limits at 96 DPI, as in Whisp.
const POPUP_WIDTH: i32 = 260;
const POPUP_MAX_HEIGHT: i32 = 300;
/// Characters read around the caret to find the token.
const TOKEN_REACH: usize = 1024;

/// The clock, locale and random source the expansions use.
struct System;

impl System {
    fn now() -> SYSTEMTIME {
        // SAFETY: plain call.
        unsafe { GetLocalTime() }
    }
}

impl Env for System {
    fn date(&mut self, days: i64) -> String {
        let now = System::now();
        let day = (
            i64::from(now.wYear),
            i64::from(now.wMonth),
            i64::from(now.wDay),
        );
        let ((year, month, day), weekday) = expand::add_days(day, days);
        if !(1601..=30827).contains(&year) {
            // Outside what Windows formats.
            return format!("{year}-{month:02}-{day:02}");
        }
        let date = SYSTEMTIME {
            wYear: year as u16,
            wMonth: month as u16,
            wDay: day as u16,
            wDayOfWeek: weekday as u16,
            ..now
        };
        let mut buf = [0u16; 128];
        // SAFETY: `buf` is a local buffer; NULL locale is the user's.
        let n = unsafe {
            GetDateFormatEx(
                PCWSTR::null(),
                DATE_SHORTDATE,
                Some(&date),
                PCWSTR::null(),
                Some(&mut buf),
                PCWSTR::null(),
            )
        };
        String::from_utf16_lossy(&buf[..(n.max(1) as usize - 1)])
    }

    fn time(&mut self) -> String {
        let now = System::now();
        let mut buf = [0u16; 128];
        // SAFETY: `buf` is a local buffer; NULL locale is the user's.
        let n = unsafe {
            GetTimeFormatEx(
                PCWSTR::null(),
                TIME_NOSECONDS,
                Some(&now),
                PCWSTR::null(),
                Some(&mut buf),
            )
        };
        String::from_utf16_lossy(&buf[..(n.max(1) as usize - 1)])
    }

    fn fill(&mut self, bytes: &mut [u8]) {
        // SAFETY: fills the slice; ProcessPrng is the OS's preferred
        // generator (bcryptprimitives.dll) and does not fail.
        let _ = unsafe { ProcessPrng(bytes) };
    }
}

pub(super) struct Popup {
    pub list: HWND,
    /// Bold command font, small description font.
    fonts: Cell<[HFONT; 2]>,
    brush: Cell<HBRUSH>,
    dpi: Cell<u32>,
    lines: Cell<(i32, i32)>,
}

impl Popup {
    fn create(parent: HWND, instance: HINSTANCE) -> windows::core::Result<Popup> {
        let style = WS_CHILD.0
            | WS_BORDER.0
            | WS_VSCROLL.0
            | WS_CLIPSIBLINGS.0
            | (LBS_OWNERDRAWFIXED | LBS_NODATA | LBS_NOINTEGRALHEIGHT) as u32;
        // SAFETY: plain window creation.
        let list = unsafe {
            CreateWindowExW(
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
            )?
        };
        Ok(Popup {
            list,
            fonts: Cell::new([HFONT::default(); 2]),
            brush: Cell::new(HBRUSH::default()),
            dpi: Cell::new(0),
            lines: Cell::new((16, 14)),
        })
    }

    fn item_height(&self) -> i32 {
        let (bold, small) = self.lines.get();
        bold + small + scale(10, self.dpi.get())
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

fn line_height(hdc: HDC, font: HFONT) -> i32 {
    let mut tm = TEXTMETRICW::default();
    // SAFETY: a DC we own for the call; the old font is put back.
    unsafe {
        let old = SelectObject(hdc, font.into());
        let _ = GetTextMetricsW(hdc, &mut tm);
        SelectObject(hdc, old);
    }
    tm.tmHeight + tm.tmExternalLeading
}

fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    // SAFETY: only used with messages that take no pointers.
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
}

impl App {
    fn popup(&self) -> Option<&Popup> {
        if self.suggest.get().is_none() {
            let popup = Popup::create(self.hwnd, self.instance).ok()?;
            let _ = self.suggest.set(popup);
        }
        self.suggest.get()
    }

    /// Fonts, brush and row height for the DPI and theme.
    fn style_popup(&self, popup: &Popup) {
        let dpi = self.dpi.get();
        let theme = self.theme.get();
        // SAFETY: GDI objects are created here and the old ones deleted
        // once nothing uses them.
        unsafe {
            if popup.dpi.get() != dpi {
                let fonts = [font(10, true, dpi), font(9, false, dpi)];
                for f in popup
                    .fonts
                    .replace(fonts)
                    .into_iter()
                    .filter(|f| !f.is_invalid())
                {
                    let _ = DeleteObject(f.into());
                }
                let hdc = GetDC(Some(popup.list));
                popup
                    .lines
                    .set((line_height(hdc, fonts[0]), line_height(hdc, fonts[1])));
                ReleaseDC(Some(popup.list), hdc);
                popup.dpi.set(dpi);
                send(
                    popup.list,
                    LB_SETITEMHEIGHT,
                    0,
                    popup.item_height() as isize,
                );
            }
            let old = popup.brush.replace(CreateSolidBrush(theme.code_background));
            if !old.is_invalid() {
                let _ = DeleteObject(old.into());
            }
            let name = if theme.dark {
                w!("DarkMode_Explorer")
            } else {
                w!("Explorer")
            };
            let _ = windows::Win32::UI::Controls::SetWindowTheme(popup.list, name, PCWSTR::null());
        }
    }

    pub(super) fn hide_suggestions(&self) {
        if self.suggest_open.replace(false)
            && let Some(popup) = self.suggest.get()
        {
            // SAFETY: plain call on our own window.
            let _ = unsafe { ShowWindow(popup.list, SW_HIDE) };
        }
    }

    /// A selection change in the editor: show, update or hide the popup
    /// after RichEdit has finished.
    pub(super) fn selection_changed(&self) {
        if !self.suggest_posted.replace(true) {
            // SAFETY: a message to our own window.
            let posted = unsafe { PostMessageW(Some(self.hwnd), WM_SUGGEST, WPARAM(0), LPARAM(0)) };
            if posted.is_err() {
                self.suggest_posted.set(false);
            }
        }
    }

    /// Shows the commands matching the `::` token before the caret, as
    /// Whisp's `check_autocomplete` does, or hides the popup.
    pub(super) fn update_suggestions(&self) {
        self.suggest_posted.set(false);
        let editor = self.editor();
        let (caret, end) = editor.selection();
        if self.search_open.get() || caret != end {
            self.hide_suggestions();
            return;
        }
        let from = caret.saturating_sub(TOKEN_REACH);
        let units = editor.text_range(from, caret);
        let start = expand::token_bounds(&units, units.len()).0;
        let word = String::from_utf16_lossy(&units[start..]);
        let rows = expand::suggestions(&word);
        if rows.is_empty() {
            self.hide_suggestions();
            return;
        }
        let token_start = from + start;
        let Some((x, top, bottom)) = editor.char_box(token_start) else {
            self.hide_suggestions();
            return;
        };
        let Some(popup) = self.popup() else {
            return;
        };
        self.style_popup(popup);
        let count = rows.len();
        // A command typed in full is the one highlighted, not the first row.
        let lower = word.to_lowercase();
        let exact = rows.iter().position(|&(command, _)| command == lower);
        *self.suggest_rows.borrow_mut() = rows;
        send(popup.list, LB_SETCOUNT, count, 0);
        send(popup.list, LB_SETCURSEL, exact.unwrap_or(0), 0);
        self.place_popup(popup, count, x, top, bottom);
        self.suggest_open.set(true);
    }

    /// Below the token's line, or above it when there is more room there,
    /// and always inside the window.
    fn place_popup(&self, popup: &Popup, count: usize, x: i32, top: i32, bottom: i32) {
        let dpi = self.dpi.get();
        let mut client = RECT::default();
        let mut corner = [POINT { x, y: top }, POINT { x, y: bottom }];
        // SAFETY: plain calls; the points are live locals.
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
            let _ = ScreenToClient(self.hwnd, &mut corner[0]);
            let _ = ScreenToClient(self.hwnd, &mut corner[1]);
        }
        let margin = scale(8, dpi);
        let gap = scale(4, dpi);
        let border = 2;
        let width = scale(POPUP_WIDTH, dpi)
            .min(client.right - 2 * margin)
            .max(1);
        let wanted =
            (popup.item_height() * count as i32 + border).min(scale(POPUP_MAX_HEIGHT, dpi));
        let below = client.bottom - margin - (corner[1].y + gap);
        let above = corner[0].y - gap - margin;
        let (y, height) = if wanted <= below || below >= above {
            (
                corner[1].y + gap,
                wanted.min(below.max(popup.item_height() + border)),
            )
        } else {
            let height = wanted.min(above);
            (corner[0].y - gap - height, height)
        };
        let x = (corner[0].x - margin)
            .min(client.right - margin - width)
            .max(margin);
        // SAFETY: plain call on our own window; it stays inactive.
        unsafe {
            let _ = SetWindowPos(
                popup.list,
                Some(HWND_TOP),
                x,
                y,
                width,
                height,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            );
            let _ = InvalidateRect(Some(popup.list), None, true);
        }
    }

    fn move_suggestion(&self, by: isize) {
        let Some(popup) = self.suggest.get() else {
            return;
        };
        let count = self.suggest_rows.borrow().len() as isize;
        let current = send(popup.list, LB_GETCURSEL, 0, 0).max(0);
        let next = (current + by).clamp(0, (count - 1).max(0));
        send(popup.list, LB_SETCURSEL, next as usize, 0);
    }

    /// Keys while the popup shows: Up and Down choose, Esc hides, Enter,
    /// Tab and Space accept. Returns `None` to let the key go on.
    pub(super) fn suggestion_key(
        &self,
        key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY,
        shift: bool,
    ) -> Option<bool> {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        if !self.suggest_open.get() {
            return None;
        }
        let popup = self.suggest.get()?;
        match key {
            VK_UP => self.move_suggestion(-1),
            VK_DOWN => self.move_suggestion(1),
            VK_ESCAPE => self.hide_suggestions(),
            // Tab completes to the highlighted suggestion, as in most
            // suggestion lists.
            VK_TAB if !shift => {
                let selected = send(popup.list, LB_GETCURSEL, 0, 0).max(0) as usize;
                self.accept_suggestion(selected);
            }
            // Enter and Space run a command typed in full, and otherwise
            // accept the highlighted suggestion. Whisp always accepts the top
            // suggestion, so ::time gave a timestamp and a typed ::roll(4d6)
            // only selected its argument.
            VK_RETURN | VK_SPACE if !shift => {
                // Space right after a bare "::" just types the space, as in Whisp.
                if key == VK_SPACE && self.token_before_caret() == "::" {
                    self.hide_suggestions();
                    return None;
                }
                if !self.expand_at_caret(if key == VK_SPACE { " " } else { "\n" }) {
                    let selected = send(popup.list, LB_GETCURSEL, 0, 0).max(0) as usize;
                    self.accept_suggestion(selected);
                }
            }
            _ => return None,
        }
        Some(true)
    }

    fn token_before_caret(&self) -> String {
        let (caret, _) = self.editor().selection();
        let from = caret.saturating_sub(TOKEN_REACH);
        let units = self.editor().text_range(from, caret);
        let start = expand::token_bounds(&units, units.len()).0;
        String::from_utf16_lossy(&units[start..])
    }

    /// Whisp's `apply_autocomplete` with a space: a command with arguments
    /// goes in with its sample argument selected; any other runs at once.
    fn accept_suggestion(&self, index: usize) {
        let Some(&(command, _)) = self.suggest_rows.borrow().get(index) else {
            return;
        };
        self.hide_suggestions();
        let editor = self.editor();
        let units = editor.text_utf16();
        let (caret, _) = editor.selection();
        let (start, end) = expand::token_bounds(&units, caret);
        if let Some(open) = command.find('(') {
            editor.replace_range(start, caret, command);
            // The command is ASCII, so bytes are UTF-16 units.
            editor.select(start + open + 1, start + command.len() - 1);
            return;
        }
        let tail = String::from_utf16_lossy(&units[caret..end]);
        if !self.run_expansion(&units, start, end, &format!("{command}{tail}"), " ") {
            editor.replace_range(start, caret, &format!("{command} "));
        }
    }

    /// Space or Enter typed after a complete `::command`: expand it, as
    /// Whisp's `handle_expansion`. Returns whether it did.
    pub(super) fn expand_at_caret(&self, typed: &str) -> bool {
        let editor = self.editor();
        let (caret, end) = editor.selection();
        if caret != end {
            return false;
        }
        let units = editor.text_utf16();
        let (start, end) = expand::token_bounds(&units, caret);
        let token = String::from_utf16_lossy(&units[start..end]);
        if !token.starts_with("::") {
            return false;
        }
        let expanded = self.run_expansion(&units, start, end, &token, typed);
        if expanded {
            self.hide_suggestions();
        }
        expanded
    }

    /// Applies the expansion of `token`, found at `start..end` in `units`,
    /// as one replacement: the token for an insert, the changed part of the
    /// note for a text command.
    fn run_expansion(
        &self,
        units: &[u16],
        start: usize,
        end: usize,
        token: &str,
        typed: &str,
    ) -> bool {
        let editor = self.editor();
        let rest_units = [&units[..start], &units[end..]].concat();
        let rest = notes::to_lf(&String::from_utf16_lossy(&rest_units));
        match expand::expand(token, &rest, &mut System) {
            None => false,
            Some(Expansion::Insert(text)) => {
                editor.replace_range(start, end, &format!("{text}{typed}"));
                true
            }
            Some(Expansion::Text(None)) => {
                editor.replace_range(start, end, "");
                true
            }
            Some(Expansion::Text(Some(text))) => {
                let new: Vec<u16> = text
                    .encode_utf16()
                    .map(|u| {
                        if u == u16::from(b'\n') {
                            u16::from(b'\r')
                        } else {
                            u
                        }
                    })
                    .collect();
                let (from, old_to, new_to) = expand::diff(units, &new);
                let replacement = notes::to_lf(&String::from_utf16_lossy(&new[from..new_to]));
                // A plain text stream into the selection: 0.5 s for 254 KB
                // in the VM, where EM_REPLACESEL took 7 to 21 s.
                editor.stream_replace(from, old_to, &replacement);
                let caret = start.min(new.len());
                editor.select(caret, caret);
                true
            }
        }
    }

    /// Mouse input on the popup, handled before the list box sees it so it
    /// never takes focus: a click accepts the row under it.
    pub(super) fn suggestion_mouse(&self, msg: &MSG) -> bool {
        let Some(popup) = self.suggest.get().filter(|p| p.list == msg.hwnd) else {
            return false;
        };
        match msg.message {
            WM_LBUTTONDOWN => {
                let hit = send(popup.list, LB_ITEMFROMPOINT, 0, msg.lParam.0);
                // The high word is 1 when the point is outside every row.
                if hit >> 16 & 0xffff == 0 {
                    self.accept_suggestion((hit & 0xffff) as usize);
                }
                true
            }
            WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_RBUTTONDBLCLK
            | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_XBUTTONDOWN | WM_XBUTTONUP => true,
            _ => false,
        }
    }

    pub(super) fn suggestion_color(&self, hdc: HDC, control: HWND) -> Option<LRESULT> {
        let popup = self.suggest.get().filter(|p| p.list == control)?;
        let theme = self.theme.get();
        // SAFETY: hdc is the list box's DC for this paint.
        unsafe {
            SetTextColor(hdc, theme.text);
            SetBkColor(hdc, theme.code_background);
        }
        Some(LRESULT(popup.brush.get().0 as isize))
    }

    /// Draws one row: the command in bold, its description dimmed below.
    pub(super) fn draw_suggestion(&self, item: &DRAWITEMSTRUCT) -> bool {
        let Some(popup) = self.suggest.get().filter(|p| p.list == item.hwndItem) else {
            return false;
        };
        let theme = self.theme.get();
        let dpi = self.dpi.get();
        let rc = item.rcItem;
        let selected = item.itemState.0 & windows::Win32::UI::Controls::ODS_SELECTED.0 != 0;
        let [bold, small] = popup.fonts.get();
        let (bold_line, small_line) = popup.lines.get();
        let pad = scale(10, dpi);
        let top = rc.top + scale(5, dpi);
        let rows = self.suggest_rows.borrow();
        let Some(&(command, description)) = rows.get(item.itemID as usize) else {
            return true;
        };
        // SAFETY: drawing into the DC the list box gave us for this item;
        // the original font is put back.
        unsafe {
            let brush = CreateSolidBrush(if selected {
                theme.selection
            } else {
                theme.code_background
            });
            FillRect(item.hDC, &rc, brush);
            let _ = DeleteObject(brush.into());
            SetBkMode(item.hDC, TRANSPARENT);
            let old = SelectObject(item.hDC, bold.into());
            let line = |font: HFONT, text: &str, color: COLORREF, y: i32, h: i32| {
                let mut units: Vec<u16> = text.encode_utf16().collect();
                let mut r = RECT {
                    left: rc.left + pad,
                    top: y,
                    right: rc.right - pad,
                    bottom: y + h,
                };
                SelectObject(item.hDC, font.into());
                SetTextColor(item.hDC, color);
                DrawTextW(
                    item.hDC,
                    &mut units,
                    &mut r,
                    DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
                );
            };
            line(bold, command, theme.text, top, bold_line);
            line(small, description, theme.quote, top + bold_line, small_line);
            SelectObject(item.hDC, old);
        }
        true
    }
}
