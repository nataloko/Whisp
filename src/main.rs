// Whisp for Windows: a Markdown scratchpad on the Win32 API and RichEdit.
#![windows_subsystem = "windows"]
// Only the Windows build uses the platform independent modules; tests cover
// them everywhere.
#![cfg_attr(not(windows), allow(dead_code))]

mod dib;
mod gesture;
mod lines;
mod markdown;
mod notes;
mod ocr_text;
mod rtf;
mod search;
#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("whisp-win only runs on Windows; build it with `cargo win`.");
}
