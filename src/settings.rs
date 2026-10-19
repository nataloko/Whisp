//! A small `key=value` settings file, `settings.ini` in
//! `%LOCALAPPDATA%\whisp-win` (or `WHISP_SETTINGS_DIR`). Lines it does not
//! know, including unknown keys and comments, are kept as they are.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::notes;

const FILE: &str = "settings.ini";

/// `WHISP_SETTINGS_DIR`, or `%LOCALAPPDATA%\whisp-win`. Never Whisp's own
/// config folder.
pub fn dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("WHISP_SETTINGS_DIR").filter(|d| !d.is_empty()) {
        return Some(dir.into());
    }
    let base = std::env::var_os("LOCALAPPDATA").filter(|d| !d.is_empty())?;
    Some(Path::new(&base).join("whisp-win"))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Settings {
    lines: Vec<String>,
}

impl Settings {
    /// Reads `dir`'s settings file. A missing or unreadable file, or one
    /// that is not UTF-8, gives the defaults.
    pub fn load(dir: &Path) -> Settings {
        fs::read(dir.join(FILE))
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map_or_else(Settings::default, |text| Settings::parse(&text))
    }

    pub fn parse(text: &str) -> Settings {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        Settings {
            lines: text.lines().map(str::to_string).collect(),
        }
    }

    /// The value of the last `key=value` line for `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines
            .iter()
            .rev()
            .find_map(|line| split(line).filter(|(k, _)| *k == key).map(|(_, v)| v))
    }

    /// A non-negative whole number setting; a missing or invalid value gives
    /// `default`.
    fn number(&self, key: &str, default: u32) -> u32 {
        self.get(key)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    /// Notes older than this many days are left out of the carousel; 0 is off.
    pub fn archive_days(&self) -> u32 {
        self.number("archive_days", 0)
    }

    /// How many unpinned notes the carousel keeps; 0 is unlimited.
    pub fn max_carousel_size(&self) -> u32 {
        self.number("max_carousel_size", 10)
    }

    /// What is shown at startup.
    pub fn startup(&self) -> Startup {
        match self.get("startup") {
            Some("empty_note") => Startup::EmptyNote,
            _ => Startup::LastNote,
        }
    }

    /// The `color_scheme` setting.
    pub fn color_scheme(&self) -> ColorScheme {
        match self.get("color_scheme") {
            Some("light") => ColorScheme::Light,
            Some("dark") => ColorScheme::Dark,
            _ => ColorScheme::System,
        }
    }

    /// Whether deleting a note asks first (`confirm_delete=1`). Off by default.
    pub fn confirm_delete(&self) -> bool {
        self.get("confirm_delete") == Some("1")
    }

    /// Sets `key`, replacing its line or adding one at the end.
    pub fn set(&mut self, key: &str, value: &str) {
        let line = format!("{key}={value}");
        match self
            .lines
            .iter()
            .rposition(|l| split(l).is_some_and(|(k, _)| k == key))
        {
            Some(i) => self.lines[i] = line,
            None => self.lines.push(line),
        }
    }

    pub fn to_text(&self) -> String {
        let mut text = self.lines.join("\n");
        text.push('\n');
        text
    }

    /// Writes the file atomically, creating the folder if needed.
    pub fn save(&self, dir: &Path) -> io::Result<()> {
        notes::write_atomic(&dir.join(FILE), &self.to_text())
    }
}

/// The `startup` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Startup {
    /// The note that was shown when the app closed (the default).
    LastNote,
    /// The empty page.
    EmptyNote,
}

/// The `color_scheme` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorScheme {
    /// Follow the system (the default).
    System,
    Light,
    Dark,
}

/// The `archive_days` choices of the preferences: days and label.
pub const ARCHIVE_CHOICES: [(u32, &str); 4] = [
    (0, "Never"),
    (7, "1 Week"),
    (30, "1 Month"),
    (365, "1 Year"),
];

/// The `max_carousel_size` choices of the preferences: notes and label.
pub const CAROUSEL_CHOICES: [(u32, &str); 4] = [
    (10, "10 Notes"),
    (25, "25 Notes"),
    (50, "50 Notes"),
    (0, "Unlimited"),
];

/// `base` plus, when `value` is not one of them, `value` as an extra last
/// choice labelled by `label`, so a number set by hand is not lost.
pub fn with_stored(
    base: &[(u32, &str)],
    value: u32,
    label: impl Fn(u32) -> String,
) -> Vec<(u32, String)> {
    let mut choices: Vec<_> = base.iter().map(|&(n, l)| (n, l.to_string())).collect();
    if !base.iter().any(|&(n, _)| n == value) {
        choices.push((value, label(value)));
    }
    choices
}

/// A window's normal (restored) rectangle and whether it is maximized, as
/// the `window` setting: `left,top,right,bottom,maximized`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub maximized: bool,
}

/// Smallest saved size that is taken seriously, in pixels.
const MIN_SIDE: i32 = 100;

impl Placement {
    pub fn parse(value: &str) -> Option<Placement> {
        let parts: Vec<&str> = value.split(',').map(str::trim).collect();
        let [left, top, right, bottom, maximized] = parts[..] else {
            return None;
        };
        let p = Placement {
            left: left.parse().ok()?,
            top: top.parse().ok()?,
            right: right.parse().ok()?,
            bottom: bottom.parse().ok()?,
            maximized: match maximized {
                "1" => true,
                "0" => false,
                _ => return None,
            },
        };
        (p.right - p.left >= MIN_SIDE && p.bottom - p.top >= MIN_SIDE).then_some(p)
    }

    pub fn format(&self) -> String {
        format!(
            "{},{},{},{},{}",
            self.left,
            self.top,
            self.right,
            self.bottom,
            u8::from(self.maximized)
        )
    }
}

/// `key=value`, both trimmed; comments and other lines give `None`.
fn split(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.starts_with(['#', ';', '[']) {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    (!key.is_empty()).then_some((key, value.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_values_and_ignores_comments() {
        let s = Settings::parse("\u{feff}# comment\n paper = dotted \nother=1\n; x=2\n[section]\n");
        assert_eq!(s.get("paper"), Some("dotted"));
        assert_eq!(s.get("other"), Some("1"));
        assert_eq!(s.get("x"), None);
        assert_eq!(s.get("missing"), None);
        // The last value wins.
        assert_eq!(Settings::parse("a=1\na=2").get("a"), Some("2"));
    }

    #[test]
    fn set_keeps_unknown_lines() {
        let mut s = Settings::parse("# mine\nzoom=3\npaper=grid\n");
        s.set("paper", "dotted");
        s.set("new", "x");
        assert_eq!(s.to_text(), "# mine\nzoom=3\npaper=dotted\nnew=x\n");
        let mut empty = Settings::default();
        empty.set("paper", "grid");
        assert_eq!(empty.to_text(), "paper=grid\n");
    }

    #[test]
    fn carousel_and_startup_settings() {
        let s = Settings::default();
        assert_eq!((s.archive_days(), s.max_carousel_size()), (0, 10));
        assert_eq!(s.startup(), Startup::LastNote);
        let s = Settings::parse("archive_days=30\nmax_carousel_size=0\nstartup=empty_note");
        assert_eq!((s.archive_days(), s.max_carousel_size()), (30, 0));
        assert_eq!(s.startup(), Startup::EmptyNote);
        let s = Settings::parse("archive_days=-1\nmax_carousel_size=x\nstartup=other");
        assert_eq!((s.archive_days(), s.max_carousel_size()), (0, 10));
        assert_eq!(s.startup(), Startup::LastNote);
    }

    #[test]
    fn appearance_and_delete_settings() {
        let s = Settings::default();
        assert_eq!(s.color_scheme(), ColorScheme::System);
        assert!(!s.confirm_delete());
        let s = Settings::parse("color_scheme=dark\nconfirm_delete=1");
        assert_eq!(s.color_scheme(), ColorScheme::Dark);
        assert!(s.confirm_delete());
        let s = Settings::parse("color_scheme=light\nconfirm_delete=0");
        assert_eq!(s.color_scheme(), ColorScheme::Light);
        assert!(!s.confirm_delete());
        assert_eq!(
            Settings::parse("color_scheme=blue").color_scheme(),
            ColorScheme::System
        );
    }

    #[test]
    fn stored_numbers_become_extra_choices() {
        let same = with_stored(&ARCHIVE_CHOICES, 30, |n| format!("{n} days"));
        assert_eq!(same.len(), 4);
        let extra = with_stored(&ARCHIVE_CHOICES, 90, |n| format!("{n} days"));
        assert_eq!(extra.len(), 5);
        assert_eq!(extra[4], (90, "90 days".to_string()));
        let zero = with_stored(&CAROUSEL_CHOICES, 0, |n| format!("{n} Notes"));
        assert_eq!(zero.len(), 4);
    }

    #[test]
    fn window_placement() {
        let p = Placement::parse("10, -20,430,580,1").unwrap();
        assert_eq!(
            p,
            Placement {
                left: 10,
                top: -20,
                right: 430,
                bottom: 580,
                maximized: true
            }
        );
        assert_eq!(Placement::parse(&p.format()), Some(p));
        assert_eq!(p.format(), "10,-20,430,580,1");
        for bad in [
            "",
            "1,2,3,4",
            "0,0,500,500,2",
            "0,0,50,500,0",
            "a,0,500,500,0",
            "0,0,500,500,0,9",
        ] {
            assert_eq!(Placement::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn missing_or_bad_files_give_defaults() {
        let dir = std::env::temp_dir().join(format!("whisp-settings-{}", notes::new_name()));
        assert_eq!(Settings::load(&dir), Settings::default());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE), b"paper=\xff\xfe").unwrap();
        assert_eq!(Settings::load(&dir), Settings::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir()
            .join(format!("whisp-settings-{}", notes::new_name()))
            .join("sub");
        let mut s = Settings::parse("keep=me");
        s.set("paper", "large_grid");
        s.save(&dir).unwrap();
        let loaded = Settings::load(&dir);
        assert_eq!(loaded.get("paper"), Some("large_grid"));
        assert_eq!(loaded.get("keep"), Some("me"));
        fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }
}
