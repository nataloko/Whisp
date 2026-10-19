//! Two finger swipe between notes. The current editor follows the fingers
//! and a second, hidden "peek" editor shows the neighbour note beside it.
//! On commit the two editors swap roles, so nothing reloads on screen.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::HSTRING;

use super::App;
use super::editor::Editor;
use crate::gesture::{self, Settle, Side, Swipe};
use crate::notes;

/// Fires when no horizontal wheel event came for `gesture::END_MS`.
pub(super) const TIMER_SWIPE_END: usize = 2;
/// Posted by the frame pacer once per display refresh while settling.
pub(super) const WM_SWIPE_FRAME: u32 = WM_APP + 2;

/// Paces the settle animation: a thread posts WM_SWIPE_FRAME once per
/// display refresh, one at a time. Window timers tick only every 15.6 ms
/// or 31 ms, which made the animation uneven.
pub(super) struct Pacer {
    running: AtomicBool,
    posted: AtomicBool,
}

pub(super) enum SwipeState {
    Idle,
    /// Following wheel events.
    Tracking(Swipe),
    /// Animating to the neighbour (`target`) or back (`None`).
    Settling {
        settle: Settle,
        target: Option<Side>,
        /// Offset and speed when the gesture ended, for the debug line.
        offset: f64,
        speed: f64,
        frames: u32,
        last_frame: u64,
        slowest: u64,
    },
}

impl App {
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn client_size(&self) -> (i32, i32) {
        let mut rc = RECT::default();
        // SAFETY: `rc` is a live local.
        let _ = unsafe { GetClientRect(self.hwnd, &mut rc) };
        (rc.right, rc.bottom)
    }

    /// Whether there is an older and a newer note (or the empty note).
    fn neighbours(&self) -> (bool, bool) {
        let current = self.current.get();
        (current > 0, current < self.notes.borrow().len())
    }

    /// The peek editor, created on first use.
    pub(super) fn peek_editor(&self) -> Option<&Editor> {
        let slot = &self.editors[1 - self.active.get()];
        if slot.get().is_none() {
            let editor = Editor::create(self.hwnd, self.instance, false).ok()?;
            editor.apply_theme(&self.theme.get());
            let _ = slot.set(editor);
            self.layout();
        }
        slot.get()
    }

    /// A horizontal wheel event: a touchpad swipe or a tilt wheel.
    /// Positive deltas move toward the newer note.
    pub(super) fn swipe_wheel(&self, delta: i32) {
        let now = self.now();
        let width = f64::from(self.client_size().0);
        let mut swipe = match self.swipe.replace(SwipeState::Idle) {
            SwipeState::Tracking(swipe) => swipe,
            SwipeState::Settling { settle, .. } => {
                self.stop_frames();
                let (older, newer) = self.neighbours();
                Swipe::resume(width, older, newer, settle.at(now).0, now)
            }
            SwipeState::Idle => {
                // Saved first, so a new note has its place in the list and
                // the neighbours are known. The peek editor may hold a note
                // from before keyboard navigation, so it is loaded afresh.
                self.flush();
                self.peek.set(None);
                let (older, newer) = self.neighbours();
                Swipe::new(width, older, newer, now)
            }
        };
        swipe.scroll(delta, now);
        if let Some(side) = swipe.side()
            && swipe.can(side)
            && self.peek.get().map(|(s, _)| s) != Some(side)
            && !self.load_peek(side)
        {
            swipe.block(side);
        }
        self.place(swipe.offset());
        self.swipe.replace(SwipeState::Tracking(swipe));
        // SAFETY: plain call; restarting the timer resets its delay.
        unsafe {
            SetTimer(
                Some(self.hwnd),
                TIMER_SWIPE_END,
                gesture::END_MS as u32,
                None,
            )
        };
    }

    /// Loads the neighbour on `side` into the peek editor.
    fn load_peek(&self, side: Side) -> bool {
        let current = self.current.get();
        let index = match side {
            Side::Older => current - 1,
            Side::Newer => current + 1,
        };
        let path = self.notes.borrow().get(index).cloned();
        let text = match path {
            None => String::new(),
            Some(path) => match notes::read(&path) {
                Ok(text) => text,
                Err(_) => return false,
            },
        };
        let Some(peek) = self.peek_editor() else {
            return false;
        };
        self.busy.set(true);
        peek.load(&text, &self.theme.get());
        peek.set_read_only(true);
        self.busy.set(false);
        self.peek.set(Some((side, index)));
        true
    }

    /// Moves the current editor left by `offset` pixels, with the peek
    /// editor beside it when its side is showing.
    fn place(&self, offset: f64) {
        let (width, height) = self.client_size();
        let x = -(offset.round() as i32);
        let peek = self.editors[1 - self.active.get()].get();
        let flags = SWP_NOZORDER | SWP_NOACTIVATE;
        // SAFETY: both are live child windows of ours.
        unsafe {
            let Ok(dwp) = BeginDeferWindowPos(2) else {
                return;
            };
            let Ok(mut dwp) =
                DeferWindowPos(dwp, self.editor().hwnd, None, x, 0, width, height, flags)
            else {
                return;
            };
            if let Some(peek) = peek {
                let side = self.peek.get().map(|(s, _)| s);
                let showing = side.filter(|s| offset * s.sign() > 0.0);
                let (peek_x, shown) = match showing {
                    Some(side) => (x + side.sign() as i32 * width, SWP_SHOWWINDOW),
                    None => (0, SWP_HIDEWINDOW),
                };
                if let Ok(next) = DeferWindowPos(
                    dwp,
                    peek.hwnd,
                    None,
                    peek_x,
                    0,
                    width,
                    height,
                    flags | shown,
                ) {
                    dwp = next;
                }
            }
            let _ = EndDeferWindowPos(dwp);
        }
    }

    pub(super) fn swipe_timer(&self, id: usize) {
        if id != TIMER_SWIPE_END {
            return;
        }
        if let SwipeState::Tracking(swipe) = &*self.swipe.borrow()
            && !swipe.ended(self.now())
        {
            // Timers can fire early; wait for the rest.
            return;
        }
        self.end_swipe(true);
    }

    /// Ends the gesture: decides where it goes and animates there, or with
    /// `animate` false goes there at once. Ending an animation finishes it.
    fn end_swipe(&self, animate: bool) {
        let now = self.now();
        // SAFETY: plain call.
        let _ = unsafe { KillTimer(Some(self.hwnd), TIMER_SWIPE_END) };
        self.stop_frames();
        match self.swipe.replace(SwipeState::Idle) {
            SwipeState::Idle => {}
            SwipeState::Tracking(swipe) => {
                let offset = swipe.offset();
                let speed = swipe.speed();
                // Without animation the gesture was cut short by a key or
                // click, so only the distance travelled counts. Committing
                // follows the navigation rules: nothing moves unless the
                // current note is saved.
                let target = if animate {
                    swipe.decide()
                } else {
                    swipe.decide_by_distance()
                }
                .filter(|_| self.flush());
                if !animate {
                    self.settled(target, offset, speed, 0, 0);
                    return;
                }
                let width = f64::from(self.client_size().0);
                let settle = Settle {
                    from: offset,
                    to: target.map_or(0.0, |s| s.sign() * width),
                    start_ms: now,
                };
                self.swipe.replace(SwipeState::Settling {
                    settle,
                    target,
                    offset,
                    speed,
                    frames: 0,
                    last_frame: now,
                    slowest: 0,
                });
                self.start_frames();
            }
            SwipeState::Settling {
                target,
                offset,
                speed,
                frames,
                slowest,
                ..
            } => self.settled(target, offset, speed, frames, slowest),
        }
    }

    fn swipe_frame(&self) {
        let now = self.now();
        let (offset, done) = match &mut *self.swipe.borrow_mut() {
            SwipeState::Settling {
                settle,
                frames,
                last_frame,
                slowest,
                ..
            } => {
                *frames += 1;
                *slowest = (*slowest).max(now - *last_frame);
                *last_frame = now;
                settle.at(now)
            }
            // A frame posted before the animation ended.
            _ => return,
        };
        self.place(offset);
        if done {
            self.end_swipe(false);
        }
    }

    /// WM_SWIPE_FRAME from the pacer.
    pub(super) fn swipe_frame_message(&self) {
        if let Some(pacer) = &*self.pacer.borrow() {
            pacer.posted.store(false, Ordering::Release);
        }
        self.swipe_frame();
    }

    fn start_frames(&self) {
        if self.pacer.borrow().is_some() {
            return;
        }
        let pacer = Arc::new(Pacer {
            running: AtomicBool::new(true),
            posted: AtomicBool::new(false),
        });
        let shared = Arc::clone(&pacer);
        let period = refresh_period();
        // HWND is not Send; the raw value is only used to post to our window.
        let hwnd = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            // Sleeping until each deadline rather than for a fixed time keeps
            // errors from adding up. Rust's sleep uses a high resolution
            // timer on Windows.
            let mut next = Instant::now() + period;
            while shared.running.load(Ordering::Acquire) {
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
                next += period;
                if shared.running.load(Ordering::Acquire)
                    && !shared.posted.swap(true, Ordering::AcqRel)
                {
                    // SAFETY: plain call; posting to a destroyed window fails.
                    let _ = unsafe {
                        PostMessageW(
                            Some(HWND(hwnd as *mut _)),
                            WM_SWIPE_FRAME,
                            WPARAM(0),
                            LPARAM(0),
                        )
                    };
                }
            }
        });
        self.pacer.replace(Some(pacer));
    }

    pub(super) fn stop_frames(&self) {
        if let Some(pacer) = self.pacer.take() {
            pacer.running.store(false, Ordering::Release);
        }
    }

    /// Finishes a gesture or animation at once, as if it had run its course.
    /// Used before keys, clicks and focus changes reach a moving editor.
    pub(super) fn finish_swipe(&self) {
        if !matches!(*self.swipe.borrow(), SwipeState::Idle) {
            self.end_swipe(false);
        }
    }

    /// Drops a gesture without moving, as on a resize.
    pub(super) fn cancel_swipe(&self) {
        if !matches!(self.swipe.replace(SwipeState::Idle), SwipeState::Idle) {
            // SAFETY: plain call.
            let _ = unsafe { KillTimer(Some(self.hwnd), TIMER_SWIPE_END) };
            self.stop_frames();
            debug("whisp: swipe cancelled by a resize\n".to_string());
        }
    }

    fn settled(&self, target: Option<Side>, offset: f64, speed: f64, frames: u32, slowest: u64) {
        let swapped = target.is_some_and(|side| self.swap_to(side));
        if !swapped {
            self.place(0.0);
        }
        let outcome = target.filter(|_| swapped).map_or("cancel", Side::name);
        debug(format!(
            "whisp: swipe offset {offset:.0} speed {speed:.0} -> {outcome}, {frames} frames, slowest frame {slowest} ms\n"
        ));
    }

    /// Makes the peek editor, which shows the neighbour on `side`, the
    /// current one. Its `load` already left it as `show` would: styled line
    /// signatures, no undo history, caret at the end, notifications on.
    fn swap_to(&self, side: Side) -> bool {
        let Some((_, index)) = self.peek.get().filter(|&(s, _)| s == side) else {
            return false;
        };
        // Leaving the current note, as `go` does.
        let removed = self.drop_if_blank();
        let index = if removed && side == Side::Newer {
            index - 1
        } else {
            index
        };
        let old = self.active.get();
        self.active.set(1 - old);
        self.editor().set_read_only(false);
        self.peek.set(None);
        self.generation.set(self.generation.get() + 1);
        self.current.set(index);
        self.dirty.set(false);
        self.update_title();
        // New current editor at rest, old one hidden.
        self.place(0.0);
        // SAFETY: the new current editor is a live child window.
        unsafe {
            let _ = ShowWindow(self.editor().hwnd, SW_SHOWNA);
            let _ = SetFocus(Some(self.editor().hwnd));
        }
        // Free the old note's text.
        if let Some(previous) = self.editors[old].get() {
            self.busy.set(true);
            previous.load("", &self.theme.get());
            previous.set_read_only(true);
            self.busy.set(false);
        }
        true
    }

    /// Handles input before it is dispatched; returns true if it was used.
    /// Keyboard input aimed at the editor that is not current goes to the
    /// current one instead.
    pub(super) fn intercept(&self, msg: &mut MSG) -> bool {
        match msg.message {
            WM_MOUSEHWHEEL => {
                let delta = (msg.wParam.0 >> 16) as u16 as i16;
                self.swipe_wheel(i32::from(delta));
                true
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                self.finish_swipe();
                // Finishing may have swapped the editors, while the key, and
                // the characters TranslateMessage makes from it, still name
                // the one that had focus when it was pressed.
                self.retarget(msg);
                self.shortcut(msg)
            }
            WM_KEYUP | WM_SYSKEYUP | WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR
            | WM_UNICHAR | WM_IME_CHAR => {
                self.retarget(msg);
                false
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
            | WM_LBUTTONDBLCLK | WM_RBUTTONDBLCLK | WM_MBUTTONDBLCLK | WM_XBUTTONDBLCLK
            | WM_NCLBUTTONDOWN | WM_NCRBUTTONDOWN => {
                self.finish_swipe();
                // A click on a peek editor that was not committed would give
                // focus to a hidden window.
                self.to_peek(msg)
            }
            m if (WM_MOUSEFIRST..=WM_MOUSELAST).contains(&m) => self.to_peek(msg),
            _ => false,
        }
    }

    /// Whether `msg` is aimed at the editor that is not current.
    fn to_peek(&self, msg: &MSG) -> bool {
        self.editors[1 - self.active.get()]
            .get()
            .is_some_and(|p| p.hwnd == msg.hwnd)
    }

    fn retarget(&self, msg: &mut MSG) {
        if self.to_peek(msg) {
            msg.hwnd = self.editor().hwnd;
        }
    }
}

/// The display refresh period, 60 Hz if DWM does not say.
fn refresh_period() -> Duration {
    let mut info = DWM_TIMING_INFO {
        cbSize: size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a properly sized local.
    let ok = unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut info) }.is_ok();
    let rate = info.rateRefresh;
    if ok && rate.uiNumerator > 0 && rate.uiDenominator > 0 {
        let hz = f64::from(rate.uiNumerator) / f64::from(rate.uiDenominator);
        if (24.0..=360.0).contains(&hz) {
            return Duration::from_secs_f64(1.0 / hz);
        }
    }
    Duration::from_micros(16_667)
}

fn debug(text: String) {
    // SAFETY: the string outlives the call.
    unsafe { OutputDebugStringW(&HSTRING::from(text)) };
}
