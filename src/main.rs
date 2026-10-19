// Whisp for Windows: a Markdown scratchpad on the Win32 API and RichEdit.
#![windows_subsystem = "windows"]
// Only the Windows build uses the platform independent modules; tests cover
// them everywhere.
#![cfg_attr(not(windows), allow(dead_code))]

mod carousel;
mod dib;
mod expand;
mod gesture;
mod json;
mod lines;
mod lists;
mod markdown;
mod markup;
mod metadata;
mod migrate;
mod notes;
mod ocr_text;
mod paper;
mod rtf;
mod search;
mod settings;
mod shortcuts;
mod trash;
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
