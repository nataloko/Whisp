//! Moving the notes to another folder, as Whisp's `migrate_to_dir` does
//! when the notes folder is changed in the preferences. Only the top level
//! of the old folder is looked at: the notes, the trash and the metadata
//! move; everything else stays, and the old folder itself is kept. Nothing
//! in the new folder is ever overwritten.

use std::fs::{self, File};
use std::io;
use std::path::Path;

use crate::metadata::{self, Metadata};
use crate::notes;
use crate::trash::TRASH;

/// What a move did, by name in the old folder, in the order the folder
/// listed them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub moved: Vec<String>,
    /// Notes, the trash or the metadata that are still in the old folder.
    pub stayed: Vec<String>,
}

impl Report {
    /// Whether everything that should have moved did.
    pub fn complete(&self) -> bool {
        self.stayed.is_empty()
    }
}

/// Moves the notes from `old` to `new`, creating `new` (and its parents):
/// - every `.md` file, unless the new folder has a file of that name, in
///   which case it stays where it is;
/// - `.trash`, unless the new folder has one;
/// - `metadata.json`, or, when the new folder has one too, both merged
///   with the old file's entries winning, written to the new folder, and
///   the old file deleted. If either cannot be parsed, both stay as they
///   are.
///
/// A step that fails leaves that item where it is and the move goes on.
/// Fails only when `new` cannot be created or `old` cannot be listed. A
/// missing `old` has nothing to move; the same folder moves nothing.
pub fn move_notes(old: &Path, new: &Path) -> io::Result<Report> {
    fs::create_dir_all(new)?;
    let mut report = Report::default();
    if notes::same_dir(old, new) {
        return Ok(report);
    }
    let entries = match fs::read_dir(old) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(report),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let from = entry.path();
        let to = new.join(&name);
        let moved = if name == TRASH && kind.is_dir() {
            !exists(&to) && move_dir(&from, &to).is_ok()
        } else if name == metadata::FILE && kind.is_file() {
            move_metadata(old, new)
        } else if kind.is_file() && notes::is_note(&from) {
            !exists(&to) && move_file(&from, &to).is_ok()
        } else {
            continue;
        };
        let name = name.to_string_lossy().into_owned();
        if moved {
            report.moved.push(name);
        } else {
            report.stayed.push(name);
        }
    }
    Ok(report)
}

/// Whether something is at `path`. When that cannot be told, it is taken
/// to be there, so nothing is moved onto it.
fn exists(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(e) => e.kind() != io::ErrorKind::NotFound,
    }
}

/// Whether a rename failed only because the two paths are on different
/// drives (ERROR_NOT_SAME_DEVICE on Windows, EXDEV elsewhere).
fn crosses_devices(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::CrossesDevices || (cfg!(windows) && e.raw_os_error() == Some(17))
}

/// Moves the file `from` to `to`, which must be free: a rename, or across
/// drives a checked copy and then the original deleted.
fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Err(e) if crosses_devices(&e) => copy_move_file(from, to),
        result => result,
    }
}

/// Moves a file by copying it, for moves across drives. If the original
/// cannot be deleted, the copy is, so the file is in one place only.
fn copy_move_file(from: &Path, to: &Path) -> io::Result<()> {
    copy_checked(from, to)?;
    if let Err(e) = fs::remove_file(from) {
        let _ = fs::remove_file(to);
        return Err(e);
    }
    Ok(())
}

/// Copies the file `from` to a new file `to`, never over an existing one,
/// with the original's modified time (the carousel orders notes by it).
/// The copy must have the original's size; on any failure it is removed.
fn copy_checked(from: &Path, to: &Path) -> io::Result<()> {
    let info = fs::metadata(from)?;
    let mut source = File::open(from)?;
    // Fails if `to` exists, which is then left alone.
    let mut copy = File::options().write(true).create_new(true).open(to)?;
    let result = io::copy(&mut source, &mut copy)
        .and_then(|_| copy.set_modified(info.modified()?))
        .and_then(|()| copy.sync_all())
        .and_then(|()| copy.metadata())
        .and_then(|written| {
            if written.len() == info.len() {
                Ok(())
            } else {
                Err(io::Error::other("the copy's size differs"))
            }
        });
    drop(copy);
    if result.is_err() {
        let _ = fs::remove_file(to);
    }
    result
}

/// Moves the folder `from` to `to`, which must be free: a rename, or
/// across drives a checked copy of everything in it and then the original
/// deleted.
fn move_dir(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Err(e) if crosses_devices(&e) => copy_move_dir(from, to),
        result => result,
    }
}

/// Moves a folder by copying it, for moves across drives. A failed copy is
/// removed and the original kept. If the original cannot be deleted after
/// a complete copy, the copy is kept, as part of the original may be gone,
/// and the error is returned.
fn copy_move_dir(from: &Path, to: &Path) -> io::Result<()> {
    // Fails if `to` exists, which is then left alone.
    fs::create_dir(to)?;
    if let Err(e) = copy_tree(from, to) {
        let _ = fs::remove_dir_all(to);
        return Err(e);
    }
    fs::remove_dir_all(from)
}

/// Copies what is in the folder `from` into the empty folder `to`.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            fs::create_dir(&target)?;
            copy_tree(&entry.path(), &target)?;
        } else {
            copy_checked(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Moves `metadata.json` from `old` to `new`, merging it into the new
/// folder's when there is one. Returns whether the old file is gone.
fn move_metadata(old: &Path, new: &Path) -> bool {
    let (from, to) = (old.join(metadata::FILE), new.join(metadata::FILE));
    if !exists(&to) {
        return move_file(&from, &to).is_ok();
    }
    let (Ok(old_meta), Ok(mut merged)) = (Metadata::load(old), Metadata::load(new)) else {
        return false;
    };
    merged.update(&old_meta);
    notes::write_atomic(&to, &merged.to_text()).is_ok() && fs::remove_file(&from).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whisp-migrate-{tag}-{}", notes::new_name()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn sorted(mut names: Vec<String>) -> Vec<String> {
        names.sort();
        names
    }

    #[test]
    fn moves_notes_trash_and_metadata_only() {
        let root = temp_dir("all");
        let (old, new) = (root.join("old"), root.join("deep").join("new"));
        fs::create_dir_all(old.join(TRASH)).unwrap();
        fs::write(old.join("a.md"), "a").unwrap();
        fs::write(old.join("B.MD"), "b").unwrap();
        fs::write(old.join(TRASH).join("gone.md"), "gone").unwrap();
        fs::write(old.join(metadata::FILE), r#"{"a.md": {"pinned": true}}"#).unwrap();
        fs::write(old.join("other.txt"), "other").unwrap();
        fs::create_dir(old.join("sub")).unwrap();
        fs::write(old.join("sub").join("deep.md"), "deep").unwrap();

        let report = move_notes(&old, &new).unwrap();
        assert!(report.complete());
        assert_eq!(
            sorted(report.moved),
            [".trash", "B.MD", "a.md", "metadata.json"]
        );
        assert_eq!(read(&new.join("a.md")), "a");
        assert_eq!(read(&new.join("B.MD")), "b");
        assert_eq!(read(&new.join(TRASH).join("gone.md")), "gone");
        assert_eq!(
            read(&new.join(metadata::FILE)),
            r#"{"a.md": {"pinned": true}}"#
        );
        // Other files and folders stay, and so does the old folder.
        assert!(!old.join("a.md").exists() && !old.join(TRASH).exists());
        assert!(!old.join(metadata::FILE).exists());
        assert_eq!(read(&old.join("other.txt")), "other");
        assert_eq!(read(&old.join("sub").join("deep.md")), "deep");
        assert!(!new.join("other.txt").exists() && !new.join("sub").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn name_clashes_leave_both_files_alone() {
        let root = temp_dir("clash");
        let (old, new) = (root.join("old"), root.join("new"));
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("a.md"), "old a").unwrap();
        fs::write(old.join("b.md"), "old b").unwrap();
        fs::write(new.join("a.md"), "new a").unwrap();

        let report = move_notes(&old, &new).unwrap();
        assert!(!report.complete());
        assert_eq!(report.moved, ["b.md"]);
        assert_eq!(report.stayed, ["a.md"]);
        assert_eq!(read(&old.join("a.md")), "old a");
        assert_eq!(read(&new.join("a.md")), "new a");
        assert_eq!(read(&new.join("b.md")), "old b");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trash_moves_only_into_a_folder_without_one() {
        let root = temp_dir("trash");
        let (old, new) = (root.join("old"), root.join("new"));
        fs::create_dir_all(old.join(TRASH)).unwrap();
        fs::create_dir_all(new.join(TRASH)).unwrap();
        fs::write(old.join(TRASH).join("x.md"), "old x").unwrap();
        fs::write(new.join(TRASH).join("y.md"), "new y").unwrap();

        let report = move_notes(&old, &new).unwrap();
        assert_eq!(report.stayed, [".trash"]);
        assert_eq!(read(&old.join(TRASH).join("x.md")), "old x");
        assert!(!new.join(TRASH).join("x.md").exists());
        assert_eq!(read(&new.join(TRASH).join("y.md")), "new y");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metadata_merges_with_the_old_entries_winning() {
        let root = temp_dir("meta");
        let (old, new) = (root.join("old"), root.join("new"));
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(
            old.join(metadata::FILE),
            r#"{"a.md": {"pinned": false}, "c.md": {"wysiwyg": true}}"#,
        )
        .unwrap();
        fs::write(
            new.join(metadata::FILE),
            r#"{"a.md": {"pinned": true}, "b.md": {"pinned": true}}"#,
        )
        .unwrap();

        let report = move_notes(&old, &new).unwrap();
        assert_eq!(report.moved, [metadata::FILE]);
        assert_eq!(
            read(&new.join(metadata::FILE)),
            r#"{"a.md": {"pinned": false}, "b.md": {"pinned": true}, "c.md": {"wysiwyg": true}}"#
        );
        assert!(!old.join(metadata::FILE).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unparseable_metadata_leaves_both_files() {
        for (old_text, new_text) in [("{oops", r#"{"a.md": {}}"#), (r#"{"a.md": {}}"#, "[1]")] {
            let root = temp_dir("badmeta");
            let (old, new) = (root.join("old"), root.join("new"));
            fs::create_dir_all(&old).unwrap();
            fs::create_dir_all(&new).unwrap();
            fs::write(old.join(metadata::FILE), old_text).unwrap();
            fs::write(new.join(metadata::FILE), new_text).unwrap();

            let report = move_notes(&old, &new).unwrap();
            assert_eq!(report.stayed, [metadata::FILE]);
            assert_eq!(read(&old.join(metadata::FILE)), old_text);
            assert_eq!(read(&new.join(metadata::FILE)), new_text);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn same_or_missing_folder_moves_nothing() {
        let root = temp_dir("same");
        fs::write(root.join("a.md"), "a").unwrap();
        fs::write(root.join(metadata::FILE), "{}").unwrap();
        let report = move_notes(&root, &root.join(".").join("")).unwrap();
        assert_eq!(report, Report::default());
        assert_eq!(read(&root.join("a.md")), "a");
        assert_eq!(read(&root.join(metadata::FILE)), "{}");
        let report = move_notes(&root.join("missing"), &root.join("new")).unwrap();
        assert_eq!(report, Report::default());
        assert!(root.join("new").is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copies_across_drives_keep_content_and_time() {
        let root = temp_dir("copy");
        let from = root.join("a.md");
        fs::write(&from, "text").unwrap();
        let time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        File::options()
            .write(true)
            .open(&from)
            .unwrap()
            .set_modified(time)
            .unwrap();
        let to = root.join("b.md");
        copy_move_file(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(read(&to), "text");
        assert_eq!(fs::metadata(&to).unwrap().modified().unwrap(), time);
        // Never over an existing file, which stays as it was.
        fs::write(&from, "again").unwrap();
        assert!(copy_move_file(&from, &to).is_err());
        assert_eq!(read(&from), "again");
        assert_eq!(read(&to), "text");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_copies_across_drives() {
        let root = temp_dir("copydir");
        let from = root.join("old");
        fs::create_dir_all(from.join("inner")).unwrap();
        fs::write(from.join("x.md"), "x").unwrap();
        fs::write(from.join("inner").join("y.md"), "y").unwrap();
        let to = root.join("new");
        copy_move_dir(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(read(&to.join("x.md")), "x");
        assert_eq!(read(&to.join("inner").join("y.md")), "y");
        // An existing target is left alone, and so is the original.
        fs::create_dir_all(&from).unwrap();
        fs::write(from.join("z.md"), "z").unwrap();
        assert!(copy_move_dir(&from, &to).is_err());
        assert_eq!(read(&from.join("z.md")), "z");
        assert!(!to.join("z.md").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
