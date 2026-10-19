//! Markdown highlighting rules, ported from Whisp's highlighter.
//!
//! Offsets are UTF-16 code units and every line break (`\n`, `\r` or `\v`)
//! counts as one unit, which is how RichEdit counts characters.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Heading line, level 1 to 6.
    Heading(u8),
    Quote,
    Comment,
    Bold,
    Italic,
    Strike,
    Link,
    Code,
    ListMarker,
    Checkbox,
    /// A checked checklist item: the box and the rest of its line.
    Checked,
    /// Markdown syntax characters: `#`, `**`, `` ` ``, `[`, `](url)` and so on.
    Symbol,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

/// Styles for `text`, to apply in the returned order: a later span overrides
/// the attributes an earlier one set.
pub fn highlight(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut in_fence = false;
    let mut chars = Vec::new();
    for line in text.split(['\n', '\r', '\u{b}']) {
        chars.clear();
        chars.extend(line.chars());
        let mut spans = Spans::new(&chars, offset, &mut out);
        in_fence = block(&chars, in_fence, &mut spans);
        offset = spans.pos[chars.len()] + 1;
    }
    out.sort_by_key(|s| rank(s.style));
    out
}

/// Styles for one line (without its line break), given whether a code
/// fence is open at its start, with offsets from the line's start. Also
/// returns whether a fence is open after it: the only state that carries
/// from one line to the next. Concatenating these per line and sorting by
/// style gives `highlight`'s result.
pub fn highlight_line(line: &str, in_fence: bool) -> (Vec<Span>, bool) {
    let mut out = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Spans::new(&chars, 0, &mut out);
    let after = block(&chars, in_fence, &mut spans);
    out.sort_by_key(|s| rank(s.style));
    (out, after)
}

fn rank(style: Style) -> u8 {
    match style {
        Style::Heading(_) => 0,
        Style::Quote | Style::Comment => 1,
        Style::Bold | Style::Italic | Style::Strike => 2,
        Style::Link => 3,
        Style::Code => 4,
        Style::ListMarker | Style::Checkbox => 5,
        Style::Checked => 6,
        Style::Symbol => 7,
    }
}

/// Collects spans for one line, mapping char indices to UTF-16 offsets.
struct Spans<'a> {
    pos: Vec<usize>,
    out: &'a mut Vec<Span>,
}

impl<'a> Spans<'a> {
    fn new(chars: &[char], base: usize, out: &'a mut Vec<Span>) -> Self {
        let mut pos = Vec::with_capacity(chars.len() + 1);
        let mut p = base;
        for c in chars {
            pos.push(p);
            p += c.len_utf16();
        }
        pos.push(p);
        Spans { pos, out }
    }

    fn add(&mut self, start: usize, end: usize, style: Style) {
        if start < end {
            self.out.push(Span {
                start: self.pos[start],
                end: self.pos[end],
                style,
            });
        }
    }
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Block rules for one line; returns whether a code fence is open after it.
fn block(c: &[char], in_fence: bool, sp: &mut Spans) -> bool {
    let len = c.len();
    let indent = c.iter().take_while(|&&ch| is_blank(ch)).count();
    let rest = &c[indent..];

    if rest.starts_with(&['`', '`', '`']) {
        let ticks = rest.iter().take_while(|&&ch| ch == '`').count();
        sp.add(0, len, Style::Code);
        sp.add(indent, indent + ticks, Style::Symbol);
        return !in_fence;
    }
    if in_fence {
        sp.add(0, len, Style::Code);
        return true;
    }

    if is_comment(rest) {
        sp.add(0, len, Style::Comment);
    }

    let hashes = c.iter().take_while(|&&ch| ch == '#').count();
    let mut start = indent;
    if (1..=6).contains(&hashes) && c.get(hashes).is_some_and(|&ch| is_blank(ch)) {
        sp.add(0, len, Style::Heading(hashes as u8));
        sp.add(0, hashes, Style::Symbol);
        start = hashes;
    } else if rest.first() == Some(&'>') {
        let n = rest
            .iter()
            .take_while(|&&ch| ch == '>' || is_blank(ch))
            .count();
        sp.add(0, len, Style::Quote);
        sp.add(indent, indent + n, Style::Symbol);
        start = indent + n;
    } else if is_rule(rest) {
        sp.add(indent, len, Style::Symbol);
        return false;
    } else if let Some((n, checked)) = checkbox(rest) {
        sp.add(indent, indent + n, Style::Checkbox);
        if checked {
            sp.add(indent, len, Style::Checked);
        }
        start = indent + n;
    } else if let Some(n) = list_marker(rest) {
        sp.add(indent, indent + n, Style::ListMarker);
        start = indent + n;
    }
    inline(c, start, sp);
    false
}

/// `// text`, optionally after a Whisp checkbox.
fn is_comment(rest: &[char]) -> bool {
    let mut i = 0;
    if matches!(rest.first(), Some('☐' | '☑')) {
        i = 1 + rest[1..].iter().take_while(|&&ch| is_blank(ch)).count();
    }
    rest[i..].starts_with(&['/', '/'])
}

/// A thematic break: `---`, `***` or `___`, spaces allowed.
fn is_rule(rest: &[char]) -> bool {
    let Some(&ch) = rest.first() else {
        return false;
    };
    matches!(ch, '-' | '*' | '_')
        && rest.iter().all(|&c| c == ch || is_blank(c))
        && rest.iter().filter(|&&c| c == ch).count() >= 3
}

/// `☐`, `☑`, `- [ ]` or `- [x]`: the box length and whether it is checked.
fn checkbox(rest: &[char]) -> Option<(usize, bool)> {
    match rest.first()? {
        '☐' => Some((1, false)),
        '☑' => Some((1, true)),
        '-' | '*' | '+' => {
            let i = 1 + rest[1..].iter().take_while(|&&ch| is_blank(ch)).count();
            match rest.get(i..i + 3)? {
                ['[', mark @ (' ' | 'x' | 'X'), ']'] => Some((i + 3, *mark != ' ')),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `-`, `*`, `+` or `1.` followed by a space: the marker length.
fn list_marker(rest: &[char]) -> Option<usize> {
    let n = match rest.first()? {
        '-' | '*' | '+' => 1,
        _ => {
            let digits = rest.iter().take_while(|ch| ch.is_ascii_digit()).count();
            if digits == 0 || rest.get(digits) != Some(&'.') {
                return None;
            }
            digits + 1
        }
    };
    rest.get(n).is_some_and(|&ch| is_blank(ch)).then_some(n)
}

/// Inline rules for `c[from..]`.
fn inline(c: &[char], from: usize, sp: &mut Spans) {
    let n = c.len();
    // Characters that are already markup and cannot start or end anything else.
    let mut masked = vec![false; n];

    // Escapes and code spans.
    let mut i = from;
    while i < n {
        match c[i] {
            '\\' if c.get(i + 1).is_some_and(|ch| ch.is_ascii_punctuation()) => {
                sp.add(i, i + 1, Style::Symbol);
                masked[i] = true;
                masked[i + 1] = true;
                i += 2;
            }
            '`' => {
                let run = run_len(c, i, '`');
                if let Some(close) = find_run(c, i + run, '`', run) {
                    let end = close + run;
                    sp.add(i, end, Style::Code);
                    sp.add(i, i + run, Style::Symbol);
                    sp.add(close, end, Style::Symbol);
                    masked[i..end].fill(true);
                    i = end;
                } else {
                    i += run;
                }
            }
            _ => i += 1,
        }
    }

    // Links: [text](url) and ![alt](src).
    let mut i = from;
    while i < n {
        if c[i] == '[' && !masked[i] {
            let close = (i + 1..n).find(|&j| c[j] == ']' && !masked[j]);
            if let Some(close) = close.filter(|&j| c.get(j + 1) == Some(&'('))
                && let Some(paren) = (close + 2..n).find(|&j| c[j] == ')' && !masked[j])
            {
                let open = if i > from && c[i - 1] == '!' && !masked[i - 1] {
                    i - 1
                } else {
                    i
                };
                sp.add(open, i + 1, Style::Symbol);
                sp.add(i + 1, close, Style::Link);
                sp.add(close, paren + 1, Style::Symbol);
                masked[open..=i].fill(true);
                masked[close..=paren].fill(true);
                i = paren + 1;
                continue;
            }
        }
        i += 1;
    }

    // Bare URLs.
    let mut i = from;
    while i < n {
        if !masked[i]
            && (i == 0 || !c[i - 1].is_alphanumeric())
            && let Some(scheme) = url_scheme(&c[i..])
        {
            let mut end = i;
            while end < n && !c[end].is_whitespace() && !masked[end] {
                end += 1;
            }
            end = trim_url(c, i, end);
            if end > i + scheme {
                sp.add(i, end, Style::Link);
                masked[i..end].fill(true);
                i = end;
                continue;
            }
        }
        i += 1;
    }

    emphasis(c, from, &masked, sp);
}

fn run_len(c: &[char], i: usize, ch: char) -> usize {
    c[i..].iter().take_while(|&&x| x == ch).count()
}

/// The next run of exactly `len` copies of `ch` at or after `from`.
fn find_run(c: &[char], from: usize, ch: char, len: usize) -> Option<usize> {
    let mut j = from;
    while j < c.len() {
        if c[j] == ch {
            let run = run_len(c, j, ch);
            if run == len {
                return Some(j);
            }
            j += run;
        } else {
            j += 1;
        }
    }
    None
}

fn url_scheme(c: &[char]) -> Option<usize> {
    ["https://", "http://"].iter().find_map(|s| {
        let n = s.chars().count();
        (c.len() >= n
            && c.iter()
                .zip(s.chars())
                .all(|(a, b)| a.eq_ignore_ascii_case(&b)))
        .then_some(n)
    })
}

/// Drops trailing punctuation that most likely ends the sentence, not the URL.
fn trim_url(c: &[char], start: usize, mut end: usize) -> usize {
    while end > start {
        let last = c[end - 1];
        let unbalanced = last == ')'
            && c[start..end].iter().filter(|&&x| x == ')').count()
                > c[start..end].iter().filter(|&&x| x == '(').count();
        if unbalanced
            || matches!(
                last,
                '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | '>' | '*' | '_' | '~'
            )
        {
            end -= 1;
        } else {
            break;
        }
    }
    end
}

struct Run {
    ch: char,
    lo: usize,
    hi: usize,
    can_open: bool,
    can_close: bool,
}

/// `**bold**`, `*italic*`, `_italic_`, `***both***` and `~~strike~~`, using
/// a simplified version of CommonMark's delimiter run rules.
fn emphasis(c: &[char], from: usize, masked: &[bool], sp: &mut Spans) {
    let mut runs = Vec::new();
    let mut i = from;
    while i < c.len() {
        let ch = c[i];
        if masked[i] || !matches!(ch, '*' | '_' | '~') {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < c.len() && c[j] == ch && !masked[j] {
            j += 1;
        }
        let prev = i.checked_sub(1).map(|p| c[p]);
        let next = c.get(j).copied();
        let mut can_open = next.is_some_and(|x| !x.is_whitespace());
        let mut can_close = prev.is_some_and(|x| !x.is_whitespace());
        if ch == '_' {
            can_open &= !prev.is_some_and(char::is_alphanumeric);
            can_close &= !next.is_some_and(char::is_alphanumeric);
        }
        if ch != '~' || j - i == 2 {
            runs.push(Run {
                ch,
                lo: i,
                hi: j,
                can_open,
                can_close,
            });
        }
        i = j;
    }

    let mut openers: Vec<usize> = Vec::new();
    for r in 0..runs.len() {
        if runs[r].can_close {
            while runs[r].lo < runs[r].hi {
                let Some(k_pos) = openers.iter().rposition(|&k| runs[k].ch == runs[r].ch) else {
                    break;
                };
                let k = openers[k_pos];
                let both_double = runs[k].hi - runs[k].lo >= 2 && runs[r].hi - runs[r].lo >= 2;
                let (used, style) = match runs[r].ch {
                    '~' => (2, Style::Strike),
                    _ if both_double => (2, Style::Bold),
                    _ => (1, Style::Italic),
                };
                runs[k].hi -= used;
                let open = runs[k].hi;
                let close = runs[r].lo;
                runs[r].lo += used;
                sp.add(open, close + used, style);
                sp.add(open, open + used, Style::Symbol);
                sp.add(close, close + used, Style::Symbol);
                if runs[k].lo == runs[k].hi {
                    openers.truncate(k_pos);
                } else {
                    openers.truncate(k_pos + 1);
                }
            }
        }
        if runs[r].can_open && runs[r].lo < runs[r].hi {
            openers.push(r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Style::*;
    use super::*;

    /// Spans as (text covered, style), which reads better than raw offsets.
    fn spans(text: &str) -> Vec<(String, Style)> {
        let units: Vec<u16> = text.encode_utf16().collect();
        highlight(text)
            .into_iter()
            .map(|s| (String::from_utf16(&units[s.start..s.end]).unwrap(), s.style))
            .collect()
    }

    fn has(text: &str, covered: &str, style: Style) -> bool {
        spans(text).contains(&(covered.to_string(), style))
    }

    #[test]
    fn plain_text_has_no_spans() {
        assert!(highlight("hello world\n\njust text 2 * 3 * 4").is_empty());
        assert!(highlight("").is_empty());
    }

    #[test]
    fn headings() {
        let s = spans("## Title");
        assert!(s.contains(&("## Title".into(), Heading(2))));
        assert!(s.contains(&("##".into(), Symbol)));
        assert!(highlight("#nospace").is_empty());
        assert!(highlight("####### seven").is_empty());
        assert!(has("###### six", "######", Symbol));
    }

    #[test]
    fn offsets_count_lines_and_utf16() {
        // "é" is one unit, the emoji two, each line break one.
        let text = "é😀\n\n# 日本";
        let h = highlight(text);
        let heading = h.iter().find(|s| s.style == Heading(1)).unwrap();
        assert_eq!((heading.start, heading.end), (5, 9));
        // Same offsets with RichEdit's \r line breaks.
        let h2 = highlight("é😀\r\r# 日本");
        assert_eq!(h, h2);
    }

    #[test]
    fn inline_offsets_after_wide_chars() {
        let h = highlight("😀 **b** é");
        let bold = h.iter().find(|s| s.style == Bold).unwrap();
        assert_eq!((bold.start, bold.end), (3, 8));
        let symbols: Vec<_> = h
            .iter()
            .filter(|s| s.style == Symbol)
            .map(|s| (s.start, s.end))
            .collect();
        assert_eq!(symbols, vec![(3, 5), (6, 8)]);
    }

    #[test]
    fn emphasis() {
        assert!(has("a **bold** b", "**bold**", Bold));
        assert!(has("a *it* b", "*it*", Italic));
        assert!(has("a _it_ b", "_it_", Italic));
        assert!(has("~~gone~~", "~~gone~~", Strike));
        let s = spans("***both***");
        assert!(s.contains(&("**both**".into(), Bold)));
        assert!(s.contains(&("***both***".into(), Italic)));
        assert_eq!(s.iter().filter(|(_, st)| *st == Symbol).count(), 4);
    }

    #[test]
    fn emphasis_needs_flanking() {
        assert!(highlight("snake_case_name").is_empty());
        assert!(highlight("a ** b ** c").is_empty());
        assert!(highlight("~single~").is_empty());
        assert!(highlight("**").is_empty());
        assert!(
            highlight("\\*not italic\\*")
                .iter()
                .all(|s| s.style == Symbol)
        );
    }

    #[test]
    fn code() {
        let s = spans("x `*a*` y");
        assert!(s.contains(&("`*a*`".into(), Code)));
        assert!(!s.iter().any(|(_, st)| *st == Italic));
        assert!(has("``a ` b``", "``a ` b``", Code));
        assert!(highlight("`unclosed").is_empty());
    }

    #[test]
    fn fenced_code() {
        let text = "a\n```rust\nlet *x* = 1;\n\n```\n*b*";
        let s = spans(text);
        assert!(s.contains(&("```rust".into(), Code)));
        assert!(s.contains(&("```".into(), Symbol)));
        assert!(s.contains(&("let *x* = 1;".into(), Code)));
        assert!(s.contains(&("*b*".into(), Italic)));
        assert!(!s.contains(&("*x*".into(), Italic)));
        // An unclosed fence runs to the end.
        assert!(has("```\n# no heading", "# no heading", Code));
    }

    #[test]
    fn lists_and_checkboxes() {
        assert!(has("- item", "-", ListMarker));
        assert!(has("  * item", "*", ListMarker));
        assert!(has("12. item", "12.", ListMarker));
        assert!(highlight("-not a list").is_empty());
        let s = spans("- [x] done *it*");
        assert!(s.contains(&("- [x]".into(), Checkbox)));
        assert!(s.contains(&("- [x] done *it*".into(), Checked)));
        assert!(s.contains(&("*it*".into(), Italic)));
        assert!(has("- [ ] todo", "- [ ]", Checkbox));
        assert!(!has("- [ ] todo", "- [ ] todo", Checked));
        assert!(has("☑ done", "☑ done", Checked));
        assert!(has("☐ todo", "☐", Checkbox));
    }

    #[test]
    fn links() {
        let s = spans("see [Whisp](https://x.org/a_b_c) now");
        assert!(s.contains(&("[".into(), Symbol)));
        assert!(s.contains(&("Whisp".into(), Link)));
        assert!(s.contains(&("](https://x.org/a_b_c)".into(), Symbol)));
        assert!(!s.iter().any(|(_, st)| *st == Italic));
        assert!(has("![alt](a.png)", "![", Symbol));
        assert!(highlight("[not a link] (x)").is_empty());
    }

    #[test]
    fn bare_urls() {
        assert!(has(
            "go to https://example.com/a_b_c.",
            "https://example.com/a_b_c",
            Link
        ));
        assert!(has(
            "(http://x.org/wiki/A_(b))",
            "http://x.org/wiki/A_(b)",
            Link
        ));
        assert!(highlight("https://").is_empty());
        assert!(highlight("xhttps://a.b").is_empty());
    }

    #[test]
    fn quotes_comments_rules() {
        let s = spans("> quoted **b**");
        assert!(s.contains(&("> quoted **b**".into(), Quote)));
        assert!(s.contains(&("> ".into(), Symbol)));
        assert!(s.contains(&("**b**".into(), Bold)));
        assert!(has("  // note", "  // note", Comment));
        assert!(has("☐ // later", "☐ // later", Comment));
        assert!(has("---", "---", Symbol));
        assert!(has("* * *", "* * *", Symbol));
    }

    #[test]
    fn symbols_come_last() {
        let h = highlight("# **a** [b](c) `d`");
        let first_symbol = h.iter().position(|s| s.style == Symbol).unwrap();
        assert!(h[first_symbol..].iter().all(|s| s.style == Symbol));
        assert_eq!(h[0].style, Heading(1));
    }
}
