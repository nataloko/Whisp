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
