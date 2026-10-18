//! Styles per line, and which lines need restyling after an edit.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use crate::markdown::Span;
use crate::rtf::{self, Attr};

pub struct Line {
    /// Offset of the first unit. The line ends at `start + len`, before its
    /// line break.
    pub start: usize,
    pub len: usize,
    /// Styled runs relative to `start`; the rest has the base format.
    pub runs: Vec<(usize, usize, Attr)>,
    /// Hash of the line's text and runs: equal signatures look the same.
    pub signature: u64,
}

fn is_break(unit: u16) -> bool {
    matches!(unit, 0x0a | 0x0b | 0x0d)
}

/// The lines of `units` (UTF-16, each line break one unit) with their runs
/// from `spans` (from `markdown::highlight` on the same text).
pub fn lines(units: &[u16], spans: &[Span]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    for i in 0..=units.len() {
        if i == units.len() || is_break(units[i]) {
            lines.push(Line {
                start,
                len: i - start,
                runs: Vec::new(),
                signature: 0,
            });
            start = i + 1;
        }
    }
    // Styled runs never contain a line break, since every span lies within
    // one line, so each belongs to the line its start is on.
    let mut index = 0;
    for (start, end, attr) in rtf::runs(units.len(), spans) {
        if attr == Attr::default() {
            continue;
        }
        while start > lines[index].start + lines[index].len {
            index += 1;
        }
        let line = &mut lines[index];
        let end = end.min(line.start + line.len);
        line.runs.push((start - line.start, end - line.start, attr));
    }
    for line in &mut lines {
        let mut hasher = DefaultHasher::new();
        units[line.start..line.start + line.len].hash(&mut hasher);
        line.runs.hash(&mut hasher);
        line.signature = hasher.finish();
    }
    lines
}

/// The lines of `new` to restyle when the editor shows lines with the
/// signatures `old`: those between the longest common prefix and suffix.
pub fn changed(old: &[u64], new: &[u64]) -> Range<usize> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    prefix..new.len() - suffix
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown;

    fn lines_of(text: &str) -> Vec<Line> {
        let units: Vec<u16> = text.encode_utf16().collect();
        lines(&units, &markdown::highlight(text))
    }

    fn signatures(text: &str) -> Vec<u64> {
        lines_of(text).iter().map(|l| l.signature).collect()
    }

    fn changed_lines(old: &str, new: &str) -> Range<usize> {
        changed(&signatures(old), &signatures(new))
    }

    #[test]
    fn splits_lines_with_relative_runs() {
        let l = lines_of("a\r\n**b**\u{b}é😀 *i*");
        // \r and \n are two breaks, as RichEdit would count them.
        assert_eq!(l.len(), 4);
        assert_eq!((l[0].start, l[0].len), (0, 1));
        assert_eq!((l[1].start, l[1].len), (2, 0));
        assert_eq!((l[2].start, l[2].len), (3, 5));
        assert_eq!((l[3].start, l[3].len), (9, 7));
        assert!(l[0].runs.is_empty() && l[1].runs.is_empty());
        let bounds: Vec<_> = l[2].runs.iter().map(|r| (r.0, r.1)).collect();
        assert_eq!(bounds, [(0, 2), (2, 3), (3, 5)]);
        // "é😀 " is four units, so "*i*" starts at 4.
        let bounds: Vec<_> = l[3].runs.iter().map(|r| (r.0, r.1)).collect();
        assert_eq!(bounds, [(4, 5), (5, 6), (6, 7)]);
        assert!(lines_of("").len() == 1 && lines_of("a\n").len() == 2);
    }

    #[test]
    fn signatures_cover_text_and_styles() {
        let s = signatures("x\n*x*\nx\n*x");
        assert_eq!(s[0], s[2]);
        assert_ne!(s[0], s[1]);
        // Same text with different styles differs too.
        assert_ne!(signatures("*a*")[0], signatures("```\n*a*")[1]);
        assert_eq!(signatures("# a")[0], signatures("b\n# a")[1]);
    }

    #[test]
    fn unchanged_text_restyles_nothing() {
        assert!(changed_lines("a\nb", "a\nb").is_empty());
        assert!(changed_lines("", "").is_empty());
    }

    #[test]
    fn insert_in_the_middle() {
        assert_eq!(changed_lines("a\nb\nc\nd", "a\nb\nnew\nc\nd"), 2..3);
        assert_eq!(changed_lines("a\nb\nc", "a\nbx\nc"), 1..2);
        // Repeated lines still align.
        assert_eq!(changed_lines("a\na", "a\na\na"), 2..3);
    }

    #[test]
    fn deleted_line() {
        assert_eq!(changed_lines("a\nb\nc", "a\nc"), 1..1);
        assert_eq!(changed_lines("a\nb\nc", "a\nbc"), 1..2);
    }

    #[test]
    fn line_split_by_enter() {
        assert_eq!(changed_lines("a\n**b** c\nd", "a\n**b**\n c\nd"), 1..3);
    }

    #[test]
    fn opening_a_fence_restyles_everything_after() {
        let old = "a\n``\nb\n*c*\nd";
        let new = "a\n```\nb\n*c*\nd";
        assert_eq!(changed_lines(old, new), 1..5);
        // Closing it again restyles the same lines back.
        assert_eq!(changed_lines(new, old), 1..5);
    }

    #[test]
    fn edits_on_first_and_last_line() {
        assert_eq!(changed_lines("a\nb\nc", "# a\nb\nc"), 0..1);
        assert_eq!(changed_lines("a\nb\nc", "a\nb\nc!"), 2..3);
        assert_eq!(changed_lines("a\nb", "a\nb\n"), 2..3);
        assert_eq!(changed_lines("a\nb\n", "a\nb"), 2..2);
        assert_eq!(changed_lines("", "x"), 0..1);
    }
}
