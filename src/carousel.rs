//! How the notes are laid out in the carousel, as in Whisp: pinned notes
//! first, then the rest, each group oldest first. Old notes can be left out
//! (`archive_days`) and the number of unpinned notes can be capped
//! (`max_carousel_size`); left out notes stay on disk.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const DAY: Duration = Duration::from_secs(86400);

/// The startup arrangement of `notes`, which are paths with their modified
/// times. `archive_days` and `max_size` of 0 mean no limit. Pinned notes are
/// never archived and do not count towards `max_size`. Ties in time are
/// broken by path, so the order is stable.
pub fn arrange(
    mut notes: Vec<(PathBuf, SystemTime)>,
    now: SystemTime,
    archive_days: u32,
    max_size: usize,
    is_pinned: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    notes.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
    let (pinned, mut rest): (Vec<_>, Vec<_>) = notes.into_iter().partition(|(p, _)| is_pinned(p));
    if archive_days > 0 {
        let limit = DAY * archive_days;
        // A modified time in the future gives an error: never archived.
        rest.retain(|(_, time)| now.duration_since(*time).map_or(true, |age| age <= limit));
    }
    if max_size > 0 && rest.len() > max_size {
        rest.drain(..rest.len() - max_size);
    }
    pinned.into_iter().chain(rest).map(|(p, _)| p).collect()
}

/// Where a note goes in the list when it is unpinned or restored. `others`
/// are the other notes in list order, each with its pinned flag and modified
/// time; `pinned` and `time` describe the note being placed. This is
/// Whisp's walk.
///
/// An unpinned note walks the whole list: pinned notes are passed, and so
/// is every unpinned note that is not newer, stopping at the first newer
/// one. A pinned note only walks the pinned block at the front, passing
/// pinned notes that are not newer.
pub fn insert_position(others: &[(bool, SystemTime)], pinned: bool, time: SystemTime) -> usize {
    let mut at = 0;
    for &(other_pinned, other_time) in others {
        let advance = if pinned {
            other_pinned && other_time <= time
        } else {
            other_pinned || other_time <= time
        };
        if !advance {
            break;
        }
        at += 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn t(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn notes(list: &[(&str, u64)]) -> Vec<(PathBuf, SystemTime)> {
        list.iter()
            .map(|&(n, s)| (PathBuf::from(n), t(s)))
            .collect()
    }

    fn names(paths: Vec<PathBuf>) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    fn pinned(p: &Path) -> bool {
        p.to_string_lossy().starts_with('p')
    }

    #[test]
    fn pinned_first_then_oldest_first() {
        let list = notes(&[("c", 30), ("p2", 50), ("a", 10), ("p1", 5), ("b", 20)]);
        let out = arrange(list, t(100), 0, 0, pinned);
        assert_eq!(names(out), ["p1", "p2", "a", "b", "c"]);
    }

    #[test]
    fn ties_break_by_path() {
        let list = notes(&[("b", 10), ("a", 10), ("c", 10)]);
        let out = arrange(list, t(100), 0, 0, |_| false);
        assert_eq!(names(out), ["a", "b", "c"]);
    }

    #[test]
    fn archive_boundary_is_strict() {
        let now = t(10 * 86400);
        // Exactly 2 days old stays, one second more goes.
        let list = notes(&[
            ("old", 8 * 86400 - 1),
            ("edge", 8 * 86400),
            ("new", 9 * 86400),
        ]);
        let out = arrange(list, now, 2, 0, |_| false);
        assert_eq!(names(out), ["edge", "new"]);
    }

    #[test]
    fn future_notes_are_never_archived() {
        let list = notes(&[("future", 200 * 86400), ("old", 1)]);
        let out = arrange(list, t(100 * 86400), 1, 0, |_| false);
        assert_eq!(names(out), ["future"]);
    }

    #[test]
    fn max_size_keeps_the_newest_unpinned() {
        let list = notes(&[("a", 1), ("b", 2), ("c", 3), ("d", 4), ("p1", 0), ("p2", 9)]);
        let out = arrange(list, t(100), 0, 2, pinned);
        assert_eq!(names(out), ["p1", "p2", "c", "d"]);
    }

    #[test]
    fn max_size_applies_after_archiving() {
        let now = t(10 * 86400);
        let list = notes(&[("a", 1), ("b", 9 * 86400), ("c", 9 * 86400 + 5)]);
        let out = arrange(list, now, 2, 5, |_| false);
        assert_eq!(names(out), ["b", "c"]);
    }

    #[test]
    fn pinned_notes_are_exempt() {
        let list = notes(&[("p1", 1), ("a", 2), ("b", 100 * 86400 - 9), ("p2", 4)]);
        let out = arrange(list, t(100 * 86400), 1, 1, pinned);
        assert_eq!(names(out), ["p1", "p2", "b"]);
        let out = arrange(notes(&[("p1", 1), ("p2", 2)]), t(100 * 86400), 1, 1, pinned);
        assert_eq!(names(out), ["p1", "p2"]);
    }

    #[test]
    fn insert_walk_for_unpinned_notes() {
        let others = [
            (true, t(50)),
            (true, t(10)),
            (false, t(10)),
            (false, t(20)),
            (false, t(30)),
        ];
        // Passes both pinned notes, then every unpinned note not newer.
        assert_eq!(insert_position(&others, false, t(5)), 2);
        assert_eq!(insert_position(&others, false, t(10)), 3);
        assert_eq!(insert_position(&others, false, t(25)), 4);
        assert_eq!(insert_position(&others, false, t(99)), 5);
        assert_eq!(insert_position(&[], false, t(1)), 0);
    }

    #[test]
    fn insert_walk_for_pinned_notes() {
        let others = [(true, t(10)), (true, t(20)), (true, t(30)), (false, t(1))];
        assert_eq!(insert_position(&others, true, t(5)), 0);
        assert_eq!(insert_position(&others, true, t(20)), 2);
        assert_eq!(insert_position(&others, true, t(99)), 3);
        assert_eq!(insert_position(&[(false, t(1))], true, t(99)), 0);
    }
}
