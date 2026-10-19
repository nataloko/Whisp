//! Editing keys and note actions ported from Whisp: the wrap keys, list
//! note keys, plain and link paste, copying, exporting and bumping a note,
//! Ctrl+click on links, and changing note with the wheel at a note's edge.

use std::cell::Cell;
use std::fs::File;
use std::time::SystemTime;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOLDERID_Documents, FileSaveDialog, IFileSaveDialog, IShellItem, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{GUID, HRESULT, HSTRING, Interface, PCSTR, PCWSTR, s, w};

use super::{App, Nav, canvas};
use crate::markup::{self, Wrap};
use crate::{expand, lists, notes};

const CF_UNICODETEXT: u32 = 13;
/// Mouse message key flags.
const MK_SHIFT: usize = 0x0004;
const MK_CONTROL: usize = 0x0008;
/// A wheel notch, as opposed to a touchpad's small deltas.
const WHEEL_NOTCH: i32 = 120;
/// Whisp's gap between wheel ticks that starts a new scroll motion.
const WHEEL_MOTION_GAP_MS: u64 = 150;

thread_local! {
    /// The time of the last wheel notch, and whether its motion changes note.
    static WHEEL: Cell<(u64, bool)> = const { Cell::new((0, false)) };
}

impl App {
    /// Ctrl+B, Ctrl+I, Ctrl+U and Ctrl+Shift+X, as Whisp's `wrap_text`.
    pub(super) fn wrap_key(&self, marker: &str) {
        let editor = self.editor();
        let units = editor.text_utf16();
        match markup::wrap(&units, editor.selection(), marker) {
            Wrap::Caret(at) => editor.select(at, at),
            Wrap::Edit {
                start,
                end,
                text,
                select,
            } => {
                editor.replace_range(start, end, &text);
                editor.select(select.0, select.1);
            }
        }
    }

    /// Whether the note's first line makes it a list note; read cheaply,
    /// as Backspace and `#` ask on every press.
    fn is_list_note(&self) -> bool {
        lists::is_list_note(&self.editor().first_line())
    }

    /// Backspace on a line that is only a box, in a list note.
    pub(super) fn backspace_key(&self) -> bool {
        self.is_list_note() && self.editor().list_action(lists::backspace_box, true)
    }

    /// `#` typed after an empty box in a list note replaces the box.
    pub(super) fn hash_key(&self) -> bool {
        self.is_list_note() && self.editor().list_action(lists::hash_after_box, true)
    }

    /// Ctrl+Backspace: nothing at a line start, a lone bullet or box goes,
    /// otherwise RichEdit's word delete.
    pub(super) fn ctrl_backspace_key(&self) -> bool {
        self.editor().list_action(lists::ctrl_backspace, true)
    }

    /// Ctrl+V and Shift+Insert with text: a long URL goes in as a short
    /// Markdown link, other text as plain text, as Whisp's smart paste.
    pub(super) fn paste_text(&self) {
        let editor = self.editor();
        match editor
            .clipboard_text()
            .as_deref()
            .and_then(markup::shorten_url)
        {
            Some(link) => editor.insert_plain(&link),
            None => editor.paste_plain(),
        }
    }

    /// Ctrl+Shift+V: the clipboard text without Markdown.
    pub(super) fn paste_stripped(&self) {
        let editor = self.editor();
        if let Some(text) = editor.clipboard_text() {
            editor.insert_plain(&markup::strip_markdown(&notes::to_lf(&text)));
        }
    }

    /// Ctrl+L: the selected URL, or the one around the caret, becomes a
    /// short Markdown link.
    pub(super) fn shorten_link(&self) {
        let editor = self.editor();
        let units = editor.text_utf16();
        let (mut start, mut end) = editor.selection();
        if start == end {
            (start, end) = expand::token_bounds(&units, start);
        }
        let url = String::from_utf16_lossy(&units[start..end]);
        if let Some(link) = markup::shorten_url(&url) {
            editor.replace_range(start, end, &link);
        }
    }

    /// Ctrl+Shift+C: the whole note on the clipboard, with Windows line
    /// breaks.
    pub(super) fn copy_note(&self) {
        let text = self.editor().text().replace('\n', "\r\n");
        let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let bytes = units.len() * 2;
        // SAFETY: the memory is filled while locked and handed to the
        // clipboard, which owns it once SetClipboardData succeeds.
        let copied = unsafe {
            let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, bytes) else {
                return;
            };
            let ptr = GlobalLock(mem) as *mut u16;
            if ptr.is_null() {
                let _ = GlobalFree(Some(mem));
                return;
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), ptr, units.len());
            let _ = GlobalUnlock(mem);
            let mut copied = false;
            if OpenClipboard(Some(self.hwnd)).is_ok() {
                let _ = EmptyClipboard();
                copied = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0))).is_ok();
                let _ = CloseClipboard();
            }
            if !copied {
                let _ = GlobalFree(Some(mem));
            }
            copied
        };
        self.show_status(if copied {
            "Note copied"
        } else {
            "Could not copy the note"
        });
    }

    /// Ctrl+Shift+S: saves the note as a Markdown file through the Windows
    /// save dialog (IFileSaveDialog), starting in Documents with Whisp's
    /// file name.
    pub(super) fn export_note(&self) {
        self.finish_swipe();
        self.hide_suggestions();
        let text = self.editor().text();
        let Some(path) = save_dialog(self.hwnd, &markup::export_name(&text)) else {
            return;
        };
        let saved = notes::write_atomic(std::path::Path::new(&path), &text).is_ok();
        self.show_status(if saved {
            "Note exported"
        } else {
            "Could not export the note"
        });
    }

    /// Ctrl+Shift+B: saves the note, touches its modified time and moves it
    /// to just before the empty note, as the newest, as Whisp does.
    pub(super) fn bump_note(&self) {
        // TODO(pinned notes, batch 3): pinned notes keep their place.
        let current = self.current.get();
        let Some(path) = self.notes.borrow().get(current).cloned() else {
            return;
        };
        if !self.flush() {
            return;
        }
        let touched = File::options()
            .write(true)
            .open(&path)
            .and_then(|f| f.set_modified(SystemTime::now()));
        if touched.is_err() {
            self.show_status("Could not move the note");
            return;
        }
        let last = {
            let mut notes = self.notes.borrow_mut();
            let note = notes.remove(current);
            notes.push(note);
            notes.len() - 1
        };
        // Same note in the same editor: only its place in the list changes.
        self.current.set(last);
        self.peek.set(None);
        self.show_status("Note moved to front");
    }

    /// Alt+F and Alt+L: the first note, and the last page, which is the
    /// empty note, as in Whisp's carousel.
    pub(super) fn go_first(&self) {
        self.go(Nav::First);
    }

    /// The http(s) link at the client point of the current editor.
    fn link_at(&self, x: i32, y: i32) -> Option<String> {
        let editor = self.editor();
        let offset = editor.char_from_point(x, y);
        let (line, column) = editor.line_around(offset);
        markup::url_at(&line, column)
    }

    /// Ctrl+click: opens the link under the pointer in the browser. Returns
    /// whether it was a link, so the click is used up.
    pub(super) fn ctrl_click(&self, msg: &MSG) -> bool {
        if msg.hwnd != self.editor().hwnd || msg.wParam.0 & MK_CONTROL == 0 {
            return false;
        }
        let (x, y) = (
            msg.lParam.0 as i16 as i32,
            (msg.lParam.0 >> 16) as i16 as i32,
        );
        let Some(url) = self.link_at(x, y) else {
            return false;
        };
        let Some(execute) = shell32(s!("ShellExecuteW")) else {
            return true;
        };
        // SAFETY: ShellExecuteW's signature; `url` is an http(s) URL without
        // spaces, so the shell opens it in the browser and nothing else.
        unsafe {
            let execute: ShellExecute = std::mem::transmute(execute);
            let url = HSTRING::from(url);
            execute(
                self.hwnd,
                w!("open"),
                PCWSTR(url.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL.0,
            );
        }
        true
    }

    /// For WM_SETCURSOR on an editor: a hand over a link while Ctrl is held.
    pub(super) fn link_cursor(&self, hwnd: HWND) -> bool {
        // SAFETY: plain calls.
        let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
        if !ctrl || self.search_open.get() || hwnd != self.editor().hwnd {
            return false;
        }
        let mut point = POINT::default();
        // SAFETY: plain calls with a live local.
        unsafe {
            if GetCursorPos(&mut point).is_err() || !ScreenToClient(hwnd, &mut point).as_bool() {
                return false;
            }
        }
        if self.link_at(point.x, point.y).is_none() {
            return false;
        }
        // SAFETY: a system cursor.
        unsafe {
            if let Ok(hand) = LoadCursorW(None, IDC_HAND) {
                SetCursor(Some(hand));
            }
        }
        true
    }

    /// Ctrl pressed or released: refresh the pointer over the editor.
    pub(super) fn refresh_cursor(&self) {
        let editor = self.editor().hwnd;
        let mut point = POINT::default();
        // SAFETY: plain calls; WM_SETCURSOR asks the editor (and the hook)
        // to set the pointer again.
        unsafe {
            if GetCursorPos(&mut point).is_ok() && WindowFromPoint(point) == editor {
                let hit = (HTCLIENT as usize) | ((WM_MOUSEMOVE as usize) << 16);
                SendMessageW(
                    editor,
                    WM_SETCURSOR,
                    Some(WPARAM(editor.0 as usize)),
                    Some(LPARAM(hit as isize)),
                );
            }
        }
    }

    /// A vertical wheel notch on the current editor, as Whisp's
    /// `on_carousel_scroll`: a scroll motion that starts at the note's top
    /// (wheel up) or bottom (wheel down) changes note, and the rest of that
    /// motion is used up. Touchpad scrolling and Ctrl or Shift with the
    /// wheel are left alone. Returns whether the notch was used.
    pub(super) fn wheel_edge(&self, msg: &MSG) -> bool {
        if msg.hwnd != self.editor().hwnd {
            return false;
        }
        let keys = msg.wParam.0 & 0xffff;
        let delta = i32::from((msg.wParam.0 >> 16) as u16 as i16);
        if keys & (MK_CONTROL | MK_SHIFT) != 0 || delta == 0 || delta % WHEEL_NOTCH != 0 {
            return false;
        }
        let now = self.epoch.elapsed().as_millis() as u64;
        let (last, changes) = WHEEL.with(Cell::get);
        let new_motion = now.saturating_sub(last) > WHEEL_MOTION_GAP_MS;
        if !new_motion {
            WHEEL.with(|w| w.set((now, changes)));
            return changes;
        }
        let (top, bottom) = self.editor().scroll_edges();
        let newer = delta < 0;
        let at_edge = if newer { bottom } else { top };
        WHEEL.with(|w| w.set((now, at_edge)));
        if at_edge {
            let before = self.generation.get();
            self.go(if newer { Nav::Newer } else { Nav::Older });
            // Land on the edge we came from, so wheeling back returns.
            if newer && self.generation.get() != before {
                self.editor().select(0, 0);
            }
        }
        at_edge
    }
}

/// Shows the save dialog; the chosen path, or `None` when cancelled.
fn save_dialog(owner: HWND, name: &str) -> Option<String> {
    // SAFETY: COM calls on this thread, which is (or becomes) a single
    // threaded apartment; the returned path is freed with CoTaskMemFree.
    unsafe {
        // Once is enough; later calls only report that it is done.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let types = [COMDLG_FILTERSPEC {
            pszName: w!("Markdown"),
            pszSpec: w!("*.md"),
        }];
        dialog.SetFileTypes(&types).ok()?;
        let _ = dialog.SetFileTypeIndex(1);
        let _ = dialog.SetDefaultExtension(w!("md"));
        let _ = dialog.SetFileName(&HSTRING::from(name));
        if let Some(documents) = documents_folder() {
            // The first time; afterwards Windows remembers the last folder.
            let _ = dialog.SetDefaultFolder(&documents);
        }
        dialog.Show(Some(owner)).ok()?;
        let item = dialog.GetResult().ok()?;
        let raw = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = raw.to_string().ok();
        CoTaskMemFree(Some(raw.0 as *const _));
        path
    }
}

type ShellExecute =
    unsafe extern "system" fn(HWND, PCWSTR, PCWSTR, PCWSTR, PCWSTR, i32) -> HINSTANCE;
type KnownFolderItem = unsafe extern "system" fn(
    *const GUID,
    i32,
    HANDLE,
    *const GUID,
    *mut *mut core::ffi::c_void,
) -> HRESULT;

/// A function from shell32.dll, loaded on first use rather than at startup.
fn shell32(name: PCSTR) -> Option<unsafe extern "system" fn() -> isize> {
    // SAFETY: loading a system DLL and looking up an export.
    unsafe {
        let module = LoadLibraryW(w!("shell32.dll")).ok()?;
        GetProcAddress(module, name)
    }
}

/// The user's Documents folder as a shell item.
fn documents_folder() -> Option<IShellItem> {
    let function = shell32(s!("SHGetKnownFolderItem"))?;
    let mut raw = std::ptr::null_mut();
    // SAFETY: SHGetKnownFolderItem's signature; on success `raw` holds an
    // owned IShellItem.
    unsafe {
        let function: KnownFolderItem = std::mem::transmute(function);
        // KF_FLAG_DEFAULT is 0.
        function(
            &FOLDERID_Documents,
            0,
            HANDLE::default(),
            &IShellItem::IID,
            &mut raw,
        )
        .ok()
        .ok()?;
        Some(IShellItem::from_raw(raw))
    }
}

/// Lets the paper subclass ask for the link pointer.
pub(super) fn install_cursor_hook() {
    canvas::set_cursor_hook(|hwnd| super::app().is_some_and(|app| app.link_cursor(hwnd)));
}
