//! Notes on disk: plain UTF-8 `.md` files in one folder, `\n` line endings.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Files larger than this are never blank in practice, so they are not read
/// when listing; this keeps startup cheap with many notes.
const BLANK_CHECK_LIMIT: u64 = 4096;

/// The notes folder: `setting` (the `notes_dir` setting) when it is set,
/// else the default folder.
pub fn notes_dir(setting: Option<&str>) -> Option<PathBuf> {
    match setting.filter(|d| !d.is_empty()) {
        Some(dir) => Some(dir.into()),
        None => default_dir(),
    }
}

/// The default notes folder: `WHISP_NOTES_DIR`, or
/// `%LOCALAPPDATA%\whisp\notes` like Whisp.
pub fn default_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("WHISP_NOTES_DIR").filter(|d| !d.is_empty()) {
        return Some(dir.into());
    }
    let base = std::env::var_os("LOCALAPPDATA").filter(|d| !d.is_empty())?;
    Some(Path::new(&base).join("whisp").join("notes"))
}

pub fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// Whether `a` and `b` are the same folder, also when spelled differently.
/// Folders that do not exist are compared as written.
pub fn same_dir(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

pub fn is_note(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// The notes in `dir` with their modified times, oldest first. Blank notes
/// are deleted, as Whisp does; subfolders and other files are ignored.
pub fn list(dir: &Path) -> io::Result<Vec<(PathBuf, SystemTime)>> {
    let mut notes = Vec::new();
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() || !is_note(&path) {
            continue;
        }
        if meta.len() <= BLANK_CHECK_LIMIT && fs::read(&path).is_ok_and(|b| is_blank(&decode(&b))) {
            let _ = fs::remove_file(&path);
            continue;
        }
        notes.push((path, meta.modified().unwrap_or(UNIX_EPOCH)));
    }
    notes.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
    Ok(notes)
}

/// File contents as editor text: BOM stripped, `\n` line endings. Invalid
/// UTF-8 is decoded lossily; that is safe because notes are only written
/// back after the user edits them.
pub fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    to_lf(&String::from_utf8_lossy(bytes))
}

pub fn read(path: &Path) -> io::Result<String> {
    fs::read(path).map(|b| decode(&b))
}

/// Converts `\r\n`, `\r` and `\v` (RichEdit's soft line break) to `\n`.
pub fn to_lf(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                chars.next_if_eq(&'\n');
                out.push('\n');
            }
            '\u{b}' => out.push('\n'),
            _ => out.push(c),
        }
    }
    out
}

/// Writes `text` to a temporary file next to `path`, then renames it over
/// `path`, so a failed write never leaves a half written note.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(dir)?;
    let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(".tmp");
    let tmp = dir.join(tmp_name);
    let result = File::create(&tmp)
        .and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// A random 32 hex digit file name, like Whisp's `uuid4().hex`.
pub fn new_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut name = String::with_capacity(35);
    for i in 0..2u64 {
        // RandomState is seeded from the OS random source.
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(nanos);
        h.write_u32(std::process::id());
        let _ = write!(name, "{:016x}", h.finish());
    }
    name.push_str(".md");
    name
}

/// A path for a new note in `dir` that does not exist yet.
pub fn new_path(dir: &Path) -> PathBuf {
    loop {
        let path = dir.join(new_name());
        if !path.exists() {
            return path;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whisp-test-{tag}-{}", new_name()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path, text: &str, secs: u64) {
        fs::write(path, text).unwrap();
        let time = UNIX_EPOCH + Duration::from_secs(1_700_000_000 + secs);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    #[test]
    fn folder_setting_wins_over_the_default() {
        assert_eq!(
            notes_dir(Some("D:\\Notes")),
            Some(PathBuf::from("D:\\Notes"))
        );
        assert_eq!(notes_dir(Some("")), default_dir());
        assert_eq!(notes_dir(None), default_dir());
    }

    #[test]
    fn same_dir_sees_through_spelling() {
        let dir = temp_dir("same");
        fs::create_dir(dir.join("a")).unwrap();
        assert!(same_dir(
            &dir.join("a"),
            &dir.join("a").join("..").join("a")
        ));
        assert!(!same_dir(&dir.join("a"), &dir));
        assert!(same_dir(&dir.join("gone"), &dir.join("gone")));
        assert!(!same_dir(&dir.join("gone"), &dir.join("a")));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn decode_handles_bom_line_endings_and_bad_utf8() {
        assert_eq!(decode(b"\xEF\xBB\xBFa\r\nb\rc\nd"), "a\nb\nc\nd");
        assert_eq!(decode(b"caf\xC3\xA9"), "café");
        assert_eq!(decode(b"a\xFFb"), "a\u{FFFD}b");
    }

    #[test]
    fn to_lf_converts_richedit_breaks() {
        assert_eq!(to_lf("a\rb\u{b}c\r\n\rd"), "a\nb\nc\n\nd");
    }

    #[test]
    fn names_are_32_hex_digits_and_unique() {
        let a = new_name();
        assert_eq!(a.len(), 35);
        assert!(a.ends_with(".md"));
        assert!(a[..32].chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, new_name());
    }

    #[test]
    fn list_orders_by_mtime_and_deletes_blank_notes() {
        let dir = temp_dir("list");
        touch(&dir.join("new.md"), "newest", 30);
        touch(&dir.join("old.md"), "oldest", 10);
        touch(&dir.join("mid.MD"), "middle", 20);
        touch(&dir.join("blank.md"), " \n\t\n", 40);
        touch(&dir.join("empty.md"), "", 50);
        touch(&dir.join("bom.md"), "\u{feff}\n", 60);
        touch(&dir.join("other.txt"), "x", 70);
        fs::create_dir(dir.join(".trash")).unwrap();
        touch(&dir.join(".trash").join("gone.md"), "x", 80);

        let names: Vec<_> = list(&dir)
            .unwrap()
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["old.md", "mid.MD", "new.md"]);
        assert!(!dir.join("blank.md").exists());
        assert!(!dir.join("empty.md").exists());
        assert!(!dir.join("bom.md").exists());
        assert!(dir.join("other.txt").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_temp_file() {
        let dir = temp_dir("write");
        let path = dir.join("note.md");
        write_atomic(&path, "first").unwrap();
        write_atomic(&path, "second é\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), "second é\n".as_bytes());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn write_atomic_creates_missing_folder() {
        let dir = temp_dir("mkdir");
        let path = dir.join("sub").join("note.md");
        write_atomic(&path, "x").unwrap();
        assert_eq!(read(&path).unwrap(), "x");
        fs::remove_dir_all(dir).unwrap();
    }
}
