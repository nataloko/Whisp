//! OCR paste: the text of a clipboard image, read with Windows.Media.Ocr on
//! a worker thread. Nothing here is created before the first image paste.

use std::time::Instant;

use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Win32::Foundation::{HGLOBAL, HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
};
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::HSTRING;

use crate::dib;
use crate::ocr_text::{self, Word};

const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
/// Posted by the worker with a `Box<Done>` in lparam.
pub(super) const WM_OCR_DONE: u32 = WM_APP + 3;

pub(super) enum Outcome {
    Text(String),
    NoText,
    NoEngine,
    Failed,
}

/// What the worker hands back: the result and the note it was meant for.
pub(super) struct Done {
    pub generation: u64,
    pub outcome: Outcome,
}

pub(super) fn clipboard_has_text() -> bool {
    // SAFETY: plain call.
    unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() }
}

pub(super) fn clipboard_has_image() -> bool {
    // SAFETY: plain call. Windows synthesises CF_DIB from CF_DIBV5 and
    // CF_BITMAP, so this covers all three.
    unsafe { IsClipboardFormatAvailable(CF_DIB).is_ok() }
}

/// A copy of the clipboard's CF_DIB.
pub(super) fn clipboard_dib(owner: HWND) -> Option<Vec<u8>> {
    // SAFETY: the clipboard memory is only read while locked, within its size.
    unsafe {
        OpenClipboard(Some(owner)).ok()?;
        let bytes = GetClipboardData(CF_DIB).ok().and_then(|handle| {
            let mem = HGLOBAL(handle.0);
            let ptr = GlobalLock(mem) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(ptr, GlobalSize(mem)).to_vec();
            let _ = GlobalUnlock(mem);
            Some(bytes)
        });
        let _ = CloseClipboard();
        bytes
    }
}

/// Reads `dib` on a new thread and posts WM_OCR_DONE to `hwnd` when done.
pub(super) fn start(hwnd: HWND, dib: Vec<u8>, generation: u64) {
    // HWND is a pointer, so it travels to the thread as a number.
    let target = hwnd.0 as isize;
    let spawned = std::thread::Builder::new()
        .name("ocr".into())
        .spawn(move || {
            // SAFETY: this thread's own WinRT apartment, left before it ends.
            // windows-rs would otherwise join the process wide MTA implicitly on
            // first activation (CoIncrementMTAUsage); this makes it explicit.
            let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
            // All WinRT objects are dropped inside `read` before RoUninitialize.
            let outcome = read(&dib).unwrap_or(Outcome::Failed);
            if initialized {
                // SAFETY: pairs with the successful RoInitialize above.
                unsafe { RoUninitialize() };
            }
            let done = Box::into_raw(Box::new(Done {
                generation,
                outcome,
            }));
            // SAFETY: on success the window procedure takes ownership of `done`.
            let posted = unsafe {
                PostMessageW(
                    Some(HWND(target as *mut _)),
                    WM_OCR_DONE,
                    WPARAM(0),
                    LPARAM(done as isize),
                )
            };
            if posted.is_err() {
                // SAFETY: not posted, so still ours.
                drop(unsafe { Box::from_raw(done) });
            }
        });
    if spawned.is_err() {
        let done = Box::into_raw(Box::new(Done {
            generation,
            outcome: Outcome::Failed,
        }));
        // SAFETY: as above.
        if unsafe { PostMessageW(Some(hwnd), WM_OCR_DONE, WPARAM(0), LPARAM(done as isize)) }
            .is_err()
        {
            drop(unsafe { Box::from_raw(done) });
        }
    }
}

/// Takes back the result posted with WM_OCR_DONE.
///
/// # Safety
/// `lparam` must come from a WM_OCR_DONE message, and is used only once.
pub(super) unsafe fn take(lparam: LPARAM) -> Box<Done> {
    unsafe { Box::from_raw(lparam.0 as *mut Done) }
}

fn read(dib: &[u8]) -> windows::core::Result<Outcome> {
    let started = Instant::now();
    let Ok(image) = dib::parse(dib) else {
        debug("whisp: ocr could not parse the clipboard image\n".into());
        return Ok(Outcome::Failed);
    };
    let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() else {
        return Ok(Outcome::NoEngine);
    };
    let max = OcrEngine::MaxImageDimension()? as usize;
    let (width, height) = dib::ocr_size(image.width, image.height, max);
    let scaled = dib::resize(&image, width, height);
    let buffer = CryptographicBuffer::CreateFromByteArray(&scaled.bgra)?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        width as i32,
        height as i32,
    )?;
    let result = engine.RecognizeAsync(&bitmap)?.join()?;

    let mut lines = Vec::new();
    for line in &result.Lines()? {
        let mut words = Vec::new();
        for word in &line.Words()? {
            let rect = word.BoundingRect()?;
            words.push(Word {
                text: word.Text()?.to_string_lossy(),
                left: f64::from(rect.X),
                width: f64::from(rect.Width),
                top: f64::from(rect.Y),
                height: f64::from(rect.Height),
            });
        }
        lines.push(words);
    }
    let text = ocr_text::reconstruct(&lines);
    debug(format!(
        "whisp: ocr {}x{} px (scaled {width}x{height}), {} lines, {} chars, {} ms\n",
        image.width,
        image.height,
        lines.len(),
        text.as_ref().map_or(0, |t| t.chars().count()),
        started.elapsed().as_millis()
    ));
    Ok(text.map_or(Outcome::NoText, Outcome::Text))
}

fn debug(text: String) {
    // SAFETY: the string outlives the call.
    unsafe { OutputDebugStringW(&HSTRING::from(text)) };
}
