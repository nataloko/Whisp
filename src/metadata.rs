//! Whisp's `metadata.json` in the notes folder: a flat object keyed by note
//! file name, with `pinned` and `wysiwyg` flags per note. Anything else in
//! the file is kept as it is, including entries for notes that are gone.

use std::fs;
use std::io;
use std::path::Path;

use crate::json::{self, Value};
use crate::notes;

const FILE: &str = "metadata.json";

#[derive(Debug)]
pub enum Error {
    /// The file exists but cannot be read.
    Io,
    /// The file is not JSON, or its root is not an object.
    Invalid,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    entries: Vec<(String, Value)>,
}

impl Metadata {
    /// Reads the folder's `metadata.json`. A missing file is empty metadata.
    pub fn load(dir: &Path) -> Result<Metadata, Error> {
        let bytes = match fs::read(dir.join(FILE)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Metadata::default()),
            Err(_) => return Err(Error::Io),
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| Error::Invalid)?;
        match json::parse(text) {
            Some(Value::Object(entries)) => Ok(Metadata { entries }),
            _ => Err(Error::Invalid),
        }
    }

    /// True only when note `name`'s `key` is JSON `true`.
    pub fn flag(&self, name: &str, key: &str) -> bool {
        self.entries
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, entry)| entry.get(key))
            == Some(&Value::Bool(true))
    }

    /// Sets note `name`'s `key`. A note without an entry gets an empty one
    /// at the end first, as in Whisp; the key is replaced in place or added.
    pub fn set_flag(&mut self, name: &str, key: &str, value: bool) {
        let at = match self.entries.iter().position(|(n, _)| n == name) {
            Some(at) => at,
            None => {
                self.entries
                    .push((name.to_string(), Value::Object(Vec::new())));
                self.entries.len() - 1
            }
        };
        let value = Value::Bool(value);
        match &mut self.entries[at].1 {
            Value::Object(members) => match members.iter_mut().find(|(k, _)| k == key) {
                Some(member) => member.1 = value,
                None => members.push((key.to_string(), value)),
            },
            // A damaged entry is replaced, as Whisp's dict update would fail
            // on it; it only happens with a hand edited file.
            other => *other = Value::Object(vec![(key.to_string(), value)]),
        }
    }

    /// The file text, as Python's `json.dumps` writes it.
    pub fn to_text(&self) -> String {
        Value::Object(self.entries.clone()).dump()
    }
}

/// Changes one flag safely: the file is read again first, so changes Whisp
/// made since startup are kept, and a file that cannot be parsed is never
/// overwritten. Returns the metadata as written.
pub fn update_flag(dir: &Path, name: &str, key: &str, value: bool) -> Result<Metadata, Error> {
    let mut meta = Metadata::load(dir)?;
    meta.set_flag(name, key, value);
    notes::write_atomic(&dir.join(FILE), &meta.to_text()).map_err(|_| Error::Io)?;
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whisp-meta-{}", notes::new_name()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = temp_dir();
        let meta = Metadata::load(&dir).unwrap();
        assert_eq!(meta, Metadata::default());
        assert!(!meta.flag("a.md", "pinned"));
        assert_eq!(meta.to_text(), "{}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bad_files_are_errors() {
        let dir = temp_dir();
        for bad in ["", "{", "[]", "\"x\"", "{} x"] {
            fs::write(dir.join(FILE), bad).unwrap();
            assert!(Metadata::load(&dir).is_err(), "{bad:?}");
        }
        fs::write(dir.join(FILE), b"{\"a\": \xff}").unwrap();
        assert!(Metadata::load(&dir).is_err());
        // A directory where the file should be cannot be read.
        fs::remove_file(dir.join(FILE)).unwrap();
        fs::create_dir(dir.join(FILE)).unwrap();
        assert!(matches!(Metadata::load(&dir), Err(Error::Io)));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn flags_read_only_true() {
        let dir = temp_dir();
        fs::write(
            dir.join(FILE),
            r#"{"a.md": {"pinned": true, "wysiwyg": false}, "b.md": {"pinned": "true"}, "c.md": 5}"#,
        )
        .unwrap();
        let meta = Metadata::load(&dir).unwrap();
        assert!(meta.flag("a.md", "pinned"));
        assert!(!meta.flag("a.md", "wysiwyg"));
        assert!(!meta.flag("a.md", "other"));
        assert!(!meta.flag("b.md", "pinned"));
        assert!(!meta.flag("c.md", "pinned"));
        assert!(!meta.flag("missing.md", "pinned"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn set_flag_keeps_order_and_unknown_keys() {
        let mut meta = Metadata {
            entries: match json::parse(
                r#"{"a.md": {"x": 1, "pinned": false}, "gone.md": {"pinned": true}}"#,
            ) {
                Some(Value::Object(e)) => e,
                _ => unreachable!(),
            },
        };
        meta.set_flag("a.md", "pinned", true);
        meta.set_flag("a.md", "wysiwyg", false);
        meta.set_flag("new.md", "pinned", true);
        assert_eq!(
            meta.to_text(),
            r#"{"a.md": {"x": 1, "pinned": true, "wysiwyg": false}, "gone.md": {"pinned": true}, "new.md": {"pinned": true}}"#
        );
    }

    #[test]
    fn update_flag_rereads_the_file() {
        let dir = temp_dir();
        let first = update_flag(&dir, "a.md", "pinned", true).unwrap();
        assert!(first.flag("a.md", "pinned"));
        // Whisp adds an entry meanwhile.
        fs::write(
            dir.join(FILE),
            r#"{"a.md": {"pinned": true}, "w.md": {"pinned": true, "wysiwyg": true}}"#,
        )
        .unwrap();
        let meta = update_flag(&dir, "a.md", "pinned", false).unwrap();
        assert!(meta.flag("w.md", "wysiwyg"));
        assert!(!meta.flag("a.md", "pinned"));
        assert_eq!(
            fs::read_to_string(dir.join(FILE)).unwrap(),
            r#"{"a.md": {"pinned": false}, "w.md": {"pinned": true, "wysiwyg": true}}"#
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_file_is_left_alone() {
        let dir = temp_dir();
        fs::write(dir.join(FILE), "{\"a.md\": {oops").unwrap();
        assert!(update_flag(&dir, "a.md", "pinned", true).is_err());
        assert_eq!(
            fs::read_to_string(dir.join(FILE)).unwrap(),
            "{\"a.md\": {oops"
        );
        fs::write(dir.join(FILE), "[1]").unwrap();
        assert!(update_flag(&dir, "a.md", "pinned", true).is_err());
        assert_eq!(fs::read_to_string(dir.join(FILE)).unwrap(), "[1]");
        fs::remove_dir_all(dir).unwrap();
    }
}
