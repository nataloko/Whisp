//! The main window: note navigation, saving, theme, DPI and shortcuts.

mod editor;
mod theme;

use std::cell::{Cell, OnceCell, RefCell};
use std::fs;
use std::path::PathBuf;

use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

use crate::notes;
use editor::Editor;
use theme::Theme;

const CLASS: PCWSTR = w!("WhispWinWindow");
const TIMER_SAVE: usize = 1;
const TIMER_HIGHLIGHT: usize = 2;
const SAVE_DELAY_MS: u32 = 500;
const SAVE_RETRY_MS: u32 = 2000;
/// Notes longer than this (UTF-16 units) are highlighted after a pause in
/// typing instead of on every change.
const LARGE_NOTE: usize = 100_000;
const HIGHLIGHT_DELAY_MS: u32 = 300;

#[derive(Clone, Copy)]
enum Nav {
    Older,
    Newer,
    /// The empty note after the newest.
    Last,
}

struct App {
    hwnd: HWND,
    editor: Editor,
    dir: PathBuf,
    /// Saved notes, oldest first. Index `notes.len()` is the empty note at
    /// the end, which gets a file once it has text.
    notes: RefCell<Vec<PathBuf>>,
    current: Cell<usize>,
    dirty: Cell<bool>,
    save_failed: Cell<bool>,
    /// Set while the program itself changes the text, so EN_CHANGE is not
    /// taken for an edit.
    busy: Cell<bool>,
    theme: Cell<Theme>,
    dpi: Cell<u32>,
}

thread_local! {
    static APP: OnceCell<&'static App> = const { OnceCell::new() };
}

fn app() -> Option<&'static App> {
    APP.with(|a| a.get().copied())
}

fn scale(px: i32, dpi: u32) -> i32 {
    px * dpi as i32 / 96
}

fn error_box(text: &str) {
    // SAFETY: both strings are valid for the call.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            w!("Whisp"),
            MB_OK | MB_ICONERROR,
        )
    };
}

pub fn run() {
    // SAFETY: Win32 calls on this thread; the strings are static.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // Held until the process exits.
        let _mutex = CreateMutexW(None, true, w!(r"Local\WhispWin.SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(hwnd) = FindWindowW(CLASS, PCWSTR::null()) {
                let _ = ShowWindow(
                    hwnd,
                    if IsIconic(hwnd).as_bool() {
                        SW_RESTORE
                    } else {
                        SW_SHOW
                    },
                );
                let _ = SetForegroundWindow(hwnd);
            }
            return;
        }
    }

    let Some(dir) = notes::notes_dir() else {
        error_box("Cannot find the notes folder: LOCALAPPDATA is not set.");
        return;
    };
    let list = match fs::create_dir_all(&dir).and_then(|()| notes::list(&dir)) {
        Ok(list) => list,
        Err(e) => {
            error_box(&format!(
                "Cannot open the notes folder {}:\n{e}",
                dir.display()
            ));
            return;
        }
    };
    let app = match create(dir, list) {
        Ok(app) => app,
        Err(e) => {
            error_box(&format!("Cannot create the window:\n{e}"));
            return;
        }
    };

    // SAFETY: standard message loop on the thread that owns the window.
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            if app.shortcut(&msg) {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn create(dir: PathBuf, list: Vec<PathBuf>) -> windows::core::Result<&'static App> {
    // SAFETY: window creation on this thread; the class name is static.
    unsafe {
        LoadLibraryW(w!("Msftedit.dll"))?;
        let instance: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&class);
        let sys_dpi = GetDpiForSystem();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("Whisp"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(420, sys_dpi),
            scale(600, sys_dpi),
            None,
            None,
            Some(instance),
            None,
        )?;
        let dpi = GetDpiForWindow(hwnd);
        if dpi != sys_dpi {
            let flags = SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE;
            let _ = SetWindowPos(hwnd, None, 0, 0, scale(420, dpi), scale(600, dpi), flags);
        }
        let editor = Editor::create(hwnd, instance)?;

        let current = list.len().saturating_sub(1);
        let app: &'static App = Box::leak(Box::new(App {
            hwnd,
            editor,
            dir,
            notes: RefCell::new(list),
            current: Cell::new(current),
            dirty: Cell::new(false),
            save_failed: Cell::new(false),
            busy: Cell::new(false),
            theme: Cell::new(Theme::current()),
            dpi: Cell::new(dpi),
        }));
        APP.with(|a| a.set(app).ok());

        app.theme.get().apply_to_frame(hwnd);
        app.editor.apply_theme(&app.theme.get());
        app.layout();
        app.show(current);
        let _ = ShowWindow(hwnd, SW_SHOWDEFAULT);
        let _ = SetFocus(Some(app.editor.hwnd));
        Ok(app)
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(result) = app().and_then(|app| app.message(msg, wparam, lparam)) {
        return result;
    }
    // SAFETY: forwarding the message as received.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl App {
    fn message(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_SIZE => self.layout(),
            WM_SETFOCUS => {
                // SAFETY: the editor is a live child window.
                let _ = unsafe { SetFocus(Some(self.editor.hwnd)) };
            }
            // The editor covers the whole client area.
            WM_ERASEBKGND => return Some(LRESULT(1)),
            WM_COMMAND
                if lparam.0 == self.editor.hwnd.0 as isize
                    && (wparam.0 >> 16) as u32 == EN_CHANGE =>
            {
                self.changed();
            }
            WM_TIMER => self.timer(wparam.0),
            WM_DPICHANGED => {
                self.dpi.set((wparam.0 & 0xffff) as u32);
                // SAFETY: for WM_DPICHANGED, lparam points to the suggested window RECT.
                unsafe {
                    let rc = *(lparam.0 as *const RECT);
                    let flags = SWP_NOZORDER | SWP_NOACTIVATE;
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        rc.left,
                        rc.top,
                        rc.right - rc.left,
                        rc.bottom - rc.top,
                        flags,
                    );
                }
                self.layout();
                self.highlight();
            }
            WM_SETTINGCHANGE if lparam.0 != 0 => {
                // SAFETY: for WM_SETTINGCHANGE, a non-zero lparam is a NUL terminated string.
                let area =
                    unsafe { PCWSTR(lparam.0 as *const u16).to_string() }.unwrap_or_default();
                if area == "ImmersiveColorSet" {
                    self.retheme();
                }
                return None;
            }
            WM_CLOSE => self.close(),
            WM_QUERYENDSESSION => return Some(LRESULT(self.flush() as isize)),
            WM_ENDSESSION => {
                if wparam.0 != 0 && self.flush() {
                    self.drop_if_blank();
                }
            }
            WM_DESTROY => {
                // SAFETY: plain call.
                unsafe { PostQuitMessage(0) };
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }

    fn layout(&self) {
        let mut rc = RECT::default();
        // SAFETY: `rc` is a live local; both windows are alive.
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
            let _ = MoveWindow(self.editor.hwnd, 0, 0, rc.right, rc.bottom, true);
        }
        self.editor.set_padding(self.dpi.get());
    }

    fn retheme(&self) {
        let theme = Theme::current();
        self.theme.set(theme);
        theme.apply_to_frame(self.hwnd);
        self.editor.apply_theme(&theme);
        // SAFETY: plain call; makes the title bar repaint in the new colours.
        unsafe {
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
            let _ = SetWindowPos(self.hwnd, None, 0, 0, 0, 0, flags);
        }
        self.highlight();
    }

    fn highlight(&self) {
        self.busy.set(true);
        self.editor.highlight(&self.theme.get(), self.dpi.get());
        self.busy.set(false);
    }

    /// The user changed the text.
    fn changed(&self) {
        if self.busy.get() {
            return;
        }
        self.dirty.set(true);
        // SAFETY: timers on our own window; restarting one resets its delay.
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_SAVE, SAVE_DELAY_MS, None);
            if self.editor.len() > LARGE_NOTE {
                SetTimer(Some(self.hwnd), TIMER_HIGHLIGHT, HIGHLIGHT_DELAY_MS, None);
                return;
            }
        }
        self.highlight();
    }

    fn timer(&self, id: usize) {
        // SAFETY: plain call.
        let _ = unsafe { KillTimer(Some(self.hwnd), id) };
        match id {
            TIMER_SAVE => {
                self.flush();
            }
            TIMER_HIGHLIGHT => self.highlight(),
            _ => {}
        }
    }

    /// Saves pending changes now. On failure the text stays in the editor,
    /// the title is marked and the save is retried; returns false.
    fn flush(&self) -> bool {
        if !self.dirty.get() {
            return true;
        }
        // SAFETY: plain call.
        let _ = unsafe { KillTimer(Some(self.hwnd), TIMER_SAVE) };
        let ok = self.save().is_ok();
        self.save_failed.set(!ok);
        if ok {
            self.dirty.set(false);
        } else {
            // SAFETY: plain call.
            unsafe { SetTimer(Some(self.hwnd), TIMER_SAVE, SAVE_RETRY_MS, None) };
        }
        self.update_title();
        ok
    }

    fn save(&self) -> std::io::Result<()> {
        let text = self.editor.text();
        let path = self.notes.borrow().get(self.current.get()).cloned();
        match path {
            Some(path) => notes::write_atomic(&path, &text),
            // The empty note gets a file once it has text.
            None if notes::is_blank(&text) => Ok(()),
            None => {
                let path = notes::new_path(&self.dir);
                notes::write_atomic(&path, &text)?;
                self.notes.borrow_mut().push(path);
                Ok(())
            }
        }
    }

    /// Deletes the current note's file if its text is blank, as when leaving
    /// it. Returns whether it was removed from the list.
    fn drop_if_blank(&self) -> bool {
        let current = self.current.get();
        let Some(path) = self.notes.borrow().get(current).cloned() else {
            return false;
        };
        if !notes::is_blank(&self.editor.text()) {
            return false;
        }
        let _ = fs::remove_file(&path);
        self.notes.borrow_mut().remove(current);
        true
    }

    fn go(&self, nav: Nav) {
        if !self.flush() {
            return;
        }
        let current = self.current.get();
        let removed = self.drop_if_blank();
        let len = self.notes.borrow().len();
        let target = match nav {
            Nav::Older => current.saturating_sub(1),
            Nav::Newer if removed => current,
            Nav::Newer => current + 1,
            Nav::Last => len,
        }
        .min(len);
        if target != current || removed {
            self.show(target);
        }
    }

    /// Loads note `index` (or the empty note) into the editor. Notes that
    /// cannot be read are dropped from the list.
    fn show(&self, index: usize) {
        let text = loop {
            let path = self.notes.borrow().get(index).cloned();
            match path.map(|p| notes::read(&p)) {
                None => break String::new(),
                Some(Ok(text)) => break text,
                Some(Err(_)) => {
                    self.notes.borrow_mut().remove(index);
                }
            }
        };
        self.busy.set(true);
        self.editor.set_text(&text);
        self.editor.highlight(&self.theme.get(), self.dpi.get());
        self.busy.set(false);
        self.current.set(index);
        self.dirty.set(false);
        self.update_title();
    }

    fn update_title(&self) {
        let position = self.current.get() + 1;
        let total = self.notes.borrow().len() + 1;
        let mark = if self.save_failed.get() { " *" } else { "" };
        let title = HSTRING::from(format!("Whisp  {position}/{total}{mark}"));
        // SAFETY: `title` outlives the call.
        let _ = unsafe { SetWindowTextW(self.hwnd, &title) };
    }

    fn close(&self) {
        // SAFETY: plain calls on our own window.
        unsafe {
            if self.flush() {
                self.drop_if_blank();
            } else {
                let answer = MessageBoxW(
                    Some(self.hwnd),
                    w!(
                        "Whisp could not save this note. Close anyway and lose the unsaved changes?"
                    ),
                    w!("Whisp"),
                    MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                );
                if answer != IDYES {
                    return;
                }
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }

    /// Handles a key before RichEdit sees it; returns true if it was used.
    fn shortcut(&self, msg: &MSG) -> bool {
        if msg.message != WM_KEYDOWN {
            return false;
        }
        // SAFETY: plain calls.
        let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
        // Ctrl+Alt is AltGr on many layouts, which types characters.
        if down(VK_MENU) {
            return false;
        }
        let key = VIRTUAL_KEY(msg.wParam.0 as u16);
        let shift = down(VK_SHIFT);
        if !down(VK_CONTROL) {
            if shift && key == VK_INSERT {
                self.editor.paste_plain();
                return true;
            }
            return false;
        }
        match key {
            // RichEdit uses these two for font size.
            VK_OEM_6 => self.go(Nav::Newer),
            VK_OEM_4 => self.go(Nav::Older),
            VK_N => self.go(Nav::Last),
            VK_V => self.editor.paste_plain(),
            VK_A if !shift => return false,
            VK_C | VK_X | VK_Z | VK_Y => return false,
            // Any other Ctrl + letter, digit or punctuation key is a RichEdit
            // formatting shortcut (bold, alignment, line spacing, font size...).
            _ => return is_character_key(key),
        }
        true
    }
}

fn is_character_key(key: VIRTUAL_KEY) -> bool {
    matches!(key.0, 0x30..=0x39 | 0x41..=0x5a | 0xba..=0xc0 | 0xdb..=0xdf | 0xe2)
}
