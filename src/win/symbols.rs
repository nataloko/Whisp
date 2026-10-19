//! Whisp's hide symbols mode (Whisp calls it WYSIWYG), Ctrl+E: Markdown
//! markers such as `**`, heading `#`s and a link's `](url)` are hidden
//! text, except on the caret's line. The text keeps them, so saving,
//! searching and every text read see the full Markdown; copying and
//! cutting are done here so the clipboard gets it too.
//!
//! As in Whisp, settings.ini `hide_symbols_scope` says whether the state is
//! shared by all notes (`global`, the default, kept in `hide_symbols`) or
//! kept per note (`per_note`, the `wysiwyg` flag in `metadata.json`).

use std::path::Path;

use super::App;
use crate::{metadata, notes};

impl App {
    /// The state is kept per note rather than shared by all notes.
    fn symbols_per_note(&self) -> bool {
        self.settings.borrow().get("hide_symbols_scope") == Some("per_note")
    }

    /// Whether note `index`, or the empty page after the last note, shows
    /// with symbols hidden.
    pub(super) fn hides_symbols(&self, index: usize) -> bool {
        if !self.symbols_per_note() {
            return self.settings.borrow().get("hide_symbols") == Some("1");
        }
        match self.notes.borrow().get(index) {
            Some(path) => file_name(path).is_some_and(|n| self.meta.borrow().flag(n, "wysiwyg")),
            // The empty page has no file to keep a flag for yet.
            None => self.hide_new.get(),
        }
    }

    /// Ctrl+E: hides or shows the symbols, and remembers that for all notes
    /// or for this one. Only the lines that look different are restyled, so
    /// the undo history is kept.
    pub(super) fn toggle_symbols(&self) {
        let on = !self.editor().hides();
        if self.symbols_per_note() {
            let path = self.notes.borrow().get(self.current.get()).cloned();
            match path {
                Some(path) => {
                    let Some(name) = file_name(&path) else {
                        return;
                    };
                    match metadata::update_flag(&self.dir, name, "wysiwyg", on) {
                        Ok(meta) => *self.meta.borrow_mut() = meta,
                        Err(_) => {
                            self.show_status("Could not save the setting");
                            return;
                        }
                    }
                }
                None => self.hide_new.set(on),
            }
        } else {
            self.save_setting("hide_symbols", if on { "1" } else { "0" });
        }
        // The peek editor may hold a neighbour styled the old way.
        self.peek.set(None);
        self.busy.set(true);
        self.editor().set_hide(&self.theme.get(), on);
        self.busy.set(false);
        self.show_status(if on {
            "Symbols hidden"
        } else {
            "Symbols shown"
        });
    }

    /// The empty page just got its file at `path`. In the per note scope,
    /// symbols hidden there become the new note's `wysiwyg` flag.
    pub(super) fn new_note_saved(&self, path: &Path) {
        if !self.symbols_per_note() {
            return;
        }
        self.hide_new.set(false);
        // A note without the flag shows its symbols.
        if !self.editor().hides() {
            return;
        }
        let Some(name) = file_name(path) else {
            return;
        };
        match metadata::update_flag(&self.dir, name, "wysiwyg", true) {
            Ok(meta) => *self.meta.borrow_mut() = meta,
            Err(_) => self.show_status("Could not save the setting"),
        }
    }

    /// Ctrl+C, Ctrl+X and Ctrl+Insert while symbols are hidden. RichEdit
    /// may leave hidden text out of what it copies, so the selection's full
    /// Markdown goes on the clipboard here, with Windows line breaks. Cut
    /// then deletes the selection as one undo step.
    pub(super) fn copy_selection(&self, cut: bool) {
        let editor = self.editor();
        let (start, end) = editor.selection();
        if start == end {
            return;
        }
        // The whole text, read with hidden text included, then the part.
        let units = editor.text_utf16();
        let selected = &units[start.min(units.len())..end.min(units.len())];
        let text = notes::to_lf(&String::from_utf16_lossy(selected)).replace('\n', "\r\n");
        if self.set_clipboard(&text) && cut {
            editor.replace_range(start, end, "");
        }
    }
}

fn file_name(path: &Path) -> Option<&str> {
    path.file_name().and_then(|n| n.to_str())
}
