//! The Preferences window (Ctrl+,): rows of a label, an optional dim
//! subtitle and a control, under group headings. Every change applies and
//! is saved at once, as in Whisp; there is no OK button.
//!
//! The window is owned by the main window and follows its theme. Tab and
//! Shift+Tab are handled by `IsDialogMessageW` in `App::popup_message`.

use std::cell::Cell;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::{COMBOBOXINFO, GetComboBoxInfo, SetWindowTheme};
use windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_CONTROL, VK_ESCAPE};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_MENU, VK_OEM_2, VK_OEM_COMMA};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

use super::dialog;
use super::theme::Theme;
use super::{App, scale};
use crate::paper::Paper;
use crate::settings::{self, Settings};

const CLASS: PCWSTR = w!("WhispPrefsWindow");
const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0);
/// Width of the client area, margins and the control column, at 96 dpi.
const CLIENT_WIDTH: i32 = 600;
const MARGIN: i32 = 24;
const GAP: i32 = 16;
const CONTROL_WIDTH: i32 = 210;
/// Space between rows, and above a heading that is not the first.
const ROW_GAP: i32 = 14;
const HEADING_GAP: i32 = 14;
/// Height of the closed combo box (a first guess; the real one is read back)
/// and of the check box, and the height of one drop down row.
const COMBO_HEIGHT: i32 = 28;
const CHECK_SIZE: i32 = 22;
const DROP_ROW: i32 = 24;

const ID_CONFIRM_DELETE: u16 = 200;
/// Static control style: `&` is not a mnemonic.
const SS_NOPREFIX: u32 = 0x80;
const BN_CLICKED: u32 = 0;
const CBN_SELCHANGE: u32 = 1;

/// A combo box: which setting it changes and the value of each item.
struct Combo {
    id: u16,
    key: &'static str,
    hwnd: HWND,
    values: Vec<String>,
}

enum Entry {
    Heading(HWND, &'static str),
    Row {
        label: HWND,
        text: &'static str,
        sub: Option<(HWND, &'static str)>,
        control: HWND,
        /// The control is a combo box; its window height includes the list.
        drop_rows: Option<usize>,
    },
}

pub(super) struct Prefs {
    pub hwnd: HWND,
    entries: Vec<Entry>,
    combos: Vec<Combo>,
    check: HWND,
    /// Normal and bold message fonts.
    fonts: Cell<[HFONT; 2]>,
    /// Window background and field (list) background.
    brushes: Cell<[HBRUSH; 2]>,
    dpi: Cell<u32>,
    /// The control that had the focus when the window was deactivated.
    focus: Cell<HWND>,
}

/// What one row of the window shows, before any window exists.
enum Spec {
    Heading(&'static str),
    Choice {
        label: &'static str,
        sub: Option<&'static str>,
        key: &'static str,
        /// Value and text of each item, and the value now in use.
        items: Vec<(String, String)>,
        current: String,
    },
    Check {
        label: &'static str,
        sub: &'static str,
        on: bool,
    },
}

fn pair(value: &str, text: &str) -> (String, String) {
    (value.to_string(), text.to_string())
}

/// The rows, from the settings as they are now.
fn specs(settings: &Settings, paper: Paper) -> Vec<Spec> {
    let scheme = match settings.color_scheme() {
        settings::ColorScheme::System => "system",
        settings::ColorScheme::Light => "light",
        settings::ColorScheme::Dark => "dark",
    };
    let numbers = |base: &[(u32, &str)], value: u32, extra: fn(u32) -> String| {
        settings::with_stored(base, value, extra)
            .into_iter()
            .map(|(n, text)| (n.to_string(), text))
            .collect::<Vec<_>>()
    };
    vec![
        Spec::Heading("Appearance"),
        Spec::Choice {
            label: "Colour scheme",
            sub: None,
            key: "color_scheme",
            items: vec![
                pair("system", "System"),
                pair("light", "Light"),
                pair("dark", "Dark"),
            ],
            current: scheme.to_string(),
        },
        Spec::Choice {
            label: "Paper",
            sub: None,
            key: "paper",
            items: vec![
                pair(Paper::Blank.name(), "Blank"),
                pair(Paper::Dotted.name(), "Dotted"),
                pair(Paper::Grid.name(), "Grid"),
                pair(Paper::LargeGrid.name(), "Large grid"),
            ],
            current: paper.name().to_string(),
        },
        Spec::Heading("Behaviour"),
        Spec::Choice {
            label: "Hide symbols scope",
            sub: Some("Apply hidden symbols globally to all notes, or remember it per note."),
            key: "hide_symbols_scope",
            items: vec![pair("global", "Global"), pair("per_note", "Per note")],
            current: if settings.get("hide_symbols_scope") == Some("per_note") {
                "per_note"
            } else {
                "global"
            }
            .to_string(),
        },
        Spec::Choice {
            label: "Startup",
            sub: None,
            key: "startup",
            items: vec![
                pair("last_note", "Restore last active note"),
                pair("empty_note", "Start with empty note"),
            ],
            current: if settings.startup() == settings::Startup::EmptyNote {
                "empty_note"
            } else {
                "last_note"
            }
            .to_string(),
        },
        Spec::Choice {
            label: "Auto-archive inactive notes",
            sub: Some(
                "Notes are not deleted. They are simply hidden from the app to reduce \
                 clutter, but remain fully searchable via Ctrl+F.",
            ),
            key: "archive_days",
            items: numbers(&settings::ARCHIVE_CHOICES, settings.archive_days(), |n| {
                format!("{n} days")
            }),
            current: settings.archive_days().to_string(),
        },
        Spec::Choice {
            label: "Max carousel size",
            sub: Some(
                "Limit the number of notes loaded into the swipeable carousel. Older notes \
                 remain fully searchable.",
            ),
            key: "max_carousel_size",
            items: numbers(
                &settings::CAROUSEL_CHOICES,
                settings.max_carousel_size(),
                |n| format!("{n} Notes"),
            ),
            current: settings.max_carousel_size().to_string(),
        },
        Spec::Check {
            label: "Confirm before deleting",
            sub: "Ask for confirmation when deleting a note",
            on: settings.confirm_delete(),
        },
    ]
}

pub(super) fn register(instance: HINSTANCE) -> windows::core::Result<()> {
    dialog::register(instance, CLASS, Some(prefs_proc))
}

unsafe extern "system" fn prefs_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(result) = super::app().and_then(|app| app.prefs_message(hwnd, msg, wparam, lparam))
    {
        return result;
    }
    // SAFETY: forwarding the message as received.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A child window of the Preferences window.
fn child(
    parent: HWND,
    instance: HINSTANCE,
    class: PCWSTR,
    text: &str,
    style: u32,
    id: u16,
) -> windows::core::Result<HWND> {
    // SAFETY: plain window creation; `text` outlives the call. The ID goes
    // in the menu parameter, as for any child window.
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            &HSTRING::from(text),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | style),
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as usize as *mut _)),
            Some(instance),
            None,
        )
    }
}

impl Prefs {
    fn create(app: &App, owner: HWND) -> windows::core::Result<Prefs> {
        let (settings, paper) = (app.settings.borrow(), app.paper.get());
        let mut rc = RECT::default();
        // SAFETY: `rc` is a live local. Created where the owner is, so the
        // window starts on the owner's monitor and gets its DPI.
        let hwnd = unsafe {
            let _ = GetWindowRect(owner, &mut rc);
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                CLASS,
                w!("Preferences"),
                STYLE | WS_CLIPCHILDREN,
                rc.left,
                rc.top,
                scale(CLIENT_WIDTH, 96),
                scale(400, 96),
                Some(owner),
                None,
                Some(app.instance),
                None,
            )?
        };
        let mut prefs = Prefs {
            hwnd,
            entries: Vec::new(),
            combos: Vec::new(),
            check: HWND::default(),
            fonts: Cell::new([HFONT::default(); 2]),
            brushes: Cell::new([HBRUSH::default(); 2]),
            dpi: Cell::new(96),
            focus: Cell::new(HWND::default()),
        };
        let built = prefs.build(app.instance, &specs(&settings, paper));
        if let Err(e) = built {
            // SAFETY: our own window.
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(e);
        }
        Ok(prefs)
    }

    /// Creates the controls, in tab order.
    fn build(&mut self, instance: HINSTANCE, specs: &[Spec]) -> windows::core::Result<()> {
        let statics = SS_NOPREFIX;
        let tab = WS_TABSTOP.0;
        let mut id = 100;
        for spec in specs {
            match spec {
                Spec::Heading(text) => {
                    let h = child(self.hwnd, instance, w!("STATIC"), text, statics, 0)?;
                    self.entries.push(Entry::Heading(h, text));
                }
                Spec::Choice {
                    label,
                    sub,
                    key,
                    items,
                    current,
                } => {
                    id += 1;
                    let (label_hwnd, sub) = self.label(instance, label, *sub)?;
                    let style = tab | (CBS_DROPDOWNLIST as u32) | WS_VSCROLL.0;
                    let combo = child(self.hwnd, instance, w!("COMBOBOX"), "", style, id)?;
                    for (_, text) in items {
                        let text = HSTRING::from(text.as_str());
                        // SAFETY: CB_ADDSTRING copies the string during the call.
                        unsafe {
                            SendMessageW(
                                combo,
                                CB_ADDSTRING,
                                None,
                                Some(LPARAM(text.as_ptr() as isize)),
                            )
                        };
                    }
                    let selected = items.iter().position(|(v, _)| v == current).unwrap_or(0);
                    // SAFETY: no pointers.
                    unsafe {
                        SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(selected)), None);
                    }
                    self.combos.push(Combo {
                        id,
                        key,
                        hwnd: combo,
                        values: items.iter().map(|(v, _)| v.clone()).collect(),
                    });
                    self.entries.push(Entry::Row {
                        label: label_hwnd,
                        text: label,
                        sub,
                        control: combo,
                        drop_rows: Some(items.len()),
                    });
                }
                Spec::Check { label, sub, on } => {
                    let (label_hwnd, sub) = self.label(instance, label, Some(sub))?;
                    // The label is its own control, so the check box has no
                    // text, which would be drawn in the wrong colour in the
                    // dark theme.
                    let style = tab | (BS_AUTOCHECKBOX as u32);
                    let check = child(
                        self.hwnd,
                        instance,
                        w!("BUTTON"),
                        "",
                        style,
                        ID_CONFIRM_DELETE,
                    )?;
                    // SAFETY: no pointers.
                    unsafe {
                        SendMessageW(check, BM_SETCHECK, Some(WPARAM(usize::from(*on))), None);
                    }
                    self.check = check;
                    self.entries.push(Entry::Row {
                        label: label_hwnd,
                        text: label,
                        sub,
                        control: check,
                        drop_rows: None,
                    });
                }
            }
        }
        Ok(())
    }

    /// The label control of a row, and its subtitle.
    fn label(
        &self,
        instance: HINSTANCE,
        label: &'static str,
        sub: Option<&'static str>,
    ) -> windows::core::Result<(HWND, Option<(HWND, &'static str)>)> {
        let statics = SS_NOPREFIX;
        let label_hwnd = child(self.hwnd, instance, w!("STATIC"), label, statics, 0)?;
        let sub = match sub {
            Some(text) => Some((
                child(self.hwnd, instance, w!("STATIC"), text, statics, 0)?,
                text,
            )),
            None => None,
        };
        Ok((label_hwnd, sub))
    }

    /// Fonts, brushes, control themes and the title bar for `theme` at the
    /// window's DPI.
    fn style(&self, theme: &Theme) {
        let dpi = self.dpi.get();
        let fonts = dialog::message_fonts(dpi);
        let brushes = make_brushes(theme);
        let old_fonts = self.fonts.replace(fonts);
        let old_brushes = self.brushes.replace(brushes);
        let set_font = |hwnd: HWND, font: HFONT| {
            // SAFETY: the font stays alive until the next restyle replaces
            // it on every control.
            unsafe { SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(font.0 as usize)), None) };
        };
        // Dark mode control themes; the light ones are the usual ones. How
        // far they go is up to Windows: the combo box and check box get
        // their dark look from the theme, the rest from WM_CTLCOLOR*.
        let (buttons, combos) = if theme.dark {
            (w!("DarkMode_Explorer"), w!("DarkMode_CFD"))
        } else {
            (w!("Explorer"), w!("CFD"))
        };
        for entry in &self.entries {
            match entry {
                Entry::Heading(h, _) => set_font(*h, fonts[1]),
                Entry::Row {
                    label,
                    sub,
                    control,
                    drop_rows,
                    ..
                } => {
                    set_font(*label, fonts[0]);
                    if let Some((sub, _)) = sub {
                        set_font(*sub, fonts[0]);
                    }
                    set_font(*control, fonts[0]);
                    // SAFETY: static strings; live windows.
                    unsafe {
                        if drop_rows.is_some() {
                            let _ = SetWindowTheme(*control, combos, PCWSTR::null());
                            // The drop down list's scroll bar.
                            let mut info = COMBOBOXINFO {
                                cbSize: size_of::<COMBOBOXINFO>() as u32,
                                ..Default::default()
                            };
                            if GetComboBoxInfo(*control, &mut info).is_ok() {
                                let _ = SetWindowTheme(info.hwndList, buttons, PCWSTR::null());
                            }
                        } else {
                            let _ = SetWindowTheme(*control, buttons, PCWSTR::null());
                        }
                    }
                }
            }
        }
        theme.apply_to_frame(self.hwnd);
        dialog::refresh_frame(self.hwnd);
        dialog::delete_objects(old_fonts.into_iter().map(HGDIOBJ::from));
        dialog::delete_objects(old_brushes.into_iter().map(HGDIOBJ::from));
        // SAFETY: plain call on our own window and its children.
        unsafe {
            let _ = RedrawWindow(
                Some(self.hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        }
    }

    /// Positions every control for the current DPI and fonts, and resizes
    /// the window around them, keeping its top left corner.
    fn fit(&self) {
        let dpi = self.dpi.get();
        let s = |v: i32| scale(v, dpi);
        let [normal, bold] = self.fonts.get();
        let margin = s(MARGIN);
        let width = s(CLIENT_WIDTH);
        let control_width = s(CONTROL_WIDTH);
        let control_x = width - margin - control_width;
        let text_width = control_x - s(GAP) - margin;
        // SAFETY: the DC is released below; plain calls on live windows.
        unsafe {
            let hdc = GetDC(Some(self.hwnd));
            let mut y = margin;
            for (i, entry) in self.entries.iter().enumerate() {
                match entry {
                    Entry::Heading(h, text) => {
                        if i > 0 {
                            y += s(HEADING_GAP);
                        }
                        let height = dialog::text_height(hdc, bold, text, width - 2 * margin);
                        let _ = MoveWindow(*h, margin, y, width - 2 * margin, height, true);
                        y += height + s(8);
                    }
                    Entry::Row {
                        label,
                        text,
                        sub,
                        control,
                        drop_rows,
                    } => {
                        let label_height = dialog::text_height(hdc, normal, text, text_width);
                        let sub_height = sub.map_or(0, |(_, text)| {
                            s(2) + dialog::text_height(hdc, normal, text, text_width)
                        });
                        let block = label_height + sub_height;
                        // A combo box's height includes its list; it keeps
                        // only the closed part for itself.
                        let control_height = match drop_rows {
                            Some(rows) => {
                                let dropped = s(COMBO_HEIGHT) + s(DROP_ROW) * *rows as i32 + s(4);
                                let _ = MoveWindow(
                                    *control,
                                    control_x,
                                    y,
                                    control_width,
                                    dropped,
                                    true,
                                );
                                let mut rc = RECT::default();
                                let _ = GetWindowRect(*control, &mut rc);
                                (rc.bottom - rc.top).max(s(COMBO_HEIGHT))
                            }
                            None => s(CHECK_SIZE),
                        };
                        let row = block.max(control_height);
                        let label_y = y + (row - block) / 2;
                        let _ = MoveWindow(*label, margin, label_y, text_width, label_height, true);
                        if let Some((sub, _)) = sub {
                            let sub_y = label_y + label_height + s(2);
                            let sub_height = sub_height - s(2);
                            let _ = MoveWindow(*sub, margin, sub_y, text_width, sub_height, true);
                        }
                        let control_y = y + (row - control_height) / 2;
                        let (w, h) = match drop_rows {
                            Some(rows) => (
                                control_width,
                                s(COMBO_HEIGHT) + s(DROP_ROW) * *rows as i32 + s(4),
                            ),
                            None => (s(CHECK_SIZE), control_height),
                        };
                        let _ = MoveWindow(*control, control_x, control_y, w, h, true);
                        y += row + s(ROW_GAP);
                    }
                }
            }
            ReleaseDC(Some(self.hwnd), hdc);
            let client_height = y - s(ROW_GAP) + margin;
            let (w, h) = dialog::outer_size(STYLE, width, client_height, dpi);
            let flags = SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE;
            let _ = SetWindowPos(self.hwnd, None, 0, 0, w, h, flags);
        }
    }

    fn first_control(&self) -> HWND {
        self.combos.first().map_or(self.hwnd, |c| c.hwnd)
    }
}

/// The window background and the field background brushes for `theme`.
fn make_brushes(theme: &Theme) -> [HBRUSH; 2] {
    let field = if theme.dark {
        theme.code_background
    } else {
        theme.background
    };
    // SAFETY: the brushes are deleted by the window when replaced or closed.
    unsafe { [CreateSolidBrush(theme.background), CreateSolidBrush(field)] }
}

impl App {
    /// Ctrl+,: opens the Preferences window, or shows the open one.
    pub(super) fn open_prefs(&self) {
        let open = self.prefs.borrow().as_ref().map(|p| p.hwnd);
        if let Some(hwnd) = open {
            dialog::bring_forward(hwnd);
            return;
        }
        self.finish_swipe();
        self.hide_suggestions();
        let prefs = match Prefs::create(self, self.hwnd) {
            Ok(prefs) => prefs,
            Err(_) => {
                self.show_status("Could not open the preferences");
                return;
            }
        };
        let hwnd = prefs.hwnd;
        *self.prefs.borrow_mut() = Some(prefs);
        let guard = self.prefs.borrow();
        let Some(prefs) = guard.as_ref() else {
            return;
        };
        // SAFETY: plain call on the window just created.
        prefs
            .dpi
            .set(unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) });
        prefs.style(&self.theme.get());
        prefs.fit();
        let mut rc = RECT::default();
        // SAFETY: `rc` is a live local; plain calls.
        unsafe {
            let _ = GetWindowRect(hwnd, &mut rc);
            dialog::centre_over(self.hwnd, hwnd, rc.right - rc.left, rc.bottom - rc.top);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetFocus(Some(prefs.first_control()));
        }
    }

    /// Esc or the close button: closes the Preferences window.
    pub(super) fn close_prefs(&self) {
        let open = self.prefs.borrow().as_ref().map(|p| p.hwnd);
        if let Some(hwnd) = open {
            // SAFETY: our own window; WM_DESTROY drops the state.
            let _ = unsafe { DestroyWindow(hwnd) };
        }
    }

    /// Alt+P changed the paper: the combo box shows it too.
    pub(super) fn sync_paper_choice(&self) {
        let guard = self.prefs.borrow();
        let Some(prefs) = guard.as_ref() else {
            return;
        };
        let name = self.paper.get().name();
        if let Some(combo) = prefs.combos.iter().find(|c| c.key == "paper") {
            let index = combo.values.iter().position(|v| v == name).unwrap_or(0);
            // SAFETY: no pointers.
            unsafe { SendMessageW(combo.hwnd, CB_SETCURSEL, Some(WPARAM(index)), None) };
        }
    }

    /// The theme changed: both extra windows follow.
    pub(super) fn restyle_popups(&self) {
        let theme = self.theme.get();
        if let Some(prefs) = self.prefs.borrow().as_ref() {
            prefs.style(&theme);
        }
        self.restyle_keys();
    }

    /// A setting was changed in the window: applies and saves it.
    fn apply_choice(&self, key: &str, value: &str) {
        match key {
            "paper" => self.set_paper(Paper::parse(value)),
            "color_scheme" => {
                self.save_setting(key, value);
                self.retheme();
            }
            "hide_symbols_scope" => {
                self.save_setting(key, value);
                self.scope_changed();
            }
            // The other choices are read when Whisp starts.
            _ => self.save_setting(key, value),
        }
    }

    /// The note shown takes the hidden or shown state of the new scope. The
    /// states of the scopes are not copied into each other, as in Whisp.
    fn scope_changed(&self) {
        let on = self.hides_symbols(self.current.get());
        if self.editor().hides() == on {
            return;
        }
        // The peek editor may hold a neighbour styled the old way.
        self.peek.set(None);
        self.busy.set(true);
        self.editor().set_hide(&self.theme.get(), on);
        self.busy.set(false);
    }

    /// Whether a message for `msg.hwnd` belongs to one of the two extra
    /// windows. `Some(true)` means it is used up, `Some(false)` that it goes
    /// on to the window, and `None` that it is for the main window.
    pub(super) fn popup_message(&self, msg: &mut MSG) -> Option<bool> {
        let prefs = self.prefs.borrow().as_ref().map(|p| p.hwnd);
        let keys = self.keys_hwnd();
        let in_prefs = prefs.is_some_and(|h| dialog::belongs(h, msg.hwnd));
        let in_keys = keys.is_some_and(|h| dialog::belongs(h, msg.hwnd));
        if !in_prefs && !in_keys {
            return None;
        }
        if msg.message == WM_KEYDOWN {
            let key = VIRTUAL_KEY(msg.wParam.0 as u16);
            // SAFETY: plain calls.
            let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
            if key == VK_ESCAPE {
                // SAFETY: a message to the focused control; only combo boxes
                // answer it.
                let dropped = unsafe { SendMessageW(msg.hwnd, CB_GETDROPPEDSTATE, None, None) };
                // Esc closes an open drop down list before the window.
                if dropped.0 == 0 {
                    if in_prefs {
                        self.close_prefs();
                    } else {
                        self.close_keys();
                    }
                    return Some(true);
                }
                return Some(false);
            }
            if down(VK_CONTROL) && !down(VK_MENU) {
                if key == VK_OEM_COMMA {
                    self.open_prefs();
                    return Some(true);
                }
                if key == VK_OEM_2 {
                    self.open_keys();
                    return Some(true);
                }
            }
        }
        // Tab, Shift+Tab, arrow keys and mnemonics between the controls.
        if let Some(hwnd) = prefs.filter(|_| in_prefs) {
            // SAFETY: plain call; `msg` is a live message.
            if unsafe { IsDialogMessageW(hwnd, msg) }.as_bool() {
                return Some(true);
            }
        }
        Some(false)
    }

    /// Messages of the Preferences window.
    pub(super) fn prefs_message(
        &self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        if msg == WM_DESTROY {
            let taken = self.prefs.borrow_mut().take();
            if let Some(prefs) = taken.filter(|p| p.hwnd == hwnd) {
                dialog::delete_objects(prefs.fonts.get().into_iter().map(HGDIOBJ::from));
                dialog::delete_objects(prefs.brushes.get().into_iter().map(HGDIOBJ::from));
            }
            return None;
        }
        let guard = self.prefs.borrow();
        let prefs = guard.as_ref().filter(|p| p.hwnd == hwnd)?;
        match msg {
            WM_ERASEBKGND => {
                let mut rc = RECT::default();
                // SAFETY: wparam is the DC to erase.
                unsafe {
                    let _ = GetClientRect(hwnd, &mut rc);
                    FillRect(HDC(wparam.0 as *mut _), &rc, prefs.brushes.get()[0]);
                }
                Some(LRESULT(1))
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLORLISTBOX | WM_CTLCOLOREDIT => {
                let (hdc, control) = (HDC(wparam.0 as *mut _), HWND(lparam.0 as *mut _));
                Some(prefs.color(&self.theme.get(), msg, hdc, control))
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xffff) as u16;
                let code = (wparam.0 >> 16) as u32;
                let change = if code == CBN_SELCHANGE {
                    prefs.combos.iter().find(|c| c.id == id).and_then(|combo| {
                        // SAFETY: no pointers.
                        let sel = unsafe { SendMessageW(combo.hwnd, CB_GETCURSEL, None, None) };
                        let value = combo.values.get(usize::try_from(sel.0).ok()?)?;
                        Some((combo.key, value.clone()))
                    })
                } else if code == BN_CLICKED && id == ID_CONFIRM_DELETE {
                    // SAFETY: no pointers.
                    let on = unsafe { SendMessageW(prefs.check, BM_GETCHECK, None, None) };
                    Some((
                        "confirm_delete",
                        if on.0 != 0 { "1" } else { "0" }.to_string(),
                    ))
                } else {
                    None
                };
                let cancel = id == IDCANCEL.0 as u16 && code == BN_CLICKED;
                drop(guard);
                if let Some((key, value)) = change {
                    self.apply_choice(key, &value);
                } else if cancel {
                    self.close_prefs();
                }
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                // SAFETY: plain calls on live windows.
                unsafe {
                    if (wparam.0 & 0xffff) as u32 == WA_INACTIVE {
                        let focus = GetFocus();
                        if dialog::belongs(hwnd, focus) && focus != hwnd {
                            prefs.focus.set(focus);
                        }
                    } else {
                        let target = prefs.focus.get();
                        let target = if target.is_invalid() {
                            prefs.first_control()
                        } else {
                            target
                        };
                        let _ = SetFocus(Some(target));
                    }
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                prefs.dpi.set((wparam.0 & 0xffff) as u32);
                // SAFETY: for WM_DPICHANGED, lparam points to the suggested window RECT.
                unsafe {
                    let rc = *(lparam.0 as *const RECT);
                    let flags = SWP_NOZORDER | SWP_NOACTIVATE;
                    let (x, y) = (rc.left, rc.top);
                    let _ = SetWindowPos(hwnd, None, x, y, 0, 0, flags | SWP_NOSIZE);
                }
                prefs.style(&self.theme.get());
                prefs.fit();
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                drop(guard);
                self.close_prefs();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

impl Prefs {
    /// The colours of a child control, for WM_CTLCOLOR*.
    fn color(&self, theme: &Theme, msg: u32, hdc: HDC, control: HWND) -> LRESULT {
        let [background, field] = self.brushes.get();
        let list = msg == WM_CTLCOLORLISTBOX || msg == WM_CTLCOLOREDIT;
        let dim = self
            .entries
            .iter()
            .any(|e| matches!(e, Entry::Row { sub: Some((h, _)), .. } if *h == control));
        let field_color = if theme.dark {
            theme.code_background
        } else {
            theme.background
        };
        // SAFETY: hdc is the control's DC for this paint.
        unsafe {
            SetTextColor(hdc, if dim { theme.dim() } else { theme.text });
            SetBkColor(hdc, if list { field_color } else { theme.background });
        }
        LRESULT(if list { field } else { background }.0 as isize)
    }
}
