//! Text helpers ported from Whisp's editor: the wrap keys, plain paste
//! without Markdown, link shortening, the link under the pointer and the
//! export file name. Offsets are UTF-16 units, as RichEdit counts them.

use crate::markdown::{self, Style};

/// The result of a wrap key.
#[derive(Debug, PartialEq, Eq)]
pub enum Wrap {
    /// Replace `start..end` with `text`, then select `select` (offsets in
    /// the new text; an empty range is a caret).
    Edit {
        start: usize,
        end: usize,
        text: String,
        select: (usize, usize),
    },
    /// Only move the caret here.
    Caret(usize),
}

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Whisp's `wrap_text` with the same marker on both sides, for the
/// selection `start..end` of `text`:
/// - a selection gets wrapped, or unwrapped when it already is;
/// - no selection: an empty pair around the caret is removed, a marker
///   right after the caret is stepped over, otherwise an empty pair goes in
///   with the caret inside.
pub fn wrap(text: &[u16], (start, mut end): (usize, usize), marker: &str) -> Wrap {
    // RichEdit's Shift+End selects the line break too (GTK's does not); the
    // markers go inside it.
    while end > start && matches!(text.get(end - 1), Some(0x0a | 0x0b | 0x0d)) {
        end -= 1;
    }
    let m = units(marker);
    let n = m.len();
    if start == end {
        let caret = start;
        let before = caret.checked_sub(n).map(|s| &text[s..caret]);
        let after = text.get(caret..caret + n);
        if before == Some(&m[..]) && after == Some(&m[..]) {
            return Wrap::Edit {
                start: caret - n,
                end: caret + n,
                text: String::new(),
                select: (caret - n, caret - n),
            };
        }
        if after == Some(&m[..]) {
            return Wrap::Caret(caret + n);
        }
        return Wrap::Edit {
            start: caret,
            end: caret,
            text: format!("{marker}{marker}"),
            select: (caret + n, caret + n),
        };
    }
    let selected = &text[start..end];
    let wrapped = selected.len() >= 2 * n && selected.starts_with(&m) && selected.ends_with(&m);
    let new: Vec<u16> = if wrapped {
        selected[n..selected.len() - n].to_vec()
    } else {
        [&m[..], selected, &m[..]].concat()
    };
    Wrap::Edit {
        start,
        end,
        select: (start, start + new.len()),
        text: String::from_utf16_lossy(&new),
    }
}

/// Replaces each `open … close` pair on one line by what is inside, as
/// Python's `re.sub` with a non-greedy `(.*?)`: matches never overlap and
/// the guards look at the original text.
fn strip_pairs(
    text: &[char],
    open: &str,
    close: &str,
    open_ok: impl Fn(&[char], usize) -> bool,
    close_ok: impl Fn(&[char], usize) -> bool,
) -> Vec<char> {
    let open: Vec<char> = open.chars().collect();
    let close: Vec<char> = close.chars().collect();
    let n = text.len();
    let mut out = Vec::with_capacity(n);
    let (mut copied, mut i) = (0, 0);
    while i < n {
        if text[i..].starts_with(&open) && open_ok(text, i) {
            let inner = i + open.len();
            let mut k = inner;
            let mut found = None;
            while k < n {
                if text[k..].starts_with(&close) && close_ok(text, k + close.len()) {
                    found = Some(k);
                    break;
                }
                if text[k] == '\n' {
                    break;
                }
                k += 1;
            }
            if let Some(k) = found {
                out.extend_from_slice(&text[copied..i]);
                out.extend_from_slice(&text[inner..k]);
                copied = k + close.len();
                i = copied;
                continue;
            }
        }
        i += 1;
    }
    out.extend_from_slice(&text[copied..]);
    out
}

/// `#{1,6}\s+` anywhere, as Whisp's first plain paste rule.
fn strip_heading_marks(text: &[char]) -> Vec<char> {
    let n = text.len();
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        if text[i] == '#' {
            let run = text[i..].iter().take_while(|&&c| c == '#').count();
            let spaces = text[i + run..]
                .iter()
                .take_while(|c| c.is_whitespace())
                .count();
            if run <= 6 && spaces > 0 {
                i += run + spaces;
                continue;
            }
            // A longer run: the regex may still match its last six marks.
            if run > 6 && spaces > 0 {
                out.extend_from_slice(&text[i..i + run - 6]);
                i += run + spaces;
                continue;
            }
            out.extend_from_slice(&text[i..i + run]);
            i += run;
            continue;
        }
        out.push(text[i]);
        i += 1;
    }
    out
}

/// A list marker at the start of a line: a bullet, `1.` or a box, with the
/// spaces around it.
fn strip_line_marker(line: &str) -> &str {
    let rest = line.trim_start_matches([' ', '\t']);
    fn after_space(s: &str) -> Option<&str> {
        let trimmed = s.trim_start_matches([' ', '\t']);
        (trimmed.len() < s.len()).then_some(trimmed)
    }
    if let Some(r) = rest.strip_prefix(['-', '*', '+']).and_then(after_space) {
        return r;
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(r) = rest[digits..].strip_prefix('.').and_then(after_space)
    {
        return r;
    }
    if let Some(r) = rest.strip_prefix(['\u{2610}', '\u{2611}']) {
        return r.trim_start_matches([' ', '\t']);
    }
    line
}

/// Ctrl+Shift+V: the text without Markdown, by Whisp's `on_clipboard_read`
/// rules in order. The line start rules work on one line at a time.
pub fn strip_markdown(text: &str) -> String {
    let star = |t: &[char], i: usize| i == 0 || t[i - 1] != '*';
    let no_star_after = |t: &[char], j: usize| j >= t.len() || t[j] != '*';
    let underscore = |t: &[char], i: usize| i == 0 || t[i - 1] != '_';
    let no_underscore_after = |t: &[char], j: usize| j >= t.len() || t[j] != '_';
    let any = |_: &[char], _: usize| true;
    let mut c: Vec<char> = text.chars().collect();
    c = strip_heading_marks(&c);
    c = strip_pairs(&c, "**", "**", any, any);
    c = strip_pairs(&c, "*", "*", star, no_star_after);
    c = strip_pairs(&c, "__", "__", any, any);
    c = strip_pairs(&c, "_", "_", underscore, no_underscore_after);
    c = strip_pairs(&c, "`", "`", any, any);
    c = strip_pairs(&c, "~~", "~~", any, any);
    let text: String = c.into_iter().collect();
    let text = strip_links(&text);
    text.split('\n')
        .map(strip_line_marker)
        .collect::<Vec<_>>()
        .join("\n")
}

/// `\[(.*?)\]\(.*?\)` becomes the link text.
fn strip_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut rest = line;
        while let Some(open) = rest.find('[') {
            let after = &rest[open + 1..];
            let Some(close) = after.find("](") else {
                break;
            };
            let Some(paren) = after[close + 2..].find(')') else {
                break;
            };
            out.push_str(&rest[..open]);
            out.push_str(&after[..close]);
            rest = &after[close + 2 + paren + 1..];
        }
        out.push_str(rest);
    }
    out
}

/// Links longer than this become `[short](url)`, as in Whisp.
const LONG_URL: usize = 30;
const SHORT_URL: usize = 25;

/// Whisp's link shortening: a single http(s) URL longer than 30
/// characters becomes `[display](url)`, the display without the scheme and
/// `www.`, cut to 25 characters and `…`.
pub fn shorten_url(text: &str) -> Option<String> {
    let url = text.trim();
    let lower = url.to_ascii_lowercase();
    let scheme = lower.starts_with("https://") || lower.starts_with("http://");
    if !scheme || url.chars().any(char::is_whitespace) || url.chars().count() <= LONG_URL {
        return None;
    }
    // Python's str.replace: every occurrence, case sensitive.
    let mut display = url
        .replace("https://", "")
        .replace("http://", "")
        .replace("www.", "");
    if display.chars().count() > SHORT_URL {
        display = display.chars().take(SHORT_URL).collect::<String>() + "\u{2026}";
    }
    Some(format!("[{display}]({url})"))
}

/// The http(s) URL of the link at UTF-16 column `column` of `line`: the
/// text of a `[text](url)` link or a bare URL, as the highlighter marks
/// them. Other schemes give `None`, so nothing else is ever opened.
pub fn url_at(line: &str, column: usize) -> Option<String> {
    let (spans, _) = markdown::highlight_line(line, false, false);
    let link = spans
        .iter()
        .find(|s| s.style == Style::Link && s.start <= column && column < s.end)?;
    let u = units(line);
    let after = &u[link.end..];
    let url = if after.starts_with(&units("](")) {
        // The URL is the "](url)" symbol span after the text.
        let symbol = spans
            .iter()
            .find(|s| s.style == Style::Symbol && s.start == link.end)?;
        String::from_utf16_lossy(&u[link.end + 2..symbol.end - 1])
    } else {
        String::from_utf16_lossy(&u[link.start..link.end])
    };
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let web = lower.starts_with("https://") || lower.starts_with("http://");
    (web && !url.chars().any(|c| c.is_whitespace() || c.is_control())).then(|| url.to_string())
}

/// Whisp's export file name: the first five words of the first non empty
/// line, letters, digits and spaces only, lowercased and joined with `-`.
pub fn export_name(text: &str) -> String {
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let clean: String = line
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || c.is_whitespace())
            .collect();
        let words: Vec<&str> = clean.split_whitespace().take(5).collect();
        if !words.is_empty() {
            return format!("{}.md", words.join("-").to_lowercase());
        }
        break;
    }
    "note.md".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, sel: (usize, usize), marker: &str) -> (String, (usize, usize)) {
        let u = units(text);
        match wrap(&u, sel, marker) {
            Wrap::Edit {
                start,
                end,
                text: new,
                select,
            } => {
                let out = [&u[..start], &units(&new)[..], &u[end..]].concat();
                (String::from_utf16(&out).unwrap(), select)
            }
            Wrap::Caret(at) => (text.to_string(), (at, at)),
        }
    }

    #[test]
    fn wrap_keys() {
        assert_eq!(
            apply("a bold b", (2, 6), "**"),
            ("a **bold** b".into(), (2, 10))
        );
        assert_eq!(
            apply("a **bold** b", (2, 10), "**"),
            ("a bold b".into(), (2, 6))
        );
        assert_eq!(apply("ab", (1, 1), "**"), ("a****b".into(), (3, 3)));
        assert_eq!(apply("a****b", (3, 3), "**"), ("ab".into(), (1, 1)));
        assert_eq!(apply("a**x**b", (4, 4), "**"), ("a**x**b".into(), (6, 6)));
        assert_eq!(apply("é😀", (3, 3), "~~"), ("é😀~~~~".into(), (5, 5)));
        assert_eq!(apply("x", (0, 1), "_"), ("_x_".into(), (0, 3)));
        // A selection of only the markers is "already wrapped" around nothing.
        assert_eq!(apply("**", (0, 2), "*"), ("".into(), (0, 0)));
    }

    #[test]
    fn plain_paste_strips_markdown() {
        assert_eq!(
            strip_markdown("# Title\nsome **bold** and *it*"),
            "Title\nsome bold and it"
        );
        assert_eq!(strip_markdown("__u__ _v_ `c` ~~s~~"), "u v c s");
        assert_eq!(
            strip_markdown("see [Whisp](https://x.y) now"),
            "see Whisp now"
        );
        assert_eq!(
            strip_markdown("  - one\n2. two\n\u{2610} three\n\u{2611}four"),
            "one\ntwo\nthree\nfour"
        );
        assert_eq!(strip_markdown("a * b * c"), "a  b  c");
        assert_eq!(strip_markdown("no markdown, é"), "no markdown, é");
        assert_eq!(
            strip_markdown("a*b\nc*d"),
            "a*b\nc*d",
            "pairs stay on one line"
        );
        assert_eq!(strip_markdown("#hashtag"), "#hashtag");
    }

    #[test]
    fn url_shortening() {
        let url = "https://www.example.com/a/very/long/path?query=1";
        assert_eq!(
            shorten_url(url).unwrap(),
            format!("[example.com/a/very/long/p\u{2026}]({url})")
        );
        assert_eq!(
            shorten_url(&format!("  {url}\n")).unwrap(),
            format!("[example.com/a/very/long/p\u{2026}]({url})")
        );
        assert_eq!(
            shorten_url("https://short.example/x"),
            None,
            "30 characters or less"
        );
        assert_eq!(
            shorten_url("http://example.com/abcdefghijklmnopq"),
            Some(
                "[example.com/abcdefghijklm\u{2026}](http://example.com/abcdefghijklmnopq)".into()
            )
        );
        assert_eq!(shorten_url("ftp://example.com/a/very/long/path/x"), None);
        assert_eq!(shorten_url("https://example.com/a very/long/path"), None);
    }

    #[test]
    fn link_under_the_pointer() {
        let line = "see [Whisp](https://x.y/a) or https://z.y/b. ok";
        assert_eq!(url_at(line, 5), Some("https://x.y/a".into()));
        assert_eq!(url_at(line, 3), None, "before the link");
        assert_eq!(url_at(line, 12), None, "the url part is not the link text");
        assert_eq!(url_at(line, 32), Some("https://z.y/b".into()));
        assert_eq!(url_at("[x](file:///c:/evil.exe)", 1), None, "only http(s)");
        assert_eq!(url_at("[x](javascript:alert)", 1), None);
        assert_eq!(url_at("😀 https://e.x", 5), Some("https://e.x".into()));
    }

    #[test]
    fn export_names() {
        assert_eq!(
            export_name("\n  # My Shopping List: today & more words here\nbody"),
            "my-shopping-list-today-more.md"
        );
        assert_eq!(export_name("!!!\nsecond"), "note.md");
        assert_eq!(export_name(""), "note.md");
        assert_eq!(export_name("Café au lait"), "caf-au-lait.md");
    }

    #[test]
    fn wrap_leaves_a_selected_line_break_outside() {
        let text: Vec<u16> = "**x**\rnext".encode_utf16().collect();
        // Home, Shift+End in RichEdit: the line and its break.
        match wrap(&text, (0, 6), "**") {
            Wrap::Edit {
                start, end, text, ..
            } => assert_eq!((start, end, text.as_str()), (0, 5, "x")),
            Wrap::Caret(_) => panic!("expected an unwrap"),
        }
        let text: Vec<u16> = "word\rnext".encode_utf16().collect();
        match wrap(&text, (0, 5), "*") {
            Wrap::Edit {
                start, end, text, ..
            } => assert_eq!((start, end, text.as_str()), (0, 4, "*word*")),
            Wrap::Caret(_) => panic!("expected a wrap"),
        }
    }
}
