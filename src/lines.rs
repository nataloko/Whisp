//! Styles per line, and which lines need parsing and restyling after an
//! edit.
//!
//! Markdown styles depend on a line's text and on one piece of state from
//! the lines above it: whether a code fence is open (see
//! `markdown::highlight_line`). So after an edit only the changed lines, and
//! the lines after them until the fence state agrees again, need parsing.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use crate::markdown;
use crate::rtf::{self, Attr};

/// A parsed line, ready for styling.
pub struct Line {
    /// Offset of the first unit. The line ends at `start + len`, before its
    /// line break.
    pub start: usize,
    pub len: usize,
    /// Styled runs relative to `start`; the rest has the base format.
    pub runs: Vec<(usize, usize, Attr)>,
}

/// What is kept of each line between restyles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    /// Hash of the line's text.
    pub text: u64,
    /// A code fence is open at the line's start, and after it.
    pub fence: bool,
    pub fence_after: bool,
    /// Hash of the text and the runs: equal signatures look the same.
    pub signature: u64,
}

fn is_break(unit: u16) -> bool {
    matches!(unit, 0x0a | 0x0b | 0x0d)
}

/// Start and length of each line of `units`, each line break one unit.
fn bounds(units: &[u16]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &u) in units.iter().enumerate() {
        if is_break(u) {
            out.push((start, i - start));
            start = i + 1;
        }
    }
    out.push((start, units.len() - start));
    out
}

fn text_hash(units: &[u16]) -> u64 {
    let mut hasher = DefaultHasher::new();
    units.hash(&mut hasher);
    hasher.finish()
}

/// Parses one line with the fence state at its start.
fn parse(units: &[u16], (start, len): (usize, usize), text: u64, fence: bool) -> (Line, State) {
    let line_units = &units[start..start + len];
    // Lone surrogates become U+FFFD, still one unit, so offsets hold.
    let (spans, fence_after) =
        markdown::highlight_line(&String::from_utf16_lossy(line_units), fence, start == 0);
    let runs: Vec<(usize, usize, Attr)> = rtf::runs(len, &spans)
        .into_iter()
        .filter(|r| r.2 != Attr::default())
        .collect();
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    runs.hash(&mut hasher);
    let state = State {
        text,
        fence,
        fence_after,
        signature: hasher.finish(),
    };
    (Line { start, len, runs }, state)
}

/// Every line of `units`, parsed.
pub fn full(units: &[u16]) -> (Vec<State>, Vec<Line>) {
    let mut states = Vec::new();
    let mut lines = Vec::new();
    let mut fence = false;
    for b in bounds(units) {
        let (line, state) = parse(units, b, text_hash(&units[b.0..b.0 + b.1]), fence);
        fence = state.fence_after;
        states.push(state);
        lines.push(line);
    }
    (states, lines)
}

pub struct Update {
    /// The state of every line of the new text.
    pub state: Vec<State>,
    /// The lines that were parsed, which are the ones to restyle.
    pub lines: Vec<Line>,
    /// Their indices in the new text.
    pub parsed: Range<usize>,
    /// The lines whose text changed.
    pub edited: Range<usize>,
}

/// The new text `units` after an edit, given the state of the old text.
/// Lines at the start and end with unchanged text are not parsed again,
/// unless the fence state at their start changed.
pub fn update(old: &[State], units: &[u16]) -> Update {
    let bounds = bounds(units);
    let texts: Vec<u64> = bounds
        .iter()
        .map(|&(s, l)| text_hash(&units[s..s + l]))
        .collect();
    let n = texts.len();
    let prefix = old
        .iter()
        .zip(&texts)
        .take_while(|(o, t)| o.text == **t)
        .count();
    let room = old.len().min(n) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(texts.iter().rev())
        .take(room)
        .take_while(|(o, t)| o.text == **t)
        .count();
    // The old index of a line in the unchanged suffix.
    let old_of = |i: usize| i + old.len() - n;

    let mut state = old[..prefix].to_vec();
    let mut lines = Vec::new();
    let mut fence = prefix.checked_sub(1).is_some_and(|i| old[i].fence_after);
    let mut i = prefix;
    while i < n {
        // An unchanged line that starts in the same fence state as before
        // ends the work: it and every line after it are as they were.
        if i >= n - suffix && old[old_of(i)].fence == fence {
            break;
        }
        let (line, s) = parse(units, bounds[i], texts[i], fence);
        fence = s.fence_after;
        state.push(s);
        lines.push(line);
        i += 1;
    }
    state.extend((i..n).map(|k| old[old_of(k)]));
    Update {
        state,
        lines,
        parsed: prefix..i,
        edited: prefix..n - suffix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    /// Updates from `old` to `new` and checks the result against a full
    /// parse of `new`: the state of every line, the runs of the lines
    /// parsed, and that every line not parsed looks as before. Returns the
    /// parsed and edited ranges.
    fn check(old: &str, new: &str) -> (Range<usize>, Range<usize>) {
        let (old_state, _) = full(&units(old));
        let new_units = units(new);
        let (want_state, want_lines) = full(&new_units);
        let update = update(&old_state, &new_units);
        assert_eq!(update.state, want_state, "{old:?} -> {new:?}");
        assert_eq!(update.lines.len(), update.parsed.len());
        for (k, line) in update.parsed.clone().zip(&update.lines) {
            assert_eq!(line.runs, want_lines[k].runs, "line {k} of {new:?}");
            assert_eq!(
                (line.start, line.len),
                (want_lines[k].start, want_lines[k].len)
            );
        }
        let (n, m) = (want_state.len(), old_state.len());
        for k in (0..update.parsed.start).chain(update.parsed.end..n) {
            let before = if k < update.parsed.start {
                old_state[k]
            } else {
                old_state[k + m - n]
            };
            assert_eq!(
                before.signature, want_state[k].signature,
                "line {k} of {new:?}"
            );
        }
        (update.parsed, update.edited)
    }

    #[test]
    fn splits_lines_with_relative_runs() {
        let (_, l) = full(&units("a\r\n**b**\u{b}é😀 *i*"));
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
        assert!(full(&units("")).1.len() == 1 && full(&units("a\n")).1.len() == 2);
    }

    #[test]
    fn full_parse_matches_the_whole_text_highlighter() {
        let text = "# T\n```\n*a*\n```\n- [x] **b** `c`\n> q\n\u{2610} t [l](u) https://x.y";
        let u = units(text);
        let (_, lines) = full(&u);
        let flat: Vec<_> = lines
            .iter()
            .flat_map(|l| {
                l.runs
                    .iter()
                    .map(move |r| (l.start + r.0, l.start + r.1, r.2))
            })
            .collect();
        let want: Vec<_> = rtf::runs(u.len(), &markdown::highlight(text))
            .into_iter()
            .filter(|r| r.2 != Attr::default())
            .collect();
        assert_eq!(flat, want);
    }

    #[test]
    fn signatures_cover_text_and_styles() {
        let s = full(&units("x\n*x*\nx\n*x")).0;
        assert_eq!(s[0].signature, s[2].signature);
        assert_ne!(s[0].signature, s[1].signature);
        // Same text with different styles differs too.
        assert_ne!(
            full(&units("*a*")).0[0].signature,
            full(&units("```\n*a*")).0[1].signature
        );
    }

    #[test]
    fn unchanged_text_parses_nothing() {
        assert_eq!(check("a\nb", "a\nb"), (2..2, 2..2));
        assert_eq!(check("", ""), (1..1, 1..1));
    }

    #[test]
    fn one_line_edit_parses_one_line() {
        let big: String = (0..3000)
            .map(|i| format!("- item {i} with **bold**\n"))
            .collect();
        let edited = big.replacen("item 1500 ", "item 1500x ", 1);
        assert_eq!(check(&big, &edited), (1500..1501, 1500..1501));
    }

    #[test]
    fn inserts_deletes_and_splits() {
        assert_eq!(check("a\nb\nc\nd", "a\nb\nnew\nc\nd"), (2..3, 2..3));
        assert_eq!(check("a\nb\nc", "a\nbx\nc"), (1..2, 1..2));
        assert_eq!(check("a\na", "a\na\na"), (2..3, 2..3));
        assert_eq!(check("a\nb\nc", "a\nc"), (1..1, 1..1));
        assert_eq!(check("a\nb\nc", "a\nbc"), (1..2, 1..2));
        assert_eq!(check("a\n**b** c\nd", "a\n**b**\n c\nd"), (1..3, 1..3));
    }

    #[test]
    fn opening_and_closing_fences() {
        let old = "a\n``\nb\n*c*\nd";
        let new = "a\n```\nb\n*c*\nd";
        // Every line after the new fence changes.
        assert_eq!(check(old, new), (1..5, 1..2));
        assert_eq!(check(new, old), (1..5, 1..2));
        // Closing an open fence: the lines after it, up to the end.
        let open = "x\n```\ncode\nmore\n*y*";
        let closed = "x\n```\ncode\n```\nmore\n*y*";
        assert_eq!(check(open, closed), (3..6, 3..4));
        assert_eq!(check(closed, open), (3..5, 3..3));
        // A fence opened and closed by the same edit: only the edited lines.
        assert_eq!(check("a\nb\nc\nd", "a\n```\nb2\n```\nc\nd"), (1..4, 1..4));
        // Deleting the first fence line flips every line after it.
        assert_eq!(
            check("```\na\n```\n*b*\n*c*", "a\n```\n*b*\n*c*"),
            (0..4, 0..0)
        );
    }

    #[test]
    fn edits_on_first_and_last_line() {
        assert_eq!(check("a\nb\nc", "# a\nb\nc"), (0..1, 0..1));
        assert_eq!(check("a\nb\nc", "a\nb\nc!"), (2..3, 2..3));
        assert_eq!(check("a\nb", "a\nb\n"), (2..3, 2..3));
        assert_eq!(check("a\nb\n", "a\nb"), (2..2, 2..2));
        assert_eq!(check("", "x"), (0..1, 0..1));
        assert_eq!(check("```\nx", "```\nx\ny"), (2..3, 2..3));
        assert_eq!(check("\r\r", "\r\u{b}\r"), (3..4, 3..4));
    }
}
