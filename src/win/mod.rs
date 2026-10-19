//! The main window: note navigation, saving, theme, DPI and shortcuts.

mod actions;
mod canvas;
mod editor;
mod ocr;
mod search_ui;
mod suggest;
mod swipe;
mod theme;

use std::cell::{Cell, OnceCell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, FillRect, GetMonitorInfoW, HDC, MONITOR_DEFAULTTONULL,
    MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromRect,
};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

use crate::carousel;
use crate::gesture::Side;
use crate::lists;
use crate::metadata::Metadata;
use crate::notes;
use crate::paper::Paper;
use crate::search::{Index, Row};
use crate::settings::{self, Placement, Settings, Startup};
use crate::trash;
use editor::Editor;
use ocr::WM_OCR_DONE;
use search_ui::{Panel, TIMER_SEARCH, WM_SEARCH_OPEN};
use suggest::{Popup, WM_SUGGEST};
use swipe::{Pacer, SwipeState, WM_SWIPE_FRAME};
use theme::Theme;

const CLASS: PCWSTR = w!("WhispWinWindow");
const TIMER_SAVE: usize = 1;
const SAVE_DELAY_MS: u32 = 500;
const SAVE_RETRY_MS: u32 = 2000;
/// Clears a passing status message from the title.
const TIMER_STATUS: usize = 4;
const STATUS_MS: u32 = 3000;
const PAPER_STATUS_MS: u32 = 2000;
/// Saves the window's size and position once it stops moving.
const TIMER_PLACEMENT: usize = 6;
const PLACEMENT_DELAY_MS: u32 = 1000;
/// Posted after a change, so restyling runs once RichEdit has finished
/// handling the keystroke.
const WM_RESTYLE: u32 = WM_APP + 1;

#[derive(Clone, Copy)]
enum Nav {
    /// The oldest note.
    First,
    Older,
    Newer,
    /// The empty note after the newest.
    Last,
}

struct App {
    hwnd: HWND,
    instance: HINSTANCE,
    /// The current editor and the peek editor used while swiping, which
    /// swap roles when a swipe moves to the neighbour. The peek editor is
    /// created on first use.
    editors: [OnceCell<Editor>; 2],
    /// Index into `editors` of the current editor.
    active: Cell<usize>,
    swipe: RefCell<SwipeState>,
    /// The frame pacer, while a swipe animation runs.
    pacer: RefCell<Option<Arc<Pacer>>>,
    /// The note in the peek editor: its side and index.
    peek: Cell<Option<(Side, usize)>>,
    /// Start of the clock for gesture times.
    epoch: Instant,
    dir: PathBuf,
    /// Saved notes: pinned ones first, then the rest, each oldest first.
    /// Index `notes.len()` is the empty note at the end, which gets a file
    /// once it has text.
    notes: RefCell<Vec<PathBuf>>,
    current: Cell<usize>,
    dirty: Cell<bool>,
    save_failed: Cell<bool>,
    /// Set while the program itself changes the text, so EN_CHANGE is not
    /// taken for an edit.
    busy: Cell<bool>,
    /// A WM_RESTYLE is posted and not handled yet.
    restyle_posted: Cell<bool>,
    /// Ctrl+Z or Ctrl+Y was pressed: the next restyle does not turn
    /// `- [ ]` into `☐`, or undoing that conversion would redo it at once.
    undoing: Cell<bool>,
    theme: Cell<Theme>,
    dpi: Cell<u32>,
    /// Changes whenever a different note is shown, so text read from an
    /// image is only inserted into the note it was pasted into.
    generation: Cell<u64>,
    /// An image is being read.
    ocr_running: Cell<bool>,
    /// A passing message shown in the title, cleared by TIMER_STATUS.
    status: RefCell<Option<&'static str>>,
    /// The search panel (Ctrl+F), created on first use.
    search: OnceCell<Panel>,
    search_open: Cell<bool>,
    /// The rows the search list shows.
    search_rows: RefCell<Vec<Row>>,
    /// All notes for searching, cached by modified time.
    index: RefCell<Index>,
    /// The `::` suggestion popup, created on first use.
    suggest: OnceCell<Popup>,
    suggest_open: Cell<bool>,
    /// The commands the popup lists, with their descriptions.
    suggest_rows: RefCell<Vec<(&'static str, &'static str)>>,
    /// A WM_SUGGEST is posted and not handled yet.
    suggest_posted: Cell<bool>,
    paper: Cell<Paper>,
    /// Where `settings` lives; None without LOCALAPPDATA.
    settings_dir: Option<PathBuf>,
    settings: RefCell<Settings>,
    /// Whisp's `metadata.json` in the notes folder, as last read or written.
    /// Empty if it could not be read; writes then refuse to overwrite it.
    meta: RefCell<Metadata>,
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
            if app.intercept(&mut msg) {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn create(dir: PathBuf, list: Vec<(PathBuf, SystemTime)>) -> windows::core::Result<&'static App> {
    // SAFETY: window creation on this thread; the class name is static.
    unsafe {
        LoadLibraryW(w!("Msftedit.dll"))?;
        let instance: HINSTANCE = GetModuleHandleW(None)?.into();
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
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: icon(SM_CXICON, SM_CYICON),
            hIconSm: icon(SM_CXSMICON, SM_CYSMICON),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let settings_dir = settings::dir();
        let settings = settings_dir
            .as_deref()
            .map_or_else(Settings::default, Settings::load);
        let sys_dpi = GetDpiForSystem();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("Whisp"),
            // Clipping the editors keeps background painting off them while they move.
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(420, sys_dpi),
            scale(600, sys_dpi),
            None,
            None,
            Some(instance),
            None,
        )?;
        // The saved size and position, before the window is shown. Done
        // before the app exists, so a DPI change on the way only updates
        // the DPI and keeps the saved size.
        let saved = settings.get("window").and_then(Placement::parse);
        let maximized = match saved {
            Some(p) if is_on_screen(&p) => {
                let placement = WINDOWPLACEMENT {
                    length: size_of::<WINDOWPLACEMENT>() as u32,
                    showCmd: SW_HIDE.0 as u32,
                    rcNormalPosition: RECT {
                        left: p.left,
                        top: p.top,
                        right: p.right,
                        bottom: p.bottom,
                    },
                    ..Default::default()
                };
                let _ = SetWindowPlacement(hwnd, &placement);
                p.maximized
            }
            Some(_) => {
                // Saved on a monitor that is gone: centre on the primary one.
                centre_on_primary(hwnd, sys_dpi);
                false
            }
            None => {
                let dpi = GetDpiForWindow(hwnd);
                if dpi != sys_dpi {
                    let flags = SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE;
                    let _ = SetWindowPos(hwnd, None, 0, 0, scale(420, dpi), scale(600, dpi), flags);
                }
                false
            }
        };
        let dpi = GetDpiForWindow(hwnd);
        let editor = Editor::create(hwnd, instance, true)?;

        let paper = Paper::parse(settings.get("paper").unwrap_or(""));
        let meta = Metadata::load(&dir).unwrap_or_default();
        let list = carousel::arrange(
            list,
            SystemTime::now(),
            settings.archive_days(),
            settings.max_carousel_size() as usize,
            |p| is_pinned(&meta, p),
        );
        let current = start_index(&list, &settings);
        let app: &'static App = Box::leak(Box::new(App {
            hwnd,
            instance,
            editors: [OnceCell::from(editor), OnceCell::new()],
            active: Cell::new(0),
            swipe: RefCell::new(SwipeState::Idle),
            peek: Cell::new(None),
            epoch: Instant::now(),
            dir,
            notes: RefCell::new(list),
            current: Cell::new(current),
            dirty: Cell::new(false),
            save_failed: Cell::new(false),
            busy: Cell::new(false),
            restyle_posted: Cell::new(false),
            undoing: Cell::new(false),
            pacer: RefCell::new(None),
            theme: Cell::new(Theme::current()),
            dpi: Cell::new(dpi),
            generation: Cell::new(0),
            ocr_running: Cell::new(false),
            status: RefCell::new(None),
            search: OnceCell::new(),
            search_open: Cell::new(false),
            search_rows: RefCell::new(Vec::new()),
            index: RefCell::new(Index::default()),
            suggest: OnceCell::new(),
            suggest_open: Cell::new(false),
            suggest_rows: RefCell::new(Vec::new()),
            suggest_posted: Cell::new(false),
            paper: Cell::new(paper),
            settings_dir,
            settings: RefCell::new(settings),
            meta: RefCell::new(meta),
        }));
        APP.with(|a| a.set(app).ok());
        actions::install_cursor_hook();

        app.theme.get().apply_to_frame(hwnd);
        app.editor().apply_theme(&app.theme.get());
        canvas::set(paper, &app.theme.get());
        app.layout();
        app.show(current);
        let _ = ShowWindow(
            hwnd,
            if maximized {
                SW_SHOWMAXIMIZED
            } else {
                SW_SHOWDEFAULT
            },
        );
        let _ = SetFocus(Some(app.editor().hwnd));
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
            WM_SIZE => {
                self.cancel_swipe();
                self.layout();
                self.placement_changed();
            }
            WM_MOVE => self.placement_changed(),
            WM_ACTIVATE => {
                if (wparam.0 & 0xffff) as u32 == WA_INACTIVE {
                    self.finish_swipe();
                    self.hide_suggestions();
                }
                return None;
            }
            WM_SETFOCUS => {
                let target = match self.search_panel() {
                    Some(panel) if self.search_open.get() => panel.edit,
                    _ => self.editor().hwnd,
                };
                // SAFETY: a live child window.
                let _ = unsafe { SetFocus(Some(target)) };
            }
            // Only what the editors leave uncovered, as at the ends of a swipe.
            WM_ERASEBKGND => {
                let mut rc = RECT::default();
                // SAFETY: wparam is the DC to erase; the brush is deleted after use.
                unsafe {
                    let _ = GetClientRect(self.hwnd, &mut rc);
                    let brush = CreateSolidBrush(self.theme.get().background);
                    FillRect(HDC(wparam.0 as *mut _), &rc, brush);
                    let _ = DeleteObject(brush.into());
                }
                self.paint_search_box(HDC(wparam.0 as *mut _));
                return Some(LRESULT(1));
            }
            WM_COMMAND
                if lparam.0 == self.editor().hwnd.0 as isize
                    && (wparam.0 >> 16) as u32 == EN_CHANGE =>
            {
                self.changed();
            }
            WM_COMMAND
                if self
                    .search_panel()
                    .is_some_and(|p| lparam.0 == p.edit.0 as isize) =>
            {
                if (wparam.0 >> 16) as u32 == EN_CHANGE {
                    self.search_changed();
                }
            }
            WM_COMMAND
                if self
                    .search_panel()
                    .is_some_and(|p| lparam.0 == p.list.0 as isize) =>
            {
                if (wparam.0 >> 16) as u32 == LBN_SELCHANGE {
                    self.search_clicked();
                }
            }
            WM_DRAWITEM => {
                // SAFETY: for WM_DRAWITEM, lparam points to a DRAWITEMSTRUCT.
                let item =
                    unsafe { &*(lparam.0 as *const windows::Win32::UI::Controls::DRAWITEMSTRUCT) };
                if self.draw_suggestion(item) {
                    return Some(LRESULT(1));
                }
                if !self.search_panel().is_some_and(|p| p.list == item.hwndItem) {
                    return None;
                }
                self.draw_search_row(item);
                return Some(LRESULT(1));
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                let (hdc, control) = (HDC(wparam.0 as *mut _), HWND(lparam.0 as *mut _));
                return self
                    .suggestion_color(hdc, control)
                    .or_else(|| self.search_color(hdc, control));
            }
            WM_NOTIFY => {
                // SAFETY: for WM_NOTIFY, lparam points to an NMHDR.
                let header = unsafe { &*(lparam.0 as *const windows::Win32::UI::Controls::NMHDR) };
                if header.hwndFrom == self.editor().hwnd
                    && header.code == windows::Win32::UI::Controls::RichEdit::EN_SELCHANGE
                {
                    self.selection_changed();
                }
                return None;
            }
            WM_SUGGEST => self.update_suggestions(),
            WM_SEARCH_OPEN if self.search_open.get() => self.open_row(wparam.0),
            WM_TIMER => self.timer(wparam.0),
            WM_SWIPE_FRAME => self.swipe_frame_message(),
            WM_OCR_DONE => {
                // SAFETY: lparam is the result the OCR worker posted.
                let done = unsafe { ocr::take(lparam) };
                self.ocr_done(*done);
            }
            WM_RESTYLE => {
                self.restyle_posted.set(false);
                let convert = !self.undoing.replace(false);
                self.busy.set(true);
                let converted = self.editor().restyle_changes(&self.theme.get(), convert);
                self.busy.set(false);
                if converted {
                    // A text change: saved and restyled like typing.
                    self.changed();
                }
            }
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
                self.style_search();
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
            // Alt pressed and released alone would enter the (missing) menu
            // bar, and the next key would go there; Alt+Space still opens the
            // window menu.
            WM_SYSCOMMAND if (wparam.0 & 0xfff0) as u32 == SC_KEYMENU && lparam.0 == 0 => {}
            WM_QUERYENDSESSION => return Some(LRESULT(self.flush() as isize)),
            WM_ENDSESSION => {
                self.save_placement();
                if wparam.0 != 0 && self.flush() {
                    self.drop_if_blank();
                    self.save_last_note();
                }
            }
            WM_DESTROY => {
                self.stop_frames();
                // SAFETY: plain call.
                unsafe { PostQuitMessage(0) };
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }

    /// Removes the search highlight from the editors, if any.
    pub(super) fn clear_highlight(&self) {
        for editor in self.editors.iter().filter_map(OnceCell::get) {
            canvas::set_highlight(editor.hwnd, None);
        }
    }

    /// The current editor.
    fn editor(&self) -> &Editor {
        self.editors[self.active.get()]
            .get()
            .expect("the current editor exists")
    }

    /// Both editors at rest: the current one filling the window, the peek
    /// editor hidden behind it with the same size and padding.
    fn layout(&self) {
        self.hide_suggestions();
        let mut rc = RECT::default();
        // SAFETY: `rc` is a live local; the windows are alive.
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
            for (i, editor) in self.editors.iter().enumerate() {
                let Some(editor) = editor.get() else { continue };
                let _ = MoveWindow(editor.hwnd, 0, 0, rc.right, rc.bottom, true);
                if i != self.active.get() {
                    let _ = ShowWindow(editor.hwnd, SW_HIDE);
                }
                editor.set_padding(self.dpi.get());
            }
        }
        self.layout_search();
    }

    fn retheme(&self) {
        self.hide_suggestions();
        self.finish_swipe();
        let theme = Theme::current();
        self.theme.set(theme);
        theme.apply_to_frame(self.hwnd);
        for editor in self.editors.iter().filter_map(OnceCell::get) {
            editor.apply_theme(&theme);
        }
        // SAFETY: plain call; makes the title bar repaint in the new colours.
        unsafe {
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
            let _ = SetWindowPos(self.hwnd, None, 0, 0, 0, 0, flags);
        }
        canvas::set(self.paper.get(), &theme);
        self.restyle();
        self.style_search();
    }

    /// Restyles after a theme change.
    fn restyle(&self) {
        self.busy.set(true);
        self.editor().restyle(&self.theme.get());
        self.busy.set(false);
    }

    /// The user changed the text. Only records it: RichEdit is still in the
    /// middle of handling the keystroke, so restyling waits for WM_RESTYLE.
    fn changed(&self) {
        if self.busy.get() {
            return;
        }
        self.dirty.set(true);
        self.clear_highlight();
        // SAFETY: a timer and a posted message to our own window; restarting
        // the timer resets its delay.
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_SAVE, SAVE_DELAY_MS, None);
            if !self.restyle_posted.replace(true)
                && PostMessageW(Some(self.hwnd), WM_RESTYLE, WPARAM(0), LPARAM(0)).is_err()
            {
                self.restyle_posted.set(false);
            }
        }
    }

    fn timer(&self, id: usize) {
        if id == TIMER_SAVE {
            // SAFETY: plain call.
            let _ = unsafe { KillTimer(Some(self.hwnd), id) };
            self.flush();
        } else if id == TIMER_SEARCH {
            self.run_search();
        } else if id == TIMER_PLACEMENT {
            // SAFETY: plain call.
            let _ = unsafe { KillTimer(Some(self.hwnd), id) };
            self.save_placement();
        } else if id == TIMER_STATUS {
            // SAFETY: plain call.
            let _ = unsafe { KillTimer(Some(self.hwnd), id) };
            self.status.replace(None);
            self.update_title();
        } else {
            self.swipe_timer(id);
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
        let text = self.editor().text();
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
        if !notes::is_blank(&self.editor().text()) {
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
            Nav::First => 0,
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
        self.hide_suggestions();
        self.clear_highlight();
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
        self.editor().load(&text, &self.theme.get());
        self.busy.set(false);
        self.generation.set(self.generation.get() + 1);
        self.current.set(index);
        self.dirty.set(false);
        self.update_title();
    }

    fn update_title(&self) {
        let position = self.current.get() + 1;
        let total = self.notes.borrow().len() + 1;
        let pinned = self
            .notes
            .borrow()
            .get(self.current.get())
            .is_some_and(|p| self.is_pinned(p));
        let pin = if pinned { " \u{b7} Pinned" } else { "" };
        let mark = if self.save_failed.get() { " *" } else { "" };
        let status = match *self.status.borrow() {
            Some(message) => format!(" \u{b7} {message}"),
            None if self.ocr_running.get() => " \u{b7} Reading image\u{2026}".to_string(),
            None if self.search_open.get() => " \u{b7} Search".to_string(),
            None => String::new(),
        };
        let title = HSTRING::from(format!("Whisp  {position}/{total}{pin}{mark}{status}"));
        // SAFETY: `title` outlives the call.
        let _ = unsafe { SetWindowTextW(self.hwnd, &title) };
    }

    fn close(&self) {
        self.finish_swipe();
        self.save_placement();
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
            self.save_last_note();
            let _ = DestroyWindow(self.hwnd);
        }
    }

    /// Remembers which note is shown, or the empty page, for the next start.
    /// Called after a blank note has been dropped, so it is not remembered.
    fn save_last_note(&self) {
        let name = self
            .notes
            .borrow()
            .get(self.current.get())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.save_setting("last_note", &name);
    }

    /// Whether the note at `path` is pinned in Whisp's metadata.
    fn is_pinned(&self, path: &Path) -> bool {
        is_pinned(&self.meta.borrow(), path)
    }

    /// Where a note goes when it is unpinned or restored, by Whisp's walk
    /// over the current list (which must not hold the note).
    fn insert_index(&self, path: &Path, pinned: bool) -> usize {
        let others: Vec<_> = self
            .notes
            .borrow()
            .iter()
            .map(|p| (self.is_pinned(p), modified(p)))
            .collect();
        carousel::insert_position(&others, pinned, modified(path))
    }

    /// Handles a key before RichEdit sees it; returns true if it was used.
    fn shortcut(&self, msg: &MSG) -> bool {
        // SAFETY: plain calls.
        let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
        // Alt+P, without Ctrl so AltGr typing is left alone: the next paper.
        if msg.message == WM_SYSKEYDOWN
            && VIRTUAL_KEY(msg.wParam.0 as u16) == VK_P
            && !down(VK_CONTROL)
        {
            self.next_paper();
            return true;
        }
        // Alt+Up and Alt+Down, again without Ctrl: move the line. Alt+F and
        // Alt+L: the first note and the last page (the empty note).
        if msg.message == WM_SYSKEYDOWN && !down(VK_CONTROL) {
            let up = match VIRTUAL_KEY(msg.wParam.0 as u16) {
                VK_UP => true,
                VK_DOWN => false,
                VK_F => {
                    self.go_first();
                    return true;
                }
                VK_L => {
                    self.go(Nav::Last);
                    return true;
                }
                _ => return false,
            };
            self.editor()
                .list_action(|lines, caret| lists::move_line(lines, caret, up), false);
            return true;
        }
        if msg.message != WM_KEYDOWN {
            return false;
        }
        // Ctrl+Alt is AltGr on many layouts, which types characters.
        if down(VK_MENU) {
            return false;
        }
        let key = VIRTUAL_KEY(msg.wParam.0 as u16);
        let shift = down(VK_SHIFT);
        if !down(VK_CONTROL) {
            if let Some(used) = self.suggestion_key(key, shift) {
                return used;
            }
            match key {
                // Esc first clears a search highlight; with none it goes on.
                VK_ESCAPE if canvas::has_highlight(self.editor().hwnd) => self.clear_highlight(),
                VK_INSERT if shift => self.paste(),
                // Backspace on a lone box in a list note removes the line.
                VK_BACK if !shift => return self.backspace_key(),
                // Shift+Delete deletes the note, as in Whisp.
                VK_DELETE if shift => self.delete_note(),
                // A complete `::command` before the caret expands, with the
                // space or line break after it, as in Whisp.
                VK_SPACE if !shift => return self.expand_at_caret(" "),
                // Then Enter continues lists; elsewhere it stays RichEdit's.
                VK_RETURN if !shift => {
                    return self.expand_at_caret("\n")
                        || self.editor().list_action(lists::enter, true);
                }
                // Tab always indents the line, as in Whisp, never typing a tab.
                VK_TAB if shift => {
                    self.editor().list_action(lists::shift_tab, false);
                }
                VK_TAB => {
                    self.editor().list_action(lists::tab, false);
                }
                _ => return false,
            }
            return true;
        }
        match key {
            // RichEdit uses these two for font size.
            VK_OEM_6 => self.go(Nav::Newer),
            VK_OEM_4 => self.go(Nav::Older),
            VK_N => self.go(Nav::Last),
            VK_D => self.delete_note(),
            VK_T if shift => self.restore_note(),
            VK_F => self.open_search(),
            VK_V if shift => self.paste_stripped(),
            VK_V => self.paste(),
            // Note commands win over Whisp's Ctrl+Shift+C/S editing keys.
            VK_C if shift => self.copy_note(),
            VK_S if shift => self.export_note(),
            VK_S => {
                self.editor().list_action(lists::toggle_checkbox, false);
            }
            VK_B if shift => self.bump_note(),
            VK_P if shift => self.toggle_pin(),
            VK_B => self.wrap_key("**"),
            VK_I => self.wrap_key("*"),
            VK_U => self.wrap_key("_"),
            VK_X if shift => self.wrap_key("~~"),
            VK_L => self.shorten_link(),
            VK_Q => self.close(),
            VK_BACK if !shift => return self.ctrl_backspace_key(),
            // Ctrl+Tab outdents, as in Whisp.
            VK_TAB => {
                self.editor().list_action(lists::shift_tab, false);
            }
            VK_A if !shift => return false,
            VK_Z | VK_Y => {
                self.undoing.set(true);
                return false;
            }
            VK_C | VK_X => return false,
            // Any other Ctrl + letter, digit or punctuation key is a RichEdit
            // formatting shortcut (bold, alignment, line spacing, font size...).
            _ => return is_character_key(key),
        }
        true
    }
}

impl App {
    /// Ctrl+V, Shift+Insert and Ctrl+Shift+V: text as plain text, else the
    /// text read from an image.
    fn paste(&self) {
        // Text comes first. Office and other apps put both text and a
        // picture of it on the clipboard, and the text is what the user
        // means. Whisp checks for an image first; this differs on purpose.
        if ocr::clipboard_has_text() {
            self.paste_text();
            return;
        }
        if !ocr::clipboard_has_image() {
            return;
        }
        if self.ocr_running.get() {
            self.show_status("Still reading the previous image");
            return;
        }
        let Some(dib) = ocr::clipboard_dib(self.hwnd) else {
            return;
        };
        self.ocr_running.set(true);
        self.update_title();
        ocr::start(self.hwnd, dib, self.generation.get());
    }

    /// The text read from an image: inserted in place of the selection if
    /// the note it was pasted into is still the current one.
    fn ocr_done(&self, done: ocr::Done) {
        self.ocr_running.set(false);
        let message = match done.outcome {
            _ if done.generation != self.generation.get() => {
                Some("Image text discarded: the note changed")
            }
            ocr::Outcome::Text(text) => {
                self.editor().insert_plain(&text);
                None
            }
            ocr::Outcome::NoText => Some("No text found in image"),
            ocr::Outcome::NoEngine => Some("No OCR language installed"),
            ocr::Outcome::Failed => Some("Could not read the image"),
        };
        match message {
            Some(message) => self.show_status(message),
            None => self.update_title(),
        }
    }

    /// Shows `message` in the title for a few seconds.
    fn show_status(&self, message: &'static str) {
        self.show_status_for(message, STATUS_MS);
    }

    fn show_status_for(&self, message: &'static str, ms: u32) {
        self.status.replace(Some(message));
        self.update_title();
        // SAFETY: a timer on our own window; restarting it resets its delay.
        unsafe { SetTimer(Some(self.hwnd), TIMER_STATUS, ms, None) };
    }

    /// Alt+P: blank, dotted, grid, large grid, and round again.
    fn next_paper(&self) {
        let paper = self.paper.get().next();
        self.paper.set(paper);
        canvas::set(paper, &self.theme.get());
        self.save_setting("paper", paper.name());
        // SAFETY: the string outlives the call.
        unsafe { OutputDebugStringW(&HSTRING::from(format!("whisp: paper {}\n", paper.name()))) };
        let message = match paper {
            Paper::Blank => "Paper: blank",
            Paper::Dotted => "Paper: dotted",
            Paper::Grid => "Paper: grid",
            Paper::LargeGrid => "Paper: large grid",
        };
        self.show_status_for(message, PAPER_STATUS_MS);
    }
}

impl App {
    /// Sets `key` in settings.ini and writes the file, if it changed.
    fn save_setting(&self, key: &str, value: &str) {
        let Some(dir) = &self.settings_dir else {
            return;
        };
        let mut settings = self.settings.borrow_mut();
        if settings.get(key) == Some(value) {
            return;
        }
        settings.set(key, value);
        let _ = settings.save(dir);
    }

    /// The window moved or changed size: save that once it settles.
    fn placement_changed(&self) {
        // SAFETY: a timer on our own window; restarting it resets its delay.
        unsafe { SetTimer(Some(self.hwnd), TIMER_PLACEMENT, PLACEMENT_DELAY_MS, None) };
    }

    /// Saves the normal (restored) rectangle and whether the window is
    /// maximized, also when it is minimized from the maximized state.
    fn save_placement(&self) {
        let mut placement = WINDOWPLACEMENT {
            length: size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        // SAFETY: `placement` is a properly sized local.
        if unsafe { GetWindowPlacement(self.hwnd, &mut placement) }.is_err() {
            return;
        }
        let shown = placement.showCmd;
        let maximized = shown == SW_SHOWMAXIMIZED.0 as u32
            || (shown == SW_SHOWMINIMIZED.0 as u32
                && placement.flags.0 & WPF_RESTORETOMAXIMIZED.0 != 0);
        let rc = placement.rcNormalPosition;
        let value = Placement {
            left: rc.left,
            top: rc.top,
            right: rc.right,
            bottom: rc.bottom,
            maximized,
        }
        .format();
        self.save_setting("window", &value);
    }

    /// The notes deleted with Ctrl+D, oldest first, by their names in the
    /// trash.
    fn deleted(&self) -> Vec<String> {
        trash::parse_deleted(self.settings.borrow().get("deleted").unwrap_or(""))
    }

    /// Ctrl+D or Shift+Delete: moves the current note into Whisp's trash
    /// folder, no dialog, as Ctrl+Shift+T brings it back. The empty note at
    /// the end and blank notes are left alone.
    fn delete_note(&self) {
        let current = self.current.get();
        let Some(path) = self.notes.borrow().get(current).cloned() else {
            return;
        };
        if notes::is_blank(&self.editor().text()) || !self.flush() {
            return;
        }
        match trash::delete(&self.dir, &path) {
            Ok(name) => {
                let mut deleted = self.deleted();
                deleted.push(name);
                self.save_setting("deleted", &trash::format_deleted(&deleted));
                self.notes.borrow_mut().remove(current);
                // The next note takes its place, as when Whisp's carousel
                // removes a page: the newer one, or the empty note.
                self.show(current);
                self.show_status("Note deleted \u{b7} Ctrl+Shift+T to restore");
            }
            Err(_) => self.show_status("Could not delete the note"),
        }
    }

    /// Ctrl+Shift+T: brings back the most recently deleted note that is
    /// still in the trash, also from an earlier session, at its place by
    /// modified time, and shows it.
    fn restore_note(&self) {
        if !self.flush() {
            return;
        }
        let mut deleted = self.deleted();
        while let Some(name) = deleted.pop() {
            if !trash::is_in_trash(&self.dir, &name) {
                // Emptied or restored by Whisp meanwhile.
                continue;
            }
            let Ok(path) = trash::restore(&self.dir, &name) else {
                self.show_status("Could not restore the note");
                return;
            };
            self.save_setting("deleted", &trash::format_deleted(&deleted));
            // Leaving the current note, as navigation does.
            self.drop_if_blank();
            // It keeps its file name, so a pin from before survives.
            let position = self.insert_index(&path, self.is_pinned(&path));
            self.notes.borrow_mut().insert(position, path);
            self.show(position);
            self.show_status("Note restored");
            return;
        }
        self.save_setting("deleted", "");
        self.show_status("No deleted note to restore");
    }
}

/// The note's modified time, or the epoch if it cannot be read.
fn modified(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

/// Whether `meta` marks the note at `path` as pinned, by file name.
fn is_pinned(meta: &Metadata, path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| meta.flag(name, "pinned"))
}

/// The index shown at startup: the empty page for `startup=empty_note`,
/// else the note shown at the last close if it is in the carousel, else the
/// newest note. As in Whisp, closing on the empty page also gives the
/// newest note.
fn start_index(list: &[PathBuf], settings: &Settings) -> usize {
    if settings.startup() == Startup::EmptyNote {
        return list.len();
    }
    let name = settings.get("last_note").unwrap_or("");
    list.iter()
        .position(|p| p.file_name().is_some_and(|n| n == name))
        .unwrap_or(list.len().saturating_sub(1))
}

/// Whether a saved window rectangle is on a monitor that exists now.
fn is_on_screen(p: &Placement) -> bool {
    let rect = RECT {
        left: p.left,
        top: p.top,
        right: p.right,
        bottom: p.bottom,
    };
    // SAFETY: plain call with a live local.
    !unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }.is_invalid()
}

/// The default size, centred on the primary monitor's work area.
fn centre_on_primary(hwnd: HWND, dpi: u32) {
    let (width, height) = (scale(420, dpi), scale(600, dpi));
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: plain calls; `info` is a properly sized local.
    unsafe {
        let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let work = info.rcWork;
        let x = work.left + (work.right - work.left - width) / 2;
        let y = work.top + (work.bottom - work.top - height) / 2;
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

fn is_character_key(key: VIRTUAL_KEY) -> bool {
    matches!(key.0, 0x30..=0x39 | 0x41..=0x5a | 0xba..=0xc0 | 0xdb..=0xdf | 0xe2)
}
