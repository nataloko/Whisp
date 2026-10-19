//! The RichEdit control: plain text in and out, Markdown highlighting and
//! plain text paste.

use std::cell::RefCell;
use std::ffi::c_void;
use std::time::Instant;

use windows::Win32::Foundation::{COLORREF, HGLOBAL, HINSTANCE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::UI::Controls::RichEdit::*;
use windows::Win32::UI::Controls::{
    EM_EMPTYUNDOBUFFER, EM_REPLACESEL, EM_SCROLLCARET, EM_SETRECT, SetWindowTheme,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, IUnknown, Interface, PCWSTR, w};

use super::theme::Theme;
use crate::lines::{self, Line};
use crate::{markdown, notes, rtf};

const CF_UNICODETEXT: u32 = 13;
/// Base font size in twips (1/20 pt): 11 pt.
const FONT_SIZE: i32 = 220;
/// H1 to H3 sizes in twips.
const HEADING_SIZES: [i32; 3] = [300, 260, 240];
pub struct Editor {
    pub hwnd: HWND,
    /// Text Object Model access, to style ranges without moving the
    /// selection and to keep formatting out of the undo stack.
    doc: Option<ITextDocument>,
    /// Signatures of the lines as currently styled.
    styled: RefCell<Vec<u64>>,
}

/// SendMessageW for RichEdit messages.
///
/// # Safety
/// Pointers passed in `wparam` or `lparam` must be valid for the message.
unsafe fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
}

impl Editor {
    pub fn create(parent: HWND, instance: HINSTANCE) -> windows::core::Result<Editor> {
        let style = (WS_CHILD | WS_VISIBLE | WS_VSCROLL).0 | (ES_MULTILINE | ES_AUTOVSCROLL) as u32;
        // SAFETY: plain window creation and messages without pointers, except
        // EM_GETOLEINTERFACE which writes one pointer to a live local.
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("RICHEDIT50W"),
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
            send(hwnd, EM_EXLIMITTEXT, 0, 0x7fff_fffe);
            // Wrap lines at the window edge.
            send(hwnd, EM_SETTARGETDEVICE, 0, 0);
            send(hwnd, EM_SETEVENTMASK, 0, ENM_CHANGE as isize);
            // Before any text goes in: text streamed into an empty control
            // takes the default format.
            let mut cf = format();
            set_face(&mut cf);
            send(
                hwnd,
                EM_SETCHARFORMAT,
                SCF_DEFAULT as usize,
                &cf as *const _ as isize,
            );

            let mut ole: *mut c_void = std::ptr::null_mut();
            send(hwnd, EM_GETOLEINTERFACE, 0, &mut ole as *mut _ as isize);
            let doc = if ole.is_null() {
                None
            } else {
                // EM_GETOLEINTERFACE hands over a reference to IRichEditOle.
                IUnknown::from_raw(ole).cast::<ITextDocument>().ok()
            };
            Ok(Editor {
                hwnd,
                doc,
                styled: RefCell::new(Vec::new()),
            })
        }
    }

    /// Inner padding, in pixels for `dpi`.
    pub fn set_padding(&self, dpi: u32) {
        let pad = (12 * dpi / 96) as i32;
        let mut rc = RECT::default();
        // SAFETY: EM_SETRECT reads the RECT during the call.
        unsafe {
            if GetClientRect(self.hwnd, &mut rc).is_err()
                || rc.right <= 2 * pad
                || rc.bottom <= 2 * pad
            {
                return;
            }
            rc.left += pad;
            rc.top += pad;
            rc.right -= pad;
            rc.bottom -= pad;
            send(self.hwnd, EM_SETRECT, 0, &rc as *const RECT as isize);
        }
    }

    pub fn apply_theme(&self, theme: &Theme) {
        // SAFETY: no pointers beyond static strings.
        unsafe {
            send(self.hwnd, EM_SETBKGNDCOLOR, 0, theme.background.0 as isize);
            // Dark or light scroll bar.
            let name = if theme.dark {
                w!("DarkMode_Explorer")
            } else {
                w!("Explorer")
            };
            let _ = SetWindowTheme(self.hwnd, name, PCWSTR::null());
        }
    }

    /// Length in UTF-16 units, each paragraph break counting as one.
    pub fn len(&self) -> usize {
        let info = GETTEXTLENGTHEX {
            flags: GTL_NUMCHARS | GTL_PRECISE,
            codepage: 1200,
        };
        // SAFETY: `info` outlives the call.
        unsafe {
            send(self.hwnd, EM_GETTEXTLENGTHEX, &info as *const _ as usize, 0).max(0) as usize
        }
    }

    /// The text as RichEdit stores it: UTF-16 with `\r` paragraph breaks.
    fn text_utf16(&self) -> Vec<u16> {
        let len = self.len();
        let mut buf = vec![0u16; len + 1];
        let info = GETTEXTEX {
            cb: (buf.len() * 2) as u32,
            flags: GT_DEFAULT,
            codepage: 1200,
            ..Default::default()
        };
        // SAFETY: `buf` holds `info.cb` bytes and outlives the call.
        let n = unsafe {
            send(
                self.hwnd,
                EM_GETTEXTEX,
                &info as *const _ as usize,
                buf.as_mut_ptr() as isize,
            )
        };
        buf.truncate((n.max(0) as usize).min(len));
        buf
    }

    /// The text with `\n` line endings.
    pub fn text(&self) -> String {
        notes::to_lf(&String::from_utf16_lossy(&self.text_utf16()))
    }

    /// Loads a note: its text and highlighting in one RTF stream, with no
    /// undo history, no change notifications and the caret at the end.
    pub fn load(&self, text: &str, theme: &Theme) {
        let spans = markdown::highlight(text);
        let expected = to_cr(text);
        let look = look(theme);
        let document = rtf::document(text, &spans, &look);
        let lines = lines::lines(&expected, &spans);
        // SAFETY: messages without pointers, or with pointers to live locals.
        unsafe {
            let mask = send(self.hwnd, EM_SETEVENTMASK, 0, 0);
            // Also sets the default format, used when typing into an empty note.
            self.reset_format(theme);
            self.stream_in(document.as_bytes(), SF_RTF);
            if !self.has_text(&expected) {
                // FALLBACK: the RTF did not reproduce the text exactly. Load it
                // as plain text, which cannot alter it, and style every line.
                OutputDebugStringW(w!(
                    "whisp: RTF load did not match the note, loaded as plain text\n"
                ));
                let bytes: Vec<u8> = expected.iter().flat_map(|u| u.to_le_bytes()).collect();
                self.stream_in(&bytes, SF_TEXT | SF_UNICODE);
                self.style_lines(&lines, &look);
            }
            *self.styled.borrow_mut() = lines.iter().map(|l| l.signature).collect();
            send(self.hwnd, EM_EMPTYUNDOBUFFER, 0, 0);
            let end = self.len() as i32;
            let caret = CHARRANGE {
                cpMin: end,
                cpMax: end,
            };
            send(self.hwnd, EM_EXSETSEL, 0, &caret as *const _ as isize);
            send(self.hwnd, EM_SCROLLCARET, 0, 0);
            send(self.hwnd, EM_SETEVENTMASK, 0, mask);
        }
    }

    /// Whether the editor holds exactly `expected` (UTF-16, `\r` breaks).
    fn has_text(&self, expected: &[u16]) -> bool {
        let actual = self.text_utf16();
        if actual == expected {
            return true;
        }
        // RichEdit may merge a final \par into the paragraph mark every
        // document ends with; put the line break back and check again.
        let cr = u16::from(b'\r');
        if expected.len() == actual.len() + 1
            && expected.ends_with(&[cr])
            && expected.starts_with(&actual)
        {
            let end = actual.len() as i32;
            let caret = CHARRANGE {
                cpMin: end,
                cpMax: end,
            };
            // SAFETY: `caret` outlives the call.
            unsafe { send(self.hwnd, EM_EXSETSEL, 0, &caret as *const _ as isize) };
            self.replace_selection(&[cr]);
            return self.text_utf16() == expected;
        }
        false
    }

    /// Replaces the whole text with a stream in `format`.
    fn stream_in(&self, data: &[u8], format: u32) {
        let mut source = StreamSource { data, pos: 0 };
        let stream = EDITSTREAM {
            dwCookie: &mut source as *mut StreamSource as usize,
            dwError: 0,
            pfnCallback: Some(read_source),
        };
        // SAFETY: `stream` and `source` outlive EM_STREAMIN, which calls
        // `read_source` synchronously.
        unsafe {
            send(
                self.hwnd,
                EM_STREAMIN,
                format as usize,
                &stream as *const _ as isize,
            )
        };
    }

    /// Restyles the lines whose text or styles changed since they were last
    /// styled, without moving the selection. Formatting stays out of the
    /// undo stack and sends no change notifications.
    pub fn restyle_changes(&self, theme: &Theme) {
        let started = Instant::now();
        let units = self.text_utf16();
        let spans = markdown::highlight(&String::from_utf16_lossy(&units));
        let lines = lines::lines(&units, &spans);
        let signatures: Vec<u64> = lines.iter().map(|l| l.signature).collect();
        let changed = lines::changed(&self.styled.borrow(), &signatures);
        let runs = self.style_lines(&lines[changed.clone()], &look(theme));
        *self.styled.borrow_mut() = signatures;
        let report = format!(
            "whisp: restyle {} lines, {runs} runs, {:.2} ms\n",
            changed.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        // SAFETY: `report` outlives the call.
        unsafe { OutputDebugStringW(&HSTRING::from(report)) };
    }

    /// Restyles everything after a theme or DPI change by reloading the text
    /// as RTF, keeping the selection and scroll position. This clears the
    /// undo history.
    pub fn restyle(&self, theme: &Theme) {
        let text = self.text();
        let mut sel = CHARRANGE::default();
        let mut scroll = POINT::default();
        // SAFETY: pointers to live locals.
        unsafe {
            send(self.hwnd, EM_EXGETSEL, 0, &mut sel as *mut _ as isize);
            send(
                self.hwnd,
                EM_GETSCROLLPOS,
                0,
                &mut scroll as *mut _ as isize,
            );
            self.load(&text, theme);
            send(self.hwnd, EM_EXSETSEL, 0, &sel as *const _ as isize);
            send(self.hwnd, EM_SETSCROLLPOS, 0, &scroll as *const _ as isize);
        }
    }

    /// Resets `lines` to the base format, then applies their runs, through
    /// TOM ranges. Returns the number of runs applied.
    fn style_lines(&self, lines: &[Line], look: &rtf::Look) -> usize {
        let Some(doc) = &self.doc else { return 0 };
        if lines.is_empty() {
            return 0;
        }
        let mut runs = 0;
        // SAFETY: TOM calls on the document of our own control.
        unsafe {
            let mask = send(self.hwnd, EM_SETEVENTMASK, 0, 0);
            let _ = doc.Freeze();
            let _ = doc.Undo(tomSuspend.0);
            for line in lines {
                // Include the line breaks on both sides: they always have the
                // base format, and a new break may have taken a heading's.
                let first = line.start.saturating_sub(1);
                if let Some(font) = font(doc, first, line.start + line.len + 1) {
                    reset(&font, look);
                }
                for &(start, end, attr) in &line.runs {
                    if let Some(font) = font(doc, line.start + start, line.start + end) {
                        apply(&font, attr, look);
                    }
                    runs += 1;
                }
            }
            let _ = doc.Undo(tomResume.0);
            let _ = doc.Unfreeze();
            // Repaint everything rather than trust what Unfreeze redraws.
            let _ = InvalidateRect(Some(self.hwnd), None, false);
            send(self.hwnd, EM_SETEVENTMASK, 0, mask);
        }
        runs
    }

    /// Resets all text to the base format and sets the default format.
    fn reset_format(&self, theme: &Theme) {
        let mut cf = format();
        cf.Base.dwMask = CFM_SIZE
            | CFM_COLOR
            | CFM_BOLD
            | CFM_ITALIC
            | CFM_UNDERLINE
            | CFM_STRIKEOUT
            | CFM_BACKCOLOR;
        cf.Base.dwEffects = CFE_AUTOBACKCOLOR;
        // Rounded to half points like the RTF and TOM sizes.
        cf.Base.yHeight = look(theme).size as i32 * 10;
        cf.Base.crTextColor = theme.text;
        // SAFETY: `cf` outlives both calls.
        unsafe {
            // The face is only set as the default, so RichEdit's own font
            // fallback for scripts Consolas lacks is kept.
            send(
                self.hwnd,
                EM_SETCHARFORMAT,
                SCF_ALL as usize,
                &cf as *const _ as isize,
            );
            set_face(&mut cf);
            send(
                self.hwnd,
                EM_SETCHARFORMAT,
                SCF_DEFAULT as usize,
                &cf as *const _ as isize,
            );
        }
    }

    /// Pastes the clipboard as plain text, as one undoable step.
    pub fn paste_plain(&self) {
        let Some(text) = clipboard_text(self.hwnd) else {
            return;
        };
        let units: Vec<u16> = notes::to_lf(&text)
            .encode_utf16()
            .filter(|&u| u != 0)
            .map(|u| {
                if u == u16::from(b'\n') {
                    u16::from(b'\r')
                } else {
                    u
                }
            })
            .collect();
        if units.is_empty() {
            return;
        }
        // RichEdit reads a string starting with "{\rtf" as RTF, so such text
        // goes in as two pieces.
        let rtf: Vec<u16> = r"{\rtf".encode_utf16().collect();
        if units.starts_with(&rtf) {
            self.replace_selection(&units[..1]);
            self.replace_selection(&units[1..]);
        } else {
            self.replace_selection(&units);
        }
    }

    fn replace_selection(&self, units: &[u16]) {
        let mut text = units.to_vec();
        text.push(0);
        // SAFETY: `text` is NUL terminated and outlives the call.
        unsafe { send(self.hwnd, EM_REPLACESEL, 1, text.as_ptr() as isize) };
    }
}

/// `text` as RichEdit stores it: UTF-16 with `\r` line breaks.
fn to_cr(text: &str) -> Vec<u16> {
    text.encode_utf16()
        .map(|u| {
            if u == u16::from(b'\n') {
                u16::from(b'\r')
            } else {
                u
            }
        })
        .collect()
}

fn rgb(c: COLORREF) -> rtf::Rgb {
    (c.0 as u8, (c.0 >> 8) as u8, (c.0 >> 16) as u8)
}

/// The theme and sizes for RTF. Sizes are in half points, rounded. They are
/// not scaled for DPI: RichEdit scales text for the monitor itself.
fn look(theme: &Theme) -> rtf::Look {
    let half_points = |size: i32| ((size + 5) / 10).max(1) as u32;
    rtf::Look {
        text: rgb(theme.text),
        symbol: rgb(theme.symbol),
        heading: rgb(theme.heading),
        link: rgb(theme.link),
        list: rgb(theme.list),
        checked: rgb(theme.checked),
        comment: rgb(theme.comment),
        quote: rgb(theme.quote),
        code_background: rgb(theme.code_background),
        size: half_points(FONT_SIZE),
        heading_sizes: HEADING_SIZES.map(half_points),
    }
}

fn format() -> CHARFORMAT2W {
    let mut cf = CHARFORMAT2W::default();
    cf.Base.cbSize = size_of::<CHARFORMAT2W>() as u32;
    cf
}

fn set_face(cf: &mut CHARFORMAT2W) {
    cf.Base.dwMask |= CFM_FACE;
    for (dst, src) in cf.Base.szFaceName.iter_mut().zip("Consolas".encode_utf16()) {
        *dst = src;
    }
}

/// The font of the range from `first` to `limit`; setting its properties
/// formats the range.
fn font(doc: &ITextDocument, first: usize, limit: usize) -> Option<ITextFont> {
    // SAFETY: plain TOM calls on a live document.
    unsafe {
        doc.Range(first as i32, limit as i32)
            .and_then(|r| r.GetFont())
            .ok()
    }
}

fn colorref((r, g, b): rtf::Rgb) -> i32 {
    i32::from(r) | i32::from(g) << 8 | i32::from(b) << 16
}

fn points(half_points: u32) -> f32 {
    half_points as f32 / 2.0
}

/// The base format, as `reset_format` sets it.
fn reset(font: &ITextFont, look: &rtf::Look) {
    // SAFETY: plain property calls on a live font.
    unsafe {
        let _ = font.SetBold(tomFalse.0);
        let _ = font.SetItalic(tomFalse.0);
        let _ = font.SetStrikeThrough(tomFalse.0);
        let _ = font.SetUnderline(tomNone.0);
        let _ = font.SetForeColor(colorref(look.text));
        let _ = font.SetBackColor(tomAutoColor.0);
        let _ = font.SetSize(points(look.size));
    }
}

/// What `attr` adds to the base format, as in the RTF.
fn apply(font: &ITextFont, attr: rtf::Attr, look: &rtf::Look) {
    // SAFETY: plain property calls on a live font.
    unsafe {
        if attr.bold {
            let _ = font.SetBold(tomTrue.0);
        }
        if attr.italic {
            let _ = font.SetItalic(tomTrue.0);
        }
        if attr.strike {
            let _ = font.SetStrikeThrough(tomTrue.0);
        }
        if attr.underline {
            let _ = font.SetUnderline(tomSingle.0);
        }
        if attr.color != rtf::Attr::default().color {
            let _ = font.SetForeColor(colorref(look.color(attr.color)));
        }
        if attr.code {
            let _ = font.SetBackColor(colorref(look.code_background));
        }
        if attr.heading > 0 {
            let _ = font.SetSize(points(look.heading_sizes[usize::from(attr.heading) - 1]));
        }
    }
}

struct StreamSource<'a> {
    data: &'a [u8],
    pos: usize,
}

unsafe extern "system" fn read_source(cookie: usize, buf: *mut u8, cb: i32, pcb: *mut i32) -> u32 {
    // SAFETY: `cookie` is the StreamSource set up in `stream_in`, alive during
    // EM_STREAMIN; RichEdit provides `cb` writable bytes at `buf`.
    unsafe {
        let source = &mut *(cookie as *mut StreamSource);
        // Whole UTF-16 units only, for plain text streams.
        let n = (source.data.len() - source.pos).min(cb.max(0) as usize & !1);
        std::ptr::copy_nonoverlapping(source.data.as_ptr().add(source.pos), buf, n);
        source.pos += n;
        *pcb = n as i32;
    }
    0
}

fn clipboard_text(owner: HWND) -> Option<String> {
    // SAFETY: the clipboard memory is only read while locked, within its size.
    unsafe {
        OpenClipboard(Some(owner)).ok()?;
        let text = GetClipboardData(CF_UNICODETEXT).ok().and_then(|handle| {
            let mem = HGLOBAL(handle.0);
            let ptr = GlobalLock(mem) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let units = std::slice::from_raw_parts(ptr, GlobalSize(mem) / 2);
            let len = units.iter().position(|&u| u == 0).unwrap_or(units.len());
            let text = String::from_utf16_lossy(&units[..len]);
            let _ = GlobalUnlock(mem);
            Some(text)
        });
        let _ = CloseClipboard();
        text
    }
}
