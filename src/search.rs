//! Search across all notes, as Whisp's search popover does: an index of the
//! notes folder cached by modified time, case insensitive matching in note
//! bodies, and result rows with snippets.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::notes;

/// The title of a note whose first line is empty.
pub const DEFAULT_TITLE: &str = "New Note";
/// Characters of context before and after a match in a snippet.
const SNIPPET_BEFORE: usize = 12;
const SNIPPET_AFTER: usize = 60;
/// Length of the body excerpt shown for a note without a match.
const EXCERPT_CHARS: usize = 120;

/// A note as indexed.
struct Entry {
    path: PathBuf,
    mtime: SystemTime,
    text: String,
    /// `text` lowercased one character at a time, see `lower`.
    lower: String,
    title: String,
    excerpt: String,
}

/// A snippet around a match, in three parts so the match can be
/// highlighted. `…` marks where the note text was cut.
#[derive(Debug, PartialEq, Eq)]
pub struct Snippet {
    pub before: String,
    pub matched: String,
    pub after: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Row {
    /// A note without a match in its body: all notes for an empty query,
    /// or a note whose title alone matches.
    Note {
        path: PathBuf,
        title: String,
        excerpt: String,
    },
    /// One match in a note's body. Only a note's first match row has the
    /// title. `start` and `end` are UTF-16 offsets in the note text, which
    /// are RichEdit's offsets too, as each `\n` becomes one `\r`.
    Match {
        path: PathBuf,
        title: Option<String>,
        snippet: Snippet,
        start: usize,
        end: usize,
    },
    /// The list was cut short.
    More,
}

#[derive(Default)]
pub struct Index {
    /// Newest first.
    entries: Vec<Entry>,
}

impl Index {
    /// Rereads `dir`, reusing notes whose modified time has not changed.
    /// Blank notes, subfolders and other files are left out.
    pub fn refresh(&mut self, dir: &Path) -> io::Result<()> {
        let mut cached: HashMap<PathBuf, Entry> = self
            .entries
            .drain(..)
            .map(|e| (e.path.clone(), e))
            .collect();
        for item in fs::read_dir(dir)? {
            let Ok(item) = item else { continue };
            let path = item.path();
            let Ok(meta) = item.metadata() else { continue };
            if !meta.is_file()
                || !path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            {
                continue;
            }
            let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
            let entry = match cached.remove(&path) {
                Some(entry) if entry.mtime == mtime => entry,
                _ => {
                    let Ok(text) = notes::read(&path) else {
                        continue;
                    };
                    Entry::new(path, mtime, text)
                }
            };
            if !notes::is_blank(&entry.text) {
                self.entries.push(entry);
            }
        }
        self.entries
            .sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Rows for `query`, at most `cap` of them plus a `More` row if cut.
    pub fn search(&self, query: &str, cap: usize) -> Vec<Row> {
        let mut rows = Vec::new();
        let query = lower(query);
        for entry in &self.entries {
            if query.is_empty() {
                rows.push(entry.note_row());
            } else if entry.lower.contains(&query) {
                let before = rows.len();
                // One more than fits, to know whether the list is cut.
                entry.match_rows(&query, cap + 1 - before, &mut rows);
                if rows.len() == before {
                    // Only the title matched.
                    rows.push(entry.note_row());
                }
            }
            if rows.len() > cap {
                rows.truncate(cap);
                rows.push(Row::More);
                break;
            }
        }
        rows
    }
}

impl Entry {
    fn new(path: PathBuf, mtime: SystemTime, text: String) -> Self {
        Entry {
            path,
            mtime,
            lower: lower(&text),
            title: title(&text),
            excerpt: excerpt(&text),
            text,
        }
    }

    fn note_row(&self) -> Row {
        Row::Note {
            path: self.path.clone(),
            title: self.title.clone(),
            excerpt: self.excerpt.clone(),
        }
    }

    /// Up to `limit` rows for the matches of `query` (lowercased) in the
    /// body, that is after the first line.
    fn match_rows(&self, query: &str, limit: usize, rows: &mut Vec<Row>) {
        let mut from = self.lower.find('\n').map_or(self.lower.len(), |i| i + 1);
        let mut walker = Walker::new(&self.text);
        let mut first = true;
        for _ in 0..limit {
            let Some(found) = self.lower[from..].find(query) else {
                break;
            };
            let at = from + found;
            from = at + query.len();
            let start = walker.floor(at);
            let end = walker.clone().ceil(from);
            rows.push(Row::Match {
                path: self.path.clone(),
                title: first.then(|| self.title.clone()),
                snippet: snippet(&self.text, start.byte, end.byte),
                start: start.units,
                end: end.units,
            });
            first = false;
        }
    }
}

/// Lowercase one character at a time, so that the text and the query are
/// lowered the same way. The length can change (İ becomes two characters),
/// so offsets are mapped back with `Walker`.
fn lower(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

fn lower_len(c: char) -> usize {
    c.to_lowercase().map(char::len_utf8).sum()
}

/// A character boundary in the original text: byte and UTF-16 offsets.
#[derive(Clone, Copy, Debug)]
struct Pos {
    byte: usize,
    units: usize,
    /// Byte offset of this boundary in the lowercased text.
    lower: usize,
}

/// Walks the original text forward, mapping offsets in its lowercased form
/// back to it.
#[derive(Clone)]
struct Walker<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    pos: Pos,
}

impl<'a> Walker<'a> {
    fn new(text: &'a str) -> Self {
        Walker {
            chars: text.chars().peekable(),
            pos: Pos {
                byte: 0,
                units: 0,
                lower: 0,
            },
        }
    }

    /// The boundary of the character that holds lowered offset `at`.
    fn floor(&mut self, at: usize) -> Pos {
        while let Some(&c) = self.chars.peek() {
            if self.pos.lower + lower_len(c) > at {
                break;
            }
            self.step();
        }
        self.pos
    }

    /// The first boundary at or after lowered offset `at`.
    fn ceil(&mut self, at: usize) -> Pos {
        self.floor(at);
        if self.pos.lower < at {
            self.step();
        }
        self.pos
    }

    fn step(&mut self) {
        if let Some(c) = self.chars.next() {
            self.pos.byte += c.len_utf8();
            self.pos.units += c.len_utf16();
            self.pos.lower += lower_len(c);
        }
    }
}

/// Whitespace runs as single spaces.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

/// The snippet for the match at bytes `start..end` of `text`.
fn snippet(text: &str, start: usize, end: usize) -> Snippet {
    let from = text[..start]
        .char_indices()
        .rev()
        .nth(SNIPPET_BEFORE - 1)
        .map_or(0, |(i, _)| i);
    let to = text[end..]
        .char_indices()
        .nth(SNIPPET_AFTER)
        .map_or(text.len(), |(i, _)| end + i);
    let mut before = if from > 0 {
        "\u{2026}".to_string()
    } else {
        String::new()
    };
    before.push_str(&collapse(&text[from..start]));
    let mut after = collapse(&text[end..to]);
    if to < text.len() {
        after.push('\u{2026}');
    }
    Snippet {
        before,
        matched: collapse(&text[start..end]),
        after,
    }
}

/// The first line without leading `#` and spaces, or `DEFAULT_TITLE`.
pub fn title(text: &str) -> String {
    let first = text.split('\n').next().unwrap_or("").trim();
    let first = match first.strip_prefix('#') {
        Some(_) => first.trim_start_matches('#').trim_start(),
        None => first,
    };
    if first.is_empty() {
        DEFAULT_TITLE.to_string()
    } else {
        first.to_string()
    }
}

/// The body on one line, cut to `EXCERPT_CHARS`; empty for a title only note.
fn excerpt(text: &str) -> String {
    let body = text.split_once('\n').map_or("", |(_, body)| body);
    let body = collapse(body);
    let body = body.trim();
    match body.char_indices().nth(EXCERPT_CHARS) {
        Some((cut, _)) => format!("{}\u{2026}", body[..cut].trim_end()),
        None => body.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::time::Duration;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whisp-search-{tag}-{}", notes::new_name()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, text: &str, secs: u64) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        let time = UNIX_EPOCH + Duration::from_secs(1_700_000_000 + secs);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(time)
            .unwrap();
        path
    }

    fn index_of(notes: &[(&str, &str)]) -> Index {
        let mut index = Index::default();
        for (i, (name, text)) in notes.iter().enumerate() {
            index.entries.push(Entry::new(
                PathBuf::from(name),
                UNIX_EPOCH + Duration::from_secs(1_000_000 - i as u64),
                text.to_string(),
            ));
        }
        index
    }

    fn matches(index: &Index, query: &str) -> Vec<(String, usize, usize)> {
        index
            .search(query, 500)
            .into_iter()
            .filter_map(|row| match row {
                Row::Match {
                    snippet,
                    start,
                    end,
                    ..
                } => Some((snippet.matched, start, end)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn titles_and_excerpts() {
        assert_eq!(title("## Shopping list\nmilk"), "Shopping list");
        assert_eq!(title("  plain title  \nbody"), "plain title");
        assert_eq!(title("\nbody"), DEFAULT_TITLE);
        assert_eq!(title("#"), DEFAULT_TITLE);
        assert_eq!(title("C# notes"), "C# notes");
        assert_eq!(excerpt("Title\n  first\n\n\tsecond  "), "first second");
        assert_eq!(excerpt("Title only"), "");
        let long = format!("t\n{}", "word ".repeat(40));
        let e = excerpt(&long);
        assert!(e.ends_with("word\u{2026}"), "{e}");
        assert_eq!(e.chars().count(), 120);
    }

    #[test]
    fn empty_query_lists_every_note_newest_first() {
        let index = index_of(&[("a.md", "# A\nbody a"), ("b.md", "B")]);
        let rows = index.search("", 500);
        assert_eq!(
            rows,
            [
                Row::Note {
                    path: "a.md".into(),
                    title: "A".into(),
                    excerpt: "body a".into()
                },
                Row::Note {
                    path: "b.md".into(),
                    title: "B".into(),
                    excerpt: String::new()
                },
            ]
        );
    }

    #[test]
    fn matches_the_body_case_insensitively() {
        let index = index_of(&[
            ("a.md", "Apple title\napple pie and APPLE juice"),
            ("b.md", "x\nno"),
        ]);
        let rows = index.search("ApPlE", 500);
        assert_eq!(rows.len(), 2, "two body matches, the title is skipped");
        let Row::Match {
            title,
            snippet,
            start,
            end,
            ..
        } = &rows[0]
        else {
            panic!()
        };
        assert_eq!(title.as_deref(), Some("Apple title"));
        assert_eq!((*start, *end), (12, 17));
        assert_eq!(snippet.before, "Apple title ");
        assert_eq!(snippet.matched, "apple");
        assert_eq!(snippet.after, " pie and APPLE juice");
        let Row::Match { title, .. } = &rows[1] else {
            panic!()
        };
        assert_eq!(*title, None, "only the first row of a note has the title");
    }

    #[test]
    fn title_only_match_gives_one_plain_row() {
        let index = index_of(&[("a.md", "Groceries\nmilk")]);
        assert!(matches!(
            &index.search("grocer", 500)[..],
            [Row::Note { .. }]
        ));
        assert!(index.search("nothing", 500).is_empty());
    }

    #[test]
    fn offsets_survive_case_changes_in_length() {
        // İ lowercases to two characters (i and a combining dot), and the
        // emoji is two UTF-16 units: offsets must still point into the text.
        let text = "t\nİstanbul 😀 STRASSE straße İ";
        let index = index_of(&[("a.md", text)]);
        let units: Vec<u16> = text.encode_utf16().collect();
        let at = |s: usize, e: usize| String::from_utf16(&units[s..e]).unwrap();

        let found = matches(&index, "straße");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "straße");
        assert_eq!(at(found[0].1, found[0].2), "straße");

        let found = matches(&index, "stanbul");
        assert_eq!(at(found[0].1, found[0].2), "stanbul");
        let found = matches(&index, "strasse");
        assert_eq!(at(found[0].1, found[0].2), "STRASSE");
        // A query matching part of İ's lowercase covers the whole letter.
        let found = matches(&index, "i");
        assert_eq!(at(found[0].1, found[0].2), "İ");
        assert_eq!(at(found.last().unwrap().1, found.last().unwrap().2), "İ");
        let found = matches(&index, "😀 s");
        assert_eq!(at(found[0].1, found[0].2), "😀 S");
    }

    #[test]
    fn snippets_cut_and_collapse() {
        let text = format!("t\n{}MATCH{}", "a\n\n b ".repeat(10), "z".repeat(100));
        let index = index_of(&[("a.md", &text)]);
        let rows = index.search("match", 500);
        let Row::Match { snippet, .. } = &rows[0] else {
            panic!()
        };
        assert!(snippet.before.starts_with('\u{2026}'));
        assert_eq!(snippet.before.chars().filter(|c| *c == '\n').count(), 0);
        assert_eq!(snippet.matched, "MATCH");
        assert_eq!(snippet.after, format!("{}\u{2026}", "z".repeat(60)));
        // Near the start and end: no ellipsis.
        let index = index_of(&[("a.md", "t\nshort match here")]);
        let Row::Match { snippet, .. } = &index.search("match", 500)[0] else {
            panic!()
        };
        assert_eq!(snippet.before, "t short ");
        assert_eq!(snippet.after, " here");
    }

    #[test]
    fn matches_do_not_overlap() {
        let index = index_of(&[("a.md", "t\naaaa")]);
        let found = matches(&index, "aa");
        assert_eq!(
            found.iter().map(|m| (m.1, m.2)).collect::<Vec<_>>(),
            [(2, 4), (4, 6)]
        );
    }

    #[test]
    fn rows_are_capped() {
        let body = "x\n".to_string() + &"hit ".repeat(10);
        let index = index_of(&[("a.md", &body), ("b.md", &body)]);
        let rows = index.search("hit", 15);
        assert_eq!(rows.len(), 16);
        assert_eq!(rows.last(), Some(&Row::More));
        assert_eq!(index.search("hit", 20).len(), 20);
        let index = index_of(&[("a.md", "a"), ("b.md", "b"), ("c.md", "c")]);
        let rows = index.search("", 2);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2], Row::More);
    }

    #[test]
    fn refresh_reads_the_folder_and_caches_by_mtime() {
        let dir = temp_dir("refresh");
        let old = write(&dir, "old.md", "Old\nbody", 10);
        write(&dir, "new.md", "New\nbody", 20);
        write(&dir, "blank.md", "  \n", 30);
        write(&dir, "other.txt", "Text\nbody", 40);
        fs::create_dir(dir.join(".trash")).unwrap();
        write(&dir.join(".trash"), "gone.md", "Gone\nbody", 50);

        let mut index = Index::default();
        index.refresh(&dir).unwrap();
        let titles = |index: &Index| -> Vec<String> {
            index.entries.iter().map(|e| e.title.clone()).collect()
        };
        assert_eq!(titles(&index), ["New", "Old"]);
        // A blank note is left out but not deleted; that is for the app.
        assert!(dir.join("blank.md").exists());

        // Same mtime: the cached text is kept even though the file changed.
        fs::write(&old, "Changed\nbody").unwrap();
        File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(1_700_000_010))
            .unwrap();
        index.refresh(&dir).unwrap();
        assert_eq!(titles(&index), ["New", "Old"]);
        // A new mtime rereads it, and it is now the newest.
        write(&dir, "old.md", "Changed\nbody", 60);
        fs::remove_file(dir.join("new.md")).unwrap();
        index.refresh(&dir).unwrap();
        assert_eq!(titles(&index), ["Changed"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_thousand_notes_search_quickly() {
        let body = "Some words about things, and more words.\n".repeat(50);
        let notes: Vec<(String, String)> = (0..1000)
            .map(|i| (format!("{i}.md"), format!("Note {i}\n{body}needle {i}")))
            .collect();
        let refs: Vec<(&str, &str)> = notes
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let index = index_of(&refs);
        let started = std::time::Instant::now();
        let rows = index.search("needle", 500);
        assert_eq!(rows.len(), 501);
        // Generous, as tests may run unoptimised.
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
    }
}
