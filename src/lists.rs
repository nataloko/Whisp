//! Lists and checklists, ported from Whisp's `editor_lists.py`: Enter
//! continues lists, Tab and Shift+Tab indent, Ctrl+S toggles checkboxes,
//! Alt+Up and Alt+Down move lines, and `- [ ]` turns into `☐`.
//!
//! Every action reads the lines (without line breaks) and the caret, and
//! returns one `Edit`, so the editor can apply it as one replacement and one
//! undo step. Positions are (line, column), columns counted in characters.

use std::ops::Range;

/// Replace `start..end` with `text` (lines joined by `\n`), then put the
/// caret at `caret`. Positions are in the text before the edit, except
/// `caret`, which is in the text after it.
#[derive(Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: (usize, usize),
    pub end: (usize, usize),
    pub text: String,
    pub caret: (usize, usize),
}

const INDENT: &str = "    ";
const INDENT_WIDTH: usize = 4;
const EMPTY_BOX: &str = "\u{2610}";
const CHECKED_BOX: &str = "\u{2611}";

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// The first `n` characters of `s`.
fn take(s: &str, n: usize) -> &str {
    &s[..s.char_indices().nth(n).map_or(s.len(), |(i, _)| i)]
}

/// `s` without its first `n` characters.
fn skip(s: &str, n: usize) -> &str {
    &s[take(s, n).len()..]
}

/// Leading whitespace, in characters (Python's `\s*`).
fn indent_of(s: &str) -> usize {
    s.chars().take_while(|c| c.is_whitespace()).count()
}

fn skip_space(s: &str) -> &str {
    s.trim_start()
}

/// A checkbox at the start of `s`: `☐`, `☑`, or `-`, `*`, `+` then
/// `[ ]`, `[x]` or `[X]` (with optional space between). Returns its length
/// in characters.
fn checkbox_len(s: &str) -> Option<usize> {
    let first = s.chars().next()?;
    if first == '\u{2610}' || first == '\u{2611}' {
        return Some(1);
    }
    if !matches!(first, '-' | '*' | '+') {
        return None;
    }
    let after = skip_space(&s[1..]);
    let spaces = chars(&s[1..]) - chars(after);
    let mut c = after.chars();
    match (c.next(), c.next(), c.next()) {
        (Some('['), Some(' ' | 'x' | 'X'), Some(']')) => Some(1 + spaces + 3),
        _ => None,
    }
}

/// Whisp's `is_list_note`: the first line is the word "list", optionally
/// as a heading and followed by `:` or more words.
pub fn is_list_note(first_line: &str) -> bool {
    let line = first_line.trim().to_lowercase();
    let hashes = line.chars().take_while(|&c| c == '#').count();
    let rest = if (1..=6).contains(&hashes) {
        skip_space(&line[hashes..])
    } else {
        &line[..]
    };
    let Some(rest) = rest.strip_prefix("list") else {
        return false;
    };
    // `(\s*[:\s].*)?$`: nothing more, or a colon or space after optional spaces.
    rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with(':')
}

/// A bullet (`-`, `*`, `+`) followed by whitespace: the bullet character.
fn bullet(s: &str) -> Option<char> {
    let mut c = s.chars();
    let b = c.next().filter(|b| matches!(b, '-' | '*' | '+'))?;
    c.next().filter(|n| n.is_whitespace()).map(|_| b)
}

/// `12.` followed by whitespace: the number and the length of `12.`.
fn number(s: &str) -> Option<(u64, usize)> {
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || !s[digits..].starts_with('.') {
        return None;
    }
    let follows = s[digits + 1..].chars().next()?;
    if !follows.is_whitespace() {
        return None;
    }
    Some((s[..digits].parse().ok()?, digits + 1))
}

/// Enter at `caret`. `None` leaves Enter to the editor (a plain newline).
pub fn enter(lines: &[&str], caret: (usize, usize)) -> Option<Edit> {
    let (line, column) = caret;
    let before = take(lines.get(line)?, column);
    let indent = take(before, indent_of(before));
    let rest = skip(before, indent_of(before));
    let insert = |text: String| {
        let caret_column = chars(&text) - 1;
        Some(Edit {
            start: caret,
            end: caret,
            text,
            caret: (line + 1, caret_column),
        })
    };
    // An empty item: the marker goes and the list ends.
    let empty_box = checkbox_len(rest).is_some_and(|n| skip(rest, n).trim().is_empty());
    let end_list = Some(Edit {
        start: (line, 0),
        end: caret,
        text: "\n".into(),
        caret: (line + 1, 0),
    });
    if is_list_note(lines[0]) {
        if empty_box {
            return end_list;
        }
        return insert(format!("\n{indent}{EMPTY_BOX} "));
    }
    if empty_box {
        return end_list;
    }
    if let Some(n) = checkbox_len(rest)
        && skip(rest, n).starts_with(char::is_whitespace)
    {
        return insert(format!("\n{indent}{EMPTY_BOX} "));
    }
    if let Some(b) = bullet(rest) {
        if skip(rest, 1).trim().is_empty() {
            return end_list;
        }
        return insert(format!("\n{indent}{b} "));
    }
    if let Some((n, len)) = number(rest) {
        if skip(rest, len).trim().is_empty() {
            return end_list;
        }
        return insert(format!("\n{indent}{}. ", n.checked_add(1)?));
    }
    None
}

/// Whisp's `recalculate_list_number`: a numbered line takes the number after
/// the nearest earlier item at its indent. Returns the new line.
fn renumber(lines: &[&str], line: usize, text: &str) -> String {
    let indent = indent_of(text);
    let Some((current, len)) = number(skip(text, indent)) else {
        return text.to_string();
    };
    let mut previous = 0;
    for other in lines[..line].iter().rev() {
        if other.trim().is_empty() {
            continue;
        }
        let other_indent = indent_of(other);
        match number(skip(other, other_indent)) {
            Some((n, _)) if other_indent == indent => {
                previous = n;
                break;
            }
            _ if other_indent < indent => break,
            _ => {}
        }
    }
    let new = previous.saturating_add(1);
    if new == current {
        return text.to_string();
    }
    format!("{}{new}.{}", take(text, indent), skip(text, indent + len))
}

/// The caret column on a line whose number changed from `old` to `new`
/// characters at `at`.
fn shift_column(column: usize, at: usize, old: usize, new: usize) -> usize {
    if column >= at + old {
        column + new - old
    } else {
        column.min(at + new)
    }
}

fn number_len(text: &str) -> usize {
    number(skip(text, indent_of(text))).map_or(0, |(_, len)| len - 1)
}

/// Tab: indent the caret's line by 4 spaces, at most one level deeper than
/// the line above, never the first line. `None` does nothing.
pub fn tab(lines: &[&str], caret: (usize, usize)) -> Option<Edit> {
    let (line, column) = caret;
    let text = lines.get(line)?;
    let previous = lines.get(line.checked_sub(1)?)?;
    if indent_of(text) >= indent_of(previous) + INDENT_WIDTH {
        return None;
    }
    let indented = format!("{INDENT}{text}");
    let new = renumber(lines, line, &indented);
    let at = indent_of(&indented);
    let column = shift_column(
        column + INDENT_WIDTH,
        at,
        number_len(&indented),
        number_len(&new),
    );
    Some(Edit {
        start: (line, 0),
        end: (line, chars(text)),
        caret: (line, column),
        text: new,
    })
}

/// Shift+Tab: remove up to 4 leading spaces. `None` does nothing.
pub fn shift_tab(lines: &[&str], caret: (usize, usize)) -> Option<Edit> {
    let (line, column) = caret;
    let text = lines.get(line)?;
    let spaces = take(text, INDENT_WIDTH)
        .chars()
        .take_while(|&c| c == ' ')
        .count();
    if spaces == 0 {
        return None;
    }
    let outdented = skip(text, spaces);
    let new = renumber(lines, line, outdented);
    let at = indent_of(outdented);
    let column = shift_column(
        column.saturating_sub(spaces),
        at,
        number_len(outdented),
        number_len(&new),
    );
    Some(Edit {
        start: (line, 0),
        end: (line, chars(text)),
        caret: (line, column),
        text: new,
    })
}

/// Ctrl+S, Whisp's `toggle_checkbox`.
pub fn toggle_checkbox(lines: &[&str], caret: (usize, usize)) -> Option<Edit> {
    let (line, column) = caret;
    let text = lines.get(line)?;
    let indent = take(text, indent_of(text));
    let rest = skip(text, indent_of(text));

    // `^(?:[☐☑]|[-*+]\s*\[[ xX]\]|[-*+])(?:\s+|$)`
    // A box that is not followed by a space still counts as a bullet.
    let followed = |n: usize| {
        let after = skip(rest, n);
        after.is_empty() || after.starts_with(char::is_whitespace)
    };
    let has_marker = checkbox_len(rest).is_some_and(followed)
        || (rest.starts_with(['-', '*', '+']) && followed(1));
    // `☑` or `[x]` anywhere in the line, as in Whisp.
    let has_checked = rest.contains(CHECKED_BOX) || rest.contains("[x]") || rest.contains("[X]");

    // Strip every leading marker and the spaces after it.
    let mut content = rest;
    loop {
        let n = checkbox_len(content).or_else(|| content.starts_with(['-', '*', '+']).then_some(1));
        let Some(n) = n else { break };
        content = skip_space(skip(content, n));
    }

    let new = if content.trim().is_empty() {
        if has_marker {
            indent.to_string()
        } else {
            format!("{indent}{EMPTY_BOX} ")
        }
    } else if has_checked {
        format!("{indent}{EMPTY_BOX} {content}")
    } else if has_marker {
        format!("{indent}{CHECKED_BOX} {content}")
    } else {
        format!("{indent}{EMPTY_BOX} {content}")
    };
    let diff = chars(&new) as isize - chars(text) as isize;
    let column = (column as isize + diff).clamp(0, chars(&new) as isize) as usize;
    Some(Edit {
        start: (line, 0),
        end: (line, chars(text)),
        caret: (line, column),
        text: new,
    })
}

/// Whisp's list item pattern: `-`, `*`, `+` or `1.` with a space, or a box.
fn is_list_item(s: &str) -> bool {
    let rest = skip(s, indent_of(s));
    bullet(rest).is_some()
        || number(rest).is_some()
        || rest.starts_with(EMPTY_BOX)
        || rest.starts_with(CHECKED_BOX)
}

/// The last line of the item at `start`: following lines indented deeper
/// than `indent`, up to a blank line.
fn subtree_end(lines: &[&str], start: usize, indent: usize) -> usize {
    let mut end = start;
    for (i, text) in lines.iter().enumerate().skip(start + 1) {
        if text.trim().is_empty() || indent_of(text) <= indent {
            break;
        }
        end = i;
    }
    end
}

/// Alt+Up (`up`) or Alt+Down: move the caret's line, or a list item with
/// the items nested under it, past its neighbour. `None` when it cannot move.
pub fn move_line(lines: &[&str], caret: (usize, usize), up: bool) -> Option<Edit> {
    let (line, column) = caret;
    let text = lines.get(line)?;
    let (upper, lower) = if !is_list_item(text) {
        let other = if up { line.checked_sub(1)? } else { line + 1 };
        if other >= lines.len() {
            return None;
        }
        let (a, b) = (line.min(other), line.max(other));
        ((a, a), (b, b))
    } else {
        let indent = indent_of(text);
        let current = (line, subtree_end(lines, line, indent));
        if up {
            // The previous item at the same indent, with its nested items.
            let mut sibling = None;
            for i in (0..line).rev() {
                let t = lines[i];
                if t.trim().is_empty() || indent_of(t) < indent {
                    break;
                }
                if indent_of(t) == indent && is_list_item(t) {
                    sibling = Some((i, subtree_end(&lines[..line], i, indent)));
                    break;
                }
            }
            (sibling?, current)
        } else {
            let next = current.1 + 1;
            let t = lines.get(next)?;
            if indent_of(t) != indent || !is_list_item(t) {
                return None;
            }
            (current, (next, subtree_end(lines, next, indent)))
        }
    };
    let block = |(a, b): (usize, usize)| lines[a..=b].join("\n");
    let moved_start = if up {
        upper.0
    } else {
        upper.0 + (lower.1 - lower.0 + 1)
    };
    let own_start = if up { lower.0 } else { upper.0 };
    Some(Edit {
        start: (upper.0, 0),
        end: (lower.1, chars(lines[lower.1])),
        text: format!("{}\n{}", block(lower), block(upper)),
        caret: (moved_start + (line - own_start), column),
    })
}

/// `- [ ]` or `- [x]` at the start of a line in `range` becomes `☐ ` or `☑ `
/// (Whisp's `convert_markdown_checkboxes`, for the edited lines only).
pub fn convert_checkboxes(
    lines: &[&str],
    range: Range<usize>,
    caret: (usize, usize),
) -> Option<Edit> {
    let range = range.start.min(lines.len())..range.end.min(lines.len());
    let mut converted: Vec<(usize, usize, String)> = Vec::new();
    for (i, line) in lines.iter().enumerate().take(range.end).skip(range.start) {
        if let Some((len, replacement)) = convert_line(line) {
            converted.push((i, len, replacement));
        }
    }
    let first = converted.first()?.0;
    let last = converted.last()?.0;
    let mut text = Vec::new();
    let mut caret_after = caret;
    for (i, line) in lines.iter().enumerate().take(last + 1).skip(first) {
        match converted.iter().find(|c| c.0 == i) {
            Some((_, len, replacement)) => {
                if caret.0 == i {
                    let new = chars(replacement);
                    caret_after.1 = if caret.1 >= *len {
                        caret.1 + new - len
                    } else {
                        caret.1.min(new)
                    };
                }
                text.push(format!("{replacement}{}", skip(line, *len)));
            }
            None => text.push(line.to_string()),
        }
    }
    Some(Edit {
        start: (first, 0),
        end: (last, chars(lines[last])),
        text: text.join("\n"),
        caret: caret_after,
    })
}

/// `^(\s*)([-*+])\s*\[\s*\](\s?)` or the same with `[x]`: the match length
/// and its replacement.
fn convert_line(line: &str) -> Option<(usize, String)> {
    let indent = indent_of(line);
    let rest = skip(line, indent);
    if !rest.starts_with(['-', '*', '+']) {
        return None;
    }
    let after_bullet = skip_space(&rest[1..]);
    let inner = after_bullet.strip_prefix('[')?;
    let (inner_len, checked) = match inner.chars().next()? {
        'x' | 'X' if inner[1..].starts_with(']') => (1, true),
        _ => {
            let spaces = inner.chars().take_while(|c| c.is_whitespace()).count();
            if !skip(inner, spaces).starts_with(']') {
                return None;
            }
            (spaces, false)
        }
    };
    let after = skip(inner, inner_len + 1);
    let trailing = usize::from(after.starts_with(char::is_whitespace));
    let len = indent + 1 + (chars(&rest[1..]) - chars(after_bullet)) + 1 + inner_len + 1 + trailing;
    let mark = if checked { CHECKED_BOX } else { EMPTY_BOX };
    // Whisp always writes the box with a space after it. Converting as soon
    // as `]` is typed, that space plus the one typed next made two, so the
    // space is kept only when one was already there.
    let space = if trailing == 1 { " " } else { "" };
    Some((len, format!("{}{mark}{space}", take(line, indent))))
}

/// Text before the caret that is only a box: `^\s*[☐☑]\s*$`.
fn only_box(s: &str) -> bool {
    let rest = s.trim_start();
    rest.starts_with([EMPTY_BOX_CHAR, CHECKED_BOX_CHAR]) && rest[3..].trim().is_empty()
}

const EMPTY_BOX_CHAR: char = '\u{2610}';
const CHECKED_BOX_CHAR: char = '\u{2611}';

/// Backspace in a list note, as Whisp: on a line that is only a box, with
/// the caret after it, the whole line goes and the caret ends on the line
/// above. `None` leaves Backspace to the editor.
pub fn backspace_box(lines: &[&str], caret: (usize, usize)) -> Option<Edit> {
    let (line, column) = caret;
    let text = lines.get(line)?;
    if !is_list_note(lines[0]) || !only_box(take(text, column)) || !only_box(text) {
        return None;
    }
    Some(match line.checked_sub(1) {
        Some(above) => Edit {
            start: (above, chars(lines[above])),
            end: (line, chars(text)),
            text: String::new(),
            caret: (above, chars(lines[above])),
        },
        None => Edit {
            start: (0, 0),
            end: (0, chars(text)),
            text: String::new(),
            caret: (0, 0),
        },
    })
}

/// Ctrl+Backspace, as Whisp: nothing at the start of a line; on a line that
/// is only a bullet or a box before the caret, that prefix goes. `None`
/// leaves the editor's word delete. An empty edit means "do nothing".
pub fn ctrl_backspace(lines: &[&str], caret: (usize, usize)) -> Option<Edit> {
    let (line, column) = caret;
    let text = lines.get(line)?;
    if column == 0 {
        return Some(Edit {
            start: caret,
            end: caret,
            text: String::new(),
            caret,
        });
    }
    let before = take(text, column);
    let rest = before.trim_start();
    let marker = rest.starts_with(['-', '*', '+', EMPTY_BOX_CHAR, CHECKED_BOX_CHAR]);
    let after_marker: String = rest.chars().skip(1).collect();
    if !marker || !after_marker.trim().is_empty() {
        return None;
    }
    Some(Edit {
        start: (line, 0),
        end: caret,
        text: String::new(),
        caret: (line, 0),
    })
}

/// `#` typed after an empty box in a list note replaces the box, as Whisp.
pub fn hash_after_box(lines: &[&str], caret: (usize, usize)) -> Option<Edit> {
    let (line, column) = caret;
    let text = lines.get(line)?;
    if !is_list_note(lines[0]) || !only_box(take(text, column)) {
        return None;
    }
    Some(Edit {
        start: (line, 0),
        end: caret,
        text: "#".into(),
        caret: (line, 1),
    })
}

/// The UTF-16 offset of `(line, column)`, with one unit per line break.
pub fn offset16(lines: &[&str], (line, column): (usize, usize)) -> usize {
    let before: usize = lines[..line.min(lines.len())]
        .iter()
        .map(|l| l.encode_utf16().count() + 1)
        .sum();
    before
        + lines
            .get(line)
            .map_or(0, |l| take(l, column).encode_utf16().count())
}

/// The (line, column) of UTF-16 offset `offset`.
pub fn position16(lines: &[&str], mut offset: usize) -> (usize, usize) {
    for (i, line) in lines.iter().enumerate() {
        let len = line.encode_utf16().count();
        if offset <= len || i + 1 == lines.len() {
            let mut units = 0;
            let column = line
                .chars()
                .take_while(|c| {
                    units += c.len_utf16();
                    units <= offset
                })
                .count();
            return (i, column);
        }
        offset -= len + 1;
    }
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies `edit` to `text` and returns the new text and caret.
    fn apply(text: &str, edit: &Edit) -> (String, (usize, usize)) {
        let lines: Vec<&str> = text.split('\n').collect();
        let units: Vec<u16> = text.encode_utf16().collect();
        let start = offset16(&lines, edit.start);
        let end = offset16(&lines, edit.end);
        let mut out = String::from_utf16(&units[..start]).unwrap();
        out.push_str(&edit.text);
        out.push_str(&String::from_utf16(&units[end..]).unwrap());
        (out, edit.caret)
    }

    fn run(
        text: &str,
        caret: (usize, usize),
        f: fn(&[&str], (usize, usize)) -> Option<Edit>,
    ) -> Option<(String, (usize, usize))> {
        let lines: Vec<&str> = text.split('\n').collect();
        f(&lines, caret).map(|e| apply(text, &e))
    }

    fn end_of(text: &str) -> (usize, usize) {
        let lines: Vec<&str> = text.split('\n').collect();
        (lines.len() - 1, lines.last().unwrap().chars().count())
    }

    #[test]
    fn list_notes() {
        assert!(is_list_note("List"));
        assert!(is_list_note("## list: groceries"));
        assert!(is_list_note("  list of things  "));
        assert!(is_list_note("#list"));
        assert!(!is_list_note("lists"));
        assert!(!is_list_note("listing"));
        assert!(!is_list_note("####### list"));
        assert!(!is_list_note("my list"));
    }

    #[test]
    fn enter_continues_bullets_numbers_and_boxes() {
        let t = "intro\n  - item";
        assert_eq!(
            run(t, end_of(t), enter),
            Some(("intro\n  - item\n  - ".into(), (2, 4)))
        );
        let t = "x\n* é";
        assert_eq!(run(t, end_of(t), enter).unwrap().0, "x\n* é\n* ");
        let t = "x\n9. nine";
        assert_eq!(
            run(t, end_of(t), enter),
            Some(("x\n9. nine\n10. ".into(), (2, 4)))
        );
        for t in [
            "x\n\u{2610} task",
            "x\n\u{2611} done",
            "x\n- [ ] a",
            "x\n- [x] a",
            "x\n+ [X] a",
        ] {
            let (out, caret) = run(t, end_of(t), enter).unwrap();
            assert!(out.ends_with("\n\u{2610} "), "{t}: {out}");
            assert_eq!(caret, (2, 2));
        }
        // In the middle of a line the rest moves to the new item.
        let t = "x\n- ab";
        assert_eq!(run(t, (1, 3), enter).unwrap().0, "x\n- a\n- b");
    }

    #[test]
    fn enter_on_an_empty_item_ends_the_list() {
        for t in [
            "x\n- ",
            "x\n  * ",
            "x\n3. ",
            "x\n\u{2610} ",
            "x\n- [ ]",
            "x\n\u{2611}",
        ] {
            let (out, caret) = run(t, end_of(t), enter).unwrap();
            assert_eq!(out, "x\n\n", "{t}");
            assert_eq!(caret, (2, 0));
        }
    }

    #[test]
    fn enter_elsewhere_is_left_to_the_editor() {
        for t in ["plain", "x\n-no space", "x\n1.no space", "x\n\u{2610}x"] {
            assert_eq!(run(t, end_of(t), enter), None, "{t}");
        }
    }

    #[test]
    fn enter_in_a_list_note() {
        let t = "List\n  milk";
        assert_eq!(
            run(t, end_of(t), enter).unwrap(),
            ("List\n  milk\n  \u{2610} ".into(), (2, 4))
        );
        // Even on the title line.
        let t = "list";
        assert_eq!(run(t, end_of(t), enter).unwrap().0, "list\n\u{2610} ");
        let t = "list\n\u{2610} ";
        assert_eq!(run(t, end_of(t), enter).unwrap().0, "list\n\n");
    }

    #[test]
    fn tab_indents_one_level_at_most() {
        let t = "a\n- b";
        assert_eq!(run(t, (1, 2), tab), Some(("a\n    - b".into(), (1, 6))));
        let t = "a\n    b";
        assert_eq!(run(t, (1, 4), tab), None, "already a level deeper");
        assert_eq!(run("only", (0, 0), tab), None, "first line");
        let t = "    a\n    b";
        assert_eq!(run(t, (1, 0), tab).unwrap().0, "    a\n        b");
    }

    #[test]
    fn tab_and_shift_tab_renumber() {
        let t = "1. a\n2. b";
        // Indented under item 1, item 2 starts a new list at 1.
        assert_eq!(run(t, (1, 4), tab), Some(("1. a\n    1. b".into(), (1, 8))));
        let t = "1. a\n    1. x\n    2. y\n    3. z";
        // Back out to the first level: it follows 1.
        assert_eq!(
            run(t, (3, 7), shift_tab),
            Some(("1. a\n    1. x\n    2. y\n2. z".into(), (3, 3)))
        );
        // A long number shortening moves the caret with it.
        let t = "1. a\n    1. x\n10. b";
        assert_eq!(
            run(t, (2, 5), tab),
            Some(("1. a\n    1. x\n    2. b".into(), (2, 8)))
        );
    }

    #[test]
    fn shift_tab_removes_up_to_four_spaces() {
        assert_eq!(
            run("a\n      b", (1, 7), shift_tab),
            Some(("a\n  b".into(), (1, 3)))
        );
        assert_eq!(
            run("a\n  b", (1, 1), shift_tab),
            Some(("a\nb".into(), (1, 0)))
        );
        assert_eq!(
            run("a\n\tb", (1, 1), shift_tab),
            None,
            "tabs are left alone"
        );
        assert_eq!(run("a\nb", (1, 0), shift_tab), None);
    }

    #[test]
    fn toggle_checkbox_rules() {
        let t = "x\nbuy milk";
        assert_eq!(
            run(t, (1, 3), toggle_checkbox),
            Some(("x\n\u{2610} buy milk".into(), (1, 5)))
        );
        let t = "x\n  \u{2610} buy";
        assert_eq!(
            run(t, (1, 6), toggle_checkbox).unwrap(),
            ("x\n  \u{2611} buy".into(), (1, 6))
        );
        let t = "x\n\u{2611} buy";
        assert_eq!(
            run(t, (1, 5), toggle_checkbox).unwrap().0,
            "x\n\u{2610} buy"
        );
        assert_eq!(
            run("x\n- [ ] buy", (1, 0), toggle_checkbox).unwrap().0,
            "x\n\u{2611} buy"
        );
        assert_eq!(
            run("x\n- [x] buy", (1, 0), toggle_checkbox).unwrap().0,
            "x\n\u{2610} buy"
        );
        assert_eq!(
            run("x\n* [X] buy", (1, 0), toggle_checkbox).unwrap().0,
            "x\n\u{2610} buy"
        );
        // A bullet becomes a checked box, as in Whisp.
        assert_eq!(
            run("x\n- buy", (1, 0), toggle_checkbox).unwrap().0,
            "x\n\u{2611} buy"
        );
        // An empty box goes; an empty line gets one.
        assert_eq!(
            run("x\n  \u{2610} ", (1, 4), toggle_checkbox).unwrap(),
            ("x\n  ".into(), (1, 2))
        );
        assert_eq!(
            run("x\n", (1, 0), toggle_checkbox).unwrap(),
            ("x\n\u{2610} ".into(), (1, 2))
        );
        // Non ASCII content and the caret's place relative to the text.
        let t = "\u{2610} 日本 😀";
        assert_eq!(
            run(t, (0, 4), toggle_checkbox).unwrap(),
            ("\u{2611} 日本 😀".into(), (0, 4))
        );
    }

    #[test]
    fn move_plain_lines() {
        let t = "a\nb\nc";
        assert_eq!(
            run(t, (1, 1), |l, c| move_line(l, c, true)),
            Some(("b\na\nc".into(), (0, 1)))
        );
        assert_eq!(
            run(t, (1, 0), |l, c| move_line(l, c, false)),
            Some(("a\nc\nb".into(), (2, 0)))
        );
        assert_eq!(run(t, (0, 0), |l, c| move_line(l, c, true)), None);
        assert_eq!(run(t, (2, 0), |l, c| move_line(l, c, false)), None);
    }

    #[test]
    fn move_list_items_with_their_children() {
        let t = "- a\n    - a1\n- b\n    - b1\n    - b2\nend";
        let (out, caret) = run(t, (2, 2), |l, c| move_line(l, c, true)).unwrap();
        assert_eq!(out, "- b\n    - b1\n    - b2\n- a\n    - a1\nend");
        assert_eq!(caret, (0, 2));
        let (out, caret) = run(t, (0, 1), |l, c| move_line(l, c, false)).unwrap();
        assert_eq!(out, "- b\n    - b1\n    - b2\n- a\n    - a1\nend");
        assert_eq!(caret, (3, 1));
        // No sibling: the last item cannot move down past "end".
        assert_eq!(run(t, (2, 0), |l, c| move_line(l, c, false)), None);
        // A blank line ends the list.
        assert_eq!(
            run("- a\n\n- b", (2, 0), |l, c| move_line(l, c, true)),
            None
        );
        // A nested item moves among its siblings only.
        let t = "- a\n    - x\n    - y";
        assert_eq!(
            run(t, (2, 5), |l, c| move_line(l, c, true)).unwrap(),
            ("- a\n    - y\n    - x".into(), (1, 5))
        );
    }

    #[test]
    fn converts_markdown_checkboxes_on_edited_lines() {
        let t = "a\n  - [ ] x\n* [x]\n- [] y\n- [ ]z";
        let lines: Vec<&str> = t.split('\n').collect();
        let edit = convert_checkboxes(&lines, 0..5, (2, 5)).unwrap();
        let (out, caret) = apply(t, &edit);
        // A space after the box only where one was already there.
        assert_eq!(out, "a\n  \u{2610} x\n\u{2611}\n\u{2610} y\n\u{2610}z");
        assert_eq!(caret, (2, 1));
        // Lines outside the range are left alone.
        assert!(convert_checkboxes(&lines, 0..1, (0, 0)).is_none());
        assert!(
            convert_checkboxes(&["- [y]", "x [ ]", "-[ ]"], 0..3, (0, 0))
                .is_some_and(|e| e.start.0 == 2)
        );
    }

    #[test]
    fn typing_a_checkbox_leaves_one_space() {
        // Converted as soon as `]` is typed, with the caret at the end...
        let edit = convert_checkboxes(&["- [ ]"], 0..1, (0, 5)).unwrap();
        let (out, caret) = apply("- [ ]", &edit);
        assert_eq!((out.as_str(), caret), ("\u{2610}", (0, 1)));
        // ...so the space typed next is the only one.
        let edit = convert_checkboxes(&["- [ ] task"], 0..1, (0, 10)).unwrap();
        assert_eq!(apply("- [ ] task", &edit).0, "\u{2610} task");
    }

    #[test]
    fn list_note_backspace_ctrl_backspace_and_hash() {
        let t = "List\nmilk\n  \u{2610} ";
        assert_eq!(
            run(t, (2, 4), backspace_box),
            Some(("List\nmilk".into(), (1, 4)))
        );
        assert_eq!(
            run(t, (2, 3), backspace_box),
            Some(("List\nmilk".into(), (1, 4)))
        );
        assert_eq!(
            run("List\n\u{2610} x", (1, 2), backspace_box),
            None,
            "the line has text"
        );
        assert_eq!(
            run("notes\n\u{2610} ", (1, 2), backspace_box),
            None,
            "not a list note"
        );
        assert_eq!(
            run("\u{2610} ", (0, 2), backspace_box),
            None,
            "a box is not the word list"
        );

        assert_eq!(
            run("a\nb", (1, 0), ctrl_backspace),
            Some(("a\nb".into(), (1, 0))),
            "does nothing"
        );
        assert_eq!(
            run("a\n  - ", (1, 4), ctrl_backspace),
            Some(("a\n".into(), (1, 0)))
        );
        assert_eq!(
            run("a\n\u{2611}", (1, 1), ctrl_backspace),
            Some(("a\n".into(), (1, 0)))
        );
        assert_eq!(
            run("a\n- word", (1, 6), ctrl_backspace),
            None,
            "the editor's word delete"
        );

        assert_eq!(
            run("list\n  \u{2610} ", (1, 4), hash_after_box),
            Some(("list\n#".into(), (1, 1)))
        );
        assert_eq!(run("todo\n\u{2610} ", (1, 2), hash_after_box), None);
    }

    #[test]
    fn utf16_positions() {
        let lines = ["é😀a", "", "日"];
        assert_eq!(offset16(&lines, (0, 2)), 3);
        assert_eq!(offset16(&lines, (2, 1)), 7);
        assert_eq!(position16(&lines, 3), (0, 2));
        assert_eq!(position16(&lines, 5), (1, 0));
        assert_eq!(position16(&lines, 7), (2, 1));
        assert_eq!(position16(&lines, 0), (0, 0));
    }
}
