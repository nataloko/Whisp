//! The keys listed in the keyboard shortcuts window (Ctrl+/), in sections.
//! Written from what the key handlers in `win` do, not from Whisp's list.

/// One line of the list: what it does, and the keys or gestures that do it.
pub struct Entry {
    pub action: &'static str,
    pub keys: &'static [&'static str],
}

pub struct Section {
    pub title: &'static str,
    pub entries: &'static [Entry],
}

const fn entry(action: &'static str, keys: &'static [&'static str]) -> Entry {
    Entry { action, keys }
}

pub const SECTIONS: [Section; 4] = [
    Section {
        title: "Application",
        entries: &[
            entry("Preferences", &["Ctrl+,"]),
            entry("Keyboard shortcuts", &["Ctrl+/"]),
            entry("Search all notes", &["Ctrl+F"]),
            entry("Hide or show Markdown symbols", &["Ctrl+E"]),
            entry("Next paper", &["Alt+P"]),
            entry("Clear search highlights", &["Esc"]),
            entry("Quit", &["Ctrl+Q"]),
        ],
    },
    Section {
        title: "Notes",
        entries: &[
            entry("Delete note", &["Ctrl+D", "Shift+Delete"]),
            entry("Restore deleted note", &["Ctrl+Shift+T"]),
            entry("Pin or unpin note", &["Ctrl+Shift+P"]),
            entry("Move note to the newest place", &["Ctrl+Shift+B"]),
            entry("Copy whole note", &["Ctrl+Shift+C"]),
            entry("Export note as Markdown", &["Ctrl+Shift+S"]),
        ],
    },
    Section {
        title: "Navigation",
        entries: &[
            entry("Next (newer) note", &["Ctrl+]"]),
            entry("Previous (older) note", &["Ctrl+["]),
            entry("Empty note at the end", &["Ctrl+N", "Alt+L"]),
            entry("Oldest note", &["Alt+F"]),
            entry("Change note", &["Two finger swipe"]),
            entry("Change note at the top or bottom", &["Mouse wheel"]),
        ],
    },
    Section {
        title: "Editing",
        entries: &[
            entry("Bold", &["Ctrl+B"]),
            entry("Italic", &["Ctrl+I"]),
            entry("Underline", &["Ctrl+U"]),
            entry("Strikethrough", &["Ctrl+Shift+X"]),
            entry("Tick or untick a checkbox", &["Ctrl+S"]),
            entry("Move line up", &["Alt+Up"]),
            entry("Move line down", &["Alt+Down"]),
            entry("Indent", &["Tab"]),
            entry("Outdent", &["Shift+Tab", "Ctrl+Tab"]),
            entry("Delete a word, or an empty list item", &["Ctrl+Backspace"]),
            entry("Paste text, or read text from an image", &["Ctrl+V"]),
            entry("Paste without Markdown", &["Ctrl+Shift+V"]),
            entry("Shorten a link", &["Ctrl+L"]),
            entry("Open a link", &["Ctrl+click"]),
            entry("Undo", &["Ctrl+Z"]),
            entry("Redo", &["Ctrl+Y"]),
            entry("Insert a command", &["::"]),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_key_is_listed_twice() {
        let mut seen = HashSet::new();
        for entry in SECTIONS.iter().flat_map(|s| s.entries) {
            assert!(!entry.keys.is_empty(), "{}", entry.action);
            for key in entry.keys {
                assert!(seen.insert(key.to_lowercase()), "{key} is listed twice");
            }
        }
    }

    #[test]
    fn no_action_is_listed_twice_and_none_is_empty() {
        let mut seen = HashSet::new();
        for section in &SECTIONS {
            assert!(!section.entries.is_empty(), "{}", section.title);
            for entry in section.entries {
                assert!(!entry.action.is_empty());
                assert!(
                    seen.insert(entry.action),
                    "{} is listed twice",
                    entry.action
                );
            }
        }
    }
}
