//! Paper backgrounds behind the editors' text.
//!
//! A windowed RichEdit paints its own opaque background. With a paper
//! chosen, the editor's WM_PAINT is replaced: RichEdit draws the update area
//! into an offscreen bitmap through ITextServices::TxDraw (what its own
//! window host does in WM_PAINT), the pixels that are exactly the background
//! colour are replaced by the pattern, and the result is copied to the
//! window in one blit, so nothing flickers. With blank paper every message
//! goes straight to RichEdit.
//!
//! The pattern is fixed to the editor's client area. RichEdit scrolls by
//! moving the window's pixels (ScrollWindowEx), which would move the pattern
//! with the text, so a change of scroll position repaints the whole editor.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Instant;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, RECTL, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::DVASPECT_CONTENT;
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::UI::Controls::RichEdit::{EM_GETSCROLLPOS, ITextServices, TXTVIEW_ACTIVE};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::HSTRING;

use super::theme::Theme;
use crate::paper::{self, Paper};

/// Logs `whisp: paint <ms> ms` at most once a second.
const LOG_PAINT_COST: bool = false;

/// The paper and the colours it is drawn in.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Look {
    paper: Paper,
    /// Background and ink as `0x00RRGGBB`.
    background: u32,
    ink: u32,
}

/// An offscreen 32 bit DIB with a DC to draw into it.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    width: usize,
    height: usize,
}

impl Surface {
    fn new(width: usize, height: usize) -> Option<Surface> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                // Negative: top down rows.
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: `info` describes the bitmap; `bits` receives its memory,
        // which lives until the bitmap is deleted in `drop`.
        unsafe {
            let bitmap = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let dc = CreateCompatibleDC(None);
            let old = SelectObject(dc, bitmap.into());
            Some(Surface {
                dc,
                bitmap,
                old,
                bits: bits as *mut u32,
                width,
                height,
            })
        }
    }

    fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: the DIB holds width * height 32 bit pixels while it lives.
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.width * self.height) }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: these objects were created by `new` and are released once.
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// What an editor needs for painting paper, kept between paints.
struct Cache {
    surface: Surface,
    pattern: Vec<u32>,
    /// What `pattern` was drawn for.
    drawn_for: (Look, u32),
}

struct Editor {
    /// RichEdit's own window procedure.
    original: isize,
    services: Option<ITextServices>,
    cache: Option<Cache>,
    /// Scroll position at the last paper paint.
    scroll: Option<(i32, i32)>,
}

thread_local! {
    static LOOK: Cell<Look> = const {
        Cell::new(Look { paper: Paper::Blank, background: 0, ink: 0 })
    };
    static EDITORS: RefCell<HashMap<isize, Editor>> = RefCell::new(HashMap::new());
    static PAINTING: Cell<bool> = const { Cell::new(false) };
    static LAST_LOG: Cell<Option<Instant>> = const { Cell::new(None) };
}

fn rgb(c: COLORREF) -> u32 {
    ((c.0 & 0xff) << 16) | (c.0 & 0xff00) | ((c.0 >> 16) & 0xff)
}

/// Starts painting paper for a RichEdit window. Without `services`, it
/// keeps painting itself, on blank paper.
pub fn attach(hwnd: HWND, services: Option<ITextServices>) {
    // SAFETY: replaces the window procedure of our own window; the old one
    // is kept and called for everything not handled here.
    let original = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, subclass as *const () as isize) };
    EDITORS.with(|e| {
        e.borrow_mut().insert(
            hwnd.0 as isize,
            Editor {
                original,
                services,
                cache: None,
                scroll: None,
            },
        )
    });
}

/// Sets the paper and colours, and repaints the editors.
pub fn set(paper: Paper, theme: &Theme) {
    LOOK.with(|l| {
        l.set(Look {
            paper,
            background: rgb(theme.background),
            ink: rgb(theme.text),
        })
    });
    let windows: Vec<isize> = EDITORS.with(|e| {
        let mut editors = e.borrow_mut();
        for editor in editors.values_mut() {
            if paper == Paper::Blank {
                // Frees the bitmaps; blank paper needs none.
                editor.cache = None;
            }
            editor.scroll = None;
        }
        editors.keys().copied().collect()
    });
    for hwnd in windows {
        // SAFETY: plain call on an editor window.
        let _ = unsafe { InvalidateRect(Some(HWND(hwnd as *mut _)), None, false) };
    }
}

fn original_of(hwnd: HWND) -> WNDPROC {
    let original = EDITORS.with(|e| e.borrow().get(&(hwnd.0 as isize)).map_or(0, |e| e.original));
    // SAFETY: `original` is the window procedure SetWindowLongPtrW returned
    // (or 0, which is None).
    unsafe { std::mem::transmute::<isize, WNDPROC>(original) }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let original = original_of(hwnd);
    if msg == WM_NCDESTROY {
        EDITORS.with(|e| e.borrow_mut().remove(&(hwnd.0 as isize)));
    }
    let paper = LOOK.with(|l| l.get().paper);
    if paper == Paper::Blank || original.is_none() {
        // SAFETY: forwarding the message as received.
        return unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) };
    }
    let painting = PAINTING.with(Cell::get);
    if msg == WM_PAINT && !painting && paint(hwnd, original) {
        return LRESULT(0);
    }
    // SAFETY: forwarding the message as received.
    let result = unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) };
    if msg != EM_GETSCROLLPOS && msg != WM_PAINT && !painting {
        follow_scroll(hwnd, original);
    }
    result
}

fn scroll_position(hwnd: HWND, original: WNDPROC) -> (i32, i32) {
    let mut point = POINT::default();
    // SAFETY: EM_GETSCROLLPOS writes the POINT during the call.
    unsafe {
        CallWindowProcW(
            original,
            hwnd,
            EM_GETSCROLLPOS,
            WPARAM(0),
            LPARAM(&mut point as *mut POINT as isize),
        )
    };
    (point.x, point.y)
}

/// After any message that scrolled the editor: repaint it all at once, so
/// the pattern the scroll moved is put back in place.
fn follow_scroll(hwnd: HWND, original: WNDPROC) {
    let painted = EDITORS.with(|e| e.borrow().get(&(hwnd.0 as isize)).and_then(|e| e.scroll));
    let Some(painted) = painted else {
        return;
    };
    if scroll_position(hwnd, original) != painted {
        // SAFETY: plain call on our own window.
        let _ = unsafe { RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_UPDATENOW) };
    }
}

/// Paints the editor with paper; false to leave it to RichEdit.
fn paint(hwnd: HWND, original: WNDPROC) -> bool {
    let key = hwnd.0 as isize;
    // Taken out of the map while painting, as TxDraw may send messages.
    let Some((services, cache, painted)) = EDITORS.with(|e| {
        let mut editors = e.borrow_mut();
        let editor = editors.get_mut(&key)?;
        let services = editor.services.clone()?;
        Some((services, editor.cache.take(), editor.scroll))
    }) else {
        return false;
    };
    let started = Instant::now();
    let look = LOOK.with(Cell::get);
    let mut client = RECT::default();
    // SAFETY: plain calls on our own window.
    let dpi = unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        GetDpiForWindow(hwnd)
    };
    let (width, height) = (client.right.max(0) as usize, client.bottom.max(0) as usize);
    let cache = match cache {
        Some(c) if c.surface.width == width && c.surface.height == height => Some(c),
        _ => Surface::new(width, height).map(|surface| Cache {
            surface,
            pattern: Vec::new(),
            drawn_for: (look, 0),
        }),
    };
    let Some(mut cache) = cache else {
        // No bitmap (an empty window, or out of memory): RichEdit paints.
        return false;
    };
    if cache.drawn_for != (look, dpi) || cache.pattern.len() != width * height {
        let scale = f64::from(dpi) / 96.0;
        cache.pattern = paper::render(look.paper, width, height, scale, look.background, look.ink);
        cache.drawn_for = (look, dpi);
    }

    PAINTING.with(|p| p.set(true));
    let scroll = scroll_position(hwnd, original);
    // SAFETY: painting our own window between BeginPaint and EndPaint, into
    // the DIB that `cache` owns.
    let drawn = unsafe {
        if painted != Some(scroll) {
            // Scrolling moved the pattern with the text: paint everything.
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut update = RECT {
            left: ps.rcPaint.left.clamp(0, client.right),
            top: ps.rcPaint.top.clamp(0, client.bottom),
            right: ps.rcPaint.right.clamp(0, client.right),
            bottom: ps.rcPaint.bottom.clamp(0, client.bottom),
        };
        let mut bounds = RECTL {
            left: 0,
            top: 0,
            right: client.right,
            bottom: client.bottom,
        };
        let mut drawn = true;
        if update.right > update.left && update.bottom > update.top {
            // What RichEdit's own window host does in WM_PAINT, but into
            // the offscreen bitmap.
            drawn = services
                .TxDraw(
                    DVASPECT_CONTENT,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    cache.surface.dc,
                    HDC::default(),
                    &mut bounds,
                    std::ptr::null_mut(),
                    &mut update,
                    0,
                    0,
                    TXTVIEW_ACTIVE.0,
                )
                .is_ok();
            if drawn {
                let _ = GdiFlush();
                let rect = (
                    update.left as usize,
                    update.top as usize,
                    update.right as usize,
                    update.bottom as usize,
                );
                let pattern = std::mem::take(&mut cache.pattern);
                paper::composite(
                    cache.surface.pixels(),
                    &pattern,
                    width,
                    rect,
                    look.background,
                );
                cache.pattern = pattern;
                let _ = BitBlt(
                    hdc,
                    update.left,
                    update.top,
                    update.right - update.left,
                    update.bottom - update.top,
                    Some(cache.surface.dc),
                    update.left,
                    update.top,
                    SRCCOPY,
                );
            }
        }
        let _ = EndPaint(hwnd, &ps);
        drawn
    };
    PAINTING.with(|p| p.set(false));
    EDITORS.with(|e| {
        if let Some(editor) = e.borrow_mut().get_mut(&key) {
            if drawn {
                editor.cache = Some(cache);
                editor.scroll = Some(scroll);
            } else {
                // TxDraw failed: stop painting paper here and let RichEdit
                // paint itself from now on.
                editor.services = None;
            }
        }
    });
    if !drawn {
        // SAFETY: plain call on our own window.
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
        debug("whisp: paper disabled, TxDraw failed\n".to_string());
        return true;
    }
    if LOG_PAINT_COST {
        let now = Instant::now();
        let due = LAST_LOG.with(|l| l.get().is_none_or(|t| now.duration_since(t).as_secs() >= 1));
        if due {
            LAST_LOG.with(|l| l.set(Some(now)));
            debug(format!(
                "whisp: paint {:.2} ms\n",
                started.elapsed().as_secs_f64() * 1000.0
            ));
        }
    }
    true
}

fn debug(text: String) {
    // SAFETY: the string outlives the call.
    unsafe { OutputDebugStringW(&HSTRING::from(text)) };
}
