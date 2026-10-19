//! Deleted notes go into the `.trash` folder inside the notes folder, as in
//! Whisp, and are never deleted for good. Moves never overwrite a file: a
//! name already taken gets a fresh one instead.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::notes;

/// Whisp's trash folder name, inside the notes folder.
pub const TRASH: &str = ".trash";
/// How many deletions Ctrl+Shift+T can undo, newest first.
const REMEMBERED: usize = 20;
/// Separates names in the settings value; it cannot appear in a file name.
const SEPARATOR: char = '/';

pub fn trash_dir(notes_dir: &Path) -> PathBuf {
    notes_dir.join(TRASH)
}

/// Moves `from` into `dir` as `name`, or under a fresh name if that is
/// taken. Returns the new path.
fn move_into(from: &Path, dir: &Path, name: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let mut target = dir.join(name);
    loop {
        if target.exists() {
            target = notes::new_path(dir);
            continue;
        }
        // A hard link fails if the target exists, so nothing is ever
        // overwritten, even if another program creates it meanwhile.
        match fs::hard_link(from, &target) {
            Ok(()) => {
                if let Err(e) = fs::remove_file(from) {
                    let _ = fs::remove_file(&target);
                    return Err(e);
                }
                return Ok(target);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            // No hard links on this file system (FAT, say): rename, which is
            // safe as the target was just checked to be free.
            Err(_) => {
                fs::rename(from, &target)?;
                return Ok(target);
            }
        }
    }
}

/// Moves the note at `note` into the trash. Returns its name there.
pub fn delete(notes_dir: &Path, note: &Path) -> io::Result<String> {
    let name = note.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let moved = move_into(note, &trash_dir(notes_dir), &name.to_string_lossy())?;
    Ok(moved
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned())
}

/// Moves `name` from the trash back into the notes folder. Returns the
/// restored path.
pub fn restore(notes_dir: &Path, name: &str) -> io::Result<PathBuf> {
    move_into(&trash_dir(notes_dir).join(name), notes_dir, name)
}

/// Whether `name` is still in the trash.
pub fn is_in_trash(notes_dir: &Path, name: &str) -> bool {
    trash_dir(notes_dir).join(name).is_file()
}

/// The deleted names from the settings value, oldest first.
pub fn parse_deleted(value: &str) -> Vec<String> {
    value
        .split(SEPARATOR)
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect()
}

/// The settings value for `names`, keeping the newest.
pub fn format_deleted(names: &[String]) -> String {
    let start = names.len().saturating_sub(REMEMBERED);
    names[start..].join(&SEPARATOR.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whisp-trash-{}", notes::new_name()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn delete_and_restore_round_trip() {
        let dir = temp_dir();
        let note = dir.join("a.md");
        fs::write(&note, "hello").unwrap();
        let name = delete(&dir, &note).unwrap();
        assert_eq!(name, "a.md");
        assert!(!note.exists());
        assert!(is_in_trash(&dir, "a.md"));
        assert_eq!(
            fs::read_to_string(dir.join(TRASH).join("a.md")).unwrap(),
            "hello"
        );
        let back = restore(&dir, &name).unwrap();
        assert_eq!(back, note);
        assert_eq!(fs::read_to_string(&note).unwrap(), "hello");
        assert!(!is_in_trash(&dir, "a.md"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn name_clashes_never_overwrite() {
        let dir = temp_dir();
        fs::create_dir_all(dir.join(TRASH)).unwrap();
        fs::write(dir.join(TRASH).join("a.md"), "old trash").unwrap();
        fs::write(dir.join("a.md"), "new").unwrap();
        // Into the trash: the earlier "a.md" there stays.
        let name = delete(&dir, &dir.join("a.md")).unwrap();
        assert_ne!(name, "a.md");
        assert!(name.ends_with(".md"));
        assert_eq!(
            fs::read_to_string(dir.join(TRASH).join("a.md")).unwrap(),
            "old trash"
        );
        assert_eq!(
            fs::read_to_string(dir.join(TRASH).join(&name)).unwrap(),
            "new"
        );
        // Back out: "a.md" exists again in the notes folder.
        fs::write(dir.join("a.md"), "taken").unwrap();
        let back = restore(&dir, "a.md").unwrap();
        assert_ne!(back, dir.join("a.md"));
        assert_eq!(fs::read_to_string(&back).unwrap(), "old trash");
        assert_eq!(fs::read_to_string(dir.join("a.md")).unwrap(), "taken");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restore_of_a_missing_note_fails_cleanly() {
        let dir = temp_dir();
        assert!(restore(&dir, "gone.md").is_err());
        assert!(!is_in_trash(&dir, "gone.md"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deleted_list_in_settings() {
        assert_eq!(parse_deleted(""), Vec::<String>::new());
        assert_eq!(parse_deleted("a.md/My, note.md/"), ["a.md", "My, note.md"]);
        let many: Vec<String> = (0..30).map(|i| format!("{i}.md")).collect();
        let value = format_deleted(&many);
        let kept = parse_deleted(&value);
        assert_eq!(kept.len(), 20);
        assert_eq!(kept.first().unwrap(), "10.md");
        assert_eq!(kept.last().unwrap(), "29.md");
    }
}
