//! Styled RTF for a note, built from its text and Markdown spans, so RichEdit
//! can load a note in one stream instead of one call per span.

use std::fmt::Write as _;
use std::ops::Range;

use crate::markdown::{Span, Style};

pub type Rgb = (u8, u8, u8);

/// Colours and sizes, mirroring the per-span formats in the editor.
pub struct Look {
    pub text: Rgb,
    pub symbol: Rgb,
    pub heading: Rgb,
    pub link: Rgb,
    pub list: Rgb,
    pub checked: Rgb,
    pub comment: Rgb,
    pub quote: Rgb,
    pub code_background: Rgb,
    /// The font's face name.
    pub face: String,
    /// Base font size in half points.
    pub size: u32,
    /// H1 to H3 sizes in half points.
    pub heading_sizes: [u32; 3],
    /// Line spacing in twentieths of a line: 20 is single spacing.
    pub line_spacing: u32,
}

impl Look {
    /// The colour for a colour table index, as used in `Attr::color`.
    pub fn color(&self, index: u8) -> Rgb {
        match index {
            SYMBOL => self.symbol,
            HEADING => self.heading,
            LINK => self.link,
            LIST => self.list,
            CHECKED => self.checked,
            COMMENT => self.comment,
            QUOTE => self.quote,
            CODE_BACKGROUND => self.code_background,
            _ => self.text,
        }
    }
}

// Colour table indices (\cfN, \highlightN), in `Look` field order.
const TEXT: u8 = 1;
const SYMBOL: u8 = 2;
const HEADING: u8 = 3;
const LINK: u8 = 4;
const LIST: u8 = 5;
const CHECKED: u8 = 6;
const COMMENT: u8 = 7;
const QUOTE: u8 = 8;
const CODE_BACKGROUND: u8 = 9;

/// Character attributes of one UTF-16 unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Attr {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub underline: bool,
    pub code: bool,
    /// Colour table index.
    pub color: u8,
    /// 0 for the base size, 1 to 3 for H1 to H3.
    pub heading: u8,
    /// A hideable marker while the hide symbols mode hides it.
    pub hidden: bool,
}

impl Default for Attr {
    fn default() -> Self {
        Attr {
            bold: false,
            italic: false,
            strike: false,
            underline: false,
            code: false,
            color: TEXT,
            heading: 0,
            hidden: false,
        }
    }
}

impl Attr {
    /// Sets what `style` sets, as EM_SETCHARFORMAT with the editor's masks
    /// does. With `hide`, a `Style::Marker` is also hidden.
    fn apply(&mut self, style: Style, hide: bool) {
        match style {
            Style::Heading(level) => {
                self.bold = true;
                self.color = HEADING;
                if level <= 3 {
                    self.heading = level;
                }
            }
            Style::ListTitle => {
                self.bold = true;
                self.color = LIST;
            }
            Style::Underline => self.underline = true,
            Style::Bold => self.bold = true,
            Style::Italic => self.italic = true,
            Style::Strike => self.strike = true,
            Style::Code => self.code = true,
            Style::ListMarker => {
                self.bold = true;
                self.color = LIST;
            }
            Style::Checkbox => {
                self.bold = true;
                self.color = CHECKED;
            }
            Style::Checked => {
                self.strike = true;
                self.color = CHECKED;
            }
            Style::Link => {
                self.underline = true;
                self.color = LINK;
            }
            Style::Quote => self.color = QUOTE,
            Style::Comment => {
                self.italic = true;
                self.color = COMMENT;
            }
            Style::Symbol => self.color = SYMBOL,
            Style::Marker => {
                self.color = SYMBOL;
                if hide {
                    self.hidden = true;
                }
            }
        }
    }
}

/// Flattens spans, applied in order, into non overlapping runs that cover
/// all `len` units: (start, end, attributes). With `hide`, hideable
/// markers are hidden; without it the runs look as if they were symbols.
pub fn runs(len: usize, spans: &[Span], hide: bool) -> Vec<(usize, usize, Attr)> {
    flatten(len, spans, |_| hide)
}

/// `runs`, asking `hide` for each span whether its markers are hidden.
fn flatten(len: usize, spans: &[Span], hide: impl Fn(&Span) -> bool) -> Vec<(usize, usize, Attr)> {
    let mut attrs = vec![Attr::default(); len];
    for span in spans {
        let end = span.end.min(len);
        let hide = hide(span);
        for attr in &mut attrs[span.start.min(end)..end] {
            attr.apply(span.style, hide);
        }
    }
    let mut runs = Vec::new();
    let mut start = 0;
    for i in 1..=len {
        if i == len || attrs[i] != attrs[start] {
            runs.push((start, i, attrs[start]));
            start = i;
        }
    }
    runs
}

/// An RTF document for `text` (with `\n` line breaks) styled by `spans`
/// (from `markdown::highlight` on the same text). With `hide`, the hide
/// symbols mode is on: hideable markers are hidden (`\v`), except those
/// within that range of units, the caret's line.
pub fn document(text: &str, spans: &[Span], look: &Look, hide: Option<Range<usize>>) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut out = String::with_capacity(units.len() + units.len() / 4 + 512);
    out.push_str(r"{\rtf1\ansi\deff0\uc1{\fonttbl{\f0\fnil ");
    // A semicolon would end the name; no real font has one.
    let face: Vec<u16> = look
        .face
        .encode_utf16()
        .filter(|&u| u >= 0x20 && u != u16::from(b';'))
        .collect();
    escape(&face, &mut out);
    out.push_str(r";}}{\colortbl;");
    for (r, g, b) in [
        look.text,
        look.symbol,
        look.heading,
        look.link,
        look.list,
        look.checked,
        look.comment,
        look.quote,
        look.code_background,
    ] {
        let _ = write!(out, r"\red{r}\green{g}\blue{b};");
    }
    // Every paragraph gets the line spacing as a multiple of single spacing
    // (\slmult1), in 240ths of a line.
    let _ = write!(
        out,
        r"}}\pard\sl{}\slmult1\plain\f0\fs{}\cf{TEXT} ",
        look.line_spacing * 12,
        look.size
    );

    let hidden = |span: &Span| {
        hide.as_ref()
            .is_some_and(|line| span.start < line.start || span.end > line.end)
    };
    for (start, end, attr) in flatten(units.len(), spans, hidden) {
        let styled = attr != Attr::default();
        if styled {
            out.push('{');
            for (on, word) in [
                (attr.bold, r"\b"),
                (attr.italic, r"\i"),
                (attr.strike, r"\strike"),
                (attr.underline, r"\ul"),
                (attr.hidden, r"\v"),
            ] {
                if on {
                    out.push_str(word);
                }
            }
            if attr.color != TEXT {
                let _ = write!(out, r"\cf{}", attr.color);
            }
            if attr.code {
                let _ = write!(out, r"\highlight{CODE_BACKGROUND}");
            }
            if attr.heading > 0 {
                let _ = write!(
                    out,
                    r"\fs{}",
                    look.heading_sizes[usize::from(attr.heading) - 1]
                );
            }
            // Ends the last control word; RTF drops this space.
            out.push(' ');
        }
        escape(&units[start..end], &mut out);
        if styled {
            out.push('}');
        }
    }
    out.push('}');
    out
}

/// Text as RTF. Control words end with a space, which RTF drops.
fn escape(units: &[u16], out: &mut String) {
    for &u in units {
        match u {
            0x5c => out.push_str(r"\\"),
            0x7b => out.push_str(r"\{"),
            0x7d => out.push_str(r"\}"),
            0x0a => out.push_str(r"\par "),
            0x09 => out.push_str(r"\tab "),
            0x20..=0x7e => out.push(char::from(u as u8)),
            // Everything else, including each half of a surrogate pair, as a
            // signed 16 bit \u with a one character fallback (\uc1).
            _ => {
                let _ = write!(out, r"\u{}?", u as i16);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown;

    fn look() -> Look {
        Look {
            text: (224, 224, 224),
            symbol: (110, 118, 129),
            heading: (208, 135, 112),
            link: (129, 161, 193),
            list: (180, 142, 173),
            checked: (128, 128, 128),
            comment: (136, 136, 136),
            quote: (157, 165, 180),
            code_background: (50, 50, 50),
            face: "Consolas".to_string(),
            size: 22,
            heading_sizes: [30, 26, 24],
            line_spacing: 24,
        }
    }

    fn rtf(text: &str) -> String {
        document(text, &markdown::highlight(text), &look(), None)
    }

    /// The document with hidden markers, except on the line `reveal`.
    fn hidden_rtf(text: &str, reveal: Range<usize>) -> String {
        document(text, &markdown::highlight(text), &look(), Some(reveal))
    }

    /// The plain text of an RTF document: a small reader for what `document`
    /// writes, skipping the font and colour tables.
    fn extract(rtf: &str) -> String {
        let c: Vec<char> = rtf.chars().collect();
        let mut units: Vec<u16> = Vec::new();
        let mut skip_depth = None;
        let mut depth = 0;
        let mut i = 0;
        while i < c.len() {
            match c[i] {
                '{' => {
                    depth += 1;
                    i += 1;
                }
                '}' => {
                    if skip_depth == Some(depth) {
                        skip_depth = None;
                    }
                    depth -= 1;
                    i += 1;
                }
                '\\' if c[i + 1].is_ascii_alphabetic() => {
                    let mut j = i + 1;
                    while c[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    let word: String = c[i + 1..j].iter().collect();
                    let k = j;
                    if c[j] == '-' {
                        j += 1;
                    }
                    while c[j].is_ascii_digit() {
                        j += 1;
                    }
                    let param: String = c[k..j].iter().collect();
                    if c[j] == ' ' {
                        j += 1;
                    }
                    if skip_depth.is_none() {
                        match word.as_str() {
                            "fonttbl" | "colortbl" => skip_depth = Some(depth),
                            "par" => units.push(0x0a),
                            "tab" => units.push(0x09),
                            "u" => {
                                units.push(param.parse::<i16>().unwrap() as u16);
                                j += 1; // the \uc1 fallback character
                            }
                            _ => {}
                        }
                    }
                    i = j;
                }
                '\\' => {
                    if skip_depth.is_none() {
                        units.push(c[i + 1] as u16);
                    }
                    i += 2;
                }
                '\r' | '\n' => i += 1,
                ch => {
                    if skip_depth.is_none() {
                        let mut buf = [0u16; 2];
                        units.extend_from_slice(ch.encode_utf16(&mut buf));
                    }
                    i += 1;
                }
            }
        }
        assert_eq!(depth, 0, "unbalanced groups");
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn escapes_rtf_characters() {
        let doc = rtf(r"a\b{c}");
        assert!(doc.contains(r"a\\b\{c\}"));
        assert_eq!(extract(&doc), r"a\b{c}");
    }

    #[test]
    fn non_ascii_as_signed_units() {
        let doc = rtf("é😀日");
        assert!(doc.contains(r"\u233?\u-10179?\u-8704?\u26085?"), "{doc}");
        assert!(doc.is_ascii());
    }

    #[test]
    fn empty_text() {
        let doc = rtf("");
        assert!(doc.ends_with(r"\cf1 }"));
        assert_eq!(extract(&doc), "");
    }

    #[test]
    fn font_and_line_spacing_in_the_header() {
        let doc = rtf("a\nb");
        assert!(doc.contains(r"{\fonttbl{\f0\fnil Consolas;}}"), "{doc}");
        assert!(
            doc.contains(r"\pard\sl288\slmult1\plain\f0\fs22\cf1 "),
            "{doc}"
        );
        // Line spacing 2.0 and a face that needs escaping.
        let look = Look {
            face: "Fancy {Font}; \\ é".to_string(),
            line_spacing: 40,
            ..look()
        };
        let doc = document("", &[], &look, None);
        assert!(
            doc.contains(r"{\fonttbl{\f0\fnil Fancy \{Font\} \\ \u233?;}}"),
            "{doc}"
        );
        assert!(doc.contains(r"\sl480\slmult1"), "{doc}");
        assert!(doc.is_ascii());
        assert_eq!(extract(&doc), "");
    }

    #[test]
    fn trailing_newline_is_kept() {
        assert!(rtf("a\n").ends_with(r"a\par }"));
        assert!(rtf("a").ends_with(r"a}"));
        assert_eq!(extract(&rtf("a\n\n")), "a\n\n");
    }

    #[test]
    fn every_style_on_one_line() {
        let doc = rtf("# h **b** *i* ~~s~~ `c` [l](u) - x");
        for word in [
            r"\b",
            r"\i",
            r"\strike",
            r"\ul",
            r"\cf2",
            r"\cf3",
            r"\cf4",
            r"\highlight9",
            r"\fs30",
        ] {
            assert!(doc.contains(word), "missing {word} in {doc}");
        }
        let doc = rtf("- [x] done\n> q\n// c\n* l");
        for word in [r"\cf5", r"\cf6", r"\cf7", r"\cf8"] {
            assert!(doc.contains(word), "missing {word} in {doc}");
        }
        assert!(doc.contains(r"\red224\green224\blue224;"));
        assert!(doc.contains(r"\red50\green50\blue50;}"));
    }

    #[test]
    fn overlapping_spans_flatten_in_order() {
        // "***both***": italic over all, bold inside, symbols on the stars.
        let text = "***both***";
        let runs = runs(text.len(), &markdown::highlight(text), false);
        let bounds: Vec<_> = runs.iter().map(|r| (r.0, r.1)).collect();
        assert_eq!(bounds, [(0, 1), (1, 3), (3, 7), (7, 9), (9, 10)]);
        let both = runs[2].2;
        assert!(both.bold && both.italic && both.color == TEXT);
        let outer = runs[0].2;
        assert!(outer.italic && !outer.bold && outer.color == SYMBOL);
        assert!(runs[1].2.bold && runs[1].2.color == SYMBOL);
        // A heading's symbol keeps the heading size and boldness.
        let runs = super::runs(4, &markdown::highlight("# ab"), false);
        assert_eq!(runs[0].2.heading, 1);
        assert!(runs[0].2.bold && runs[0].2.color == SYMBOL);
        assert_eq!(runs[1].2.color, HEADING);
    }

    #[test]
    fn runs_cover_everything_once() {
        let text = "x **a** y\n\n`c`";
        let runs = runs(text.len(), &markdown::highlight(text), false);
        assert_eq!(runs.first().unwrap().0, 0);
        assert_eq!(runs.last().unwrap().1, text.len());
        assert!(
            runs.windows(2)
                .all(|w| w[0].1 == w[1].0 && w[0].2 != w[1].2)
        );
        assert!(super::runs(0, &[], false).is_empty());
    }

    #[test]
    fn markers_look_like_symbols_when_shown() {
        let text = "# h **b** _u_ `c` [l](u) ![i](p) > q";
        let spans = markdown::highlight(text);
        let shown = runs(text.len(), &spans, false);
        assert!(shown.iter().all(|r| !r.2.hidden));
        // The same runs as if every marker were a plain symbol.
        let as_symbols: Vec<Span> = spans
            .iter()
            .map(|s| Span {
                style: if s.style == Style::Marker {
                    Style::Symbol
                } else {
                    s.style
                },
                ..*s
            })
            .collect();
        assert_eq!(shown, runs(text.len(), &as_symbols, false));
        assert!(!rtf(text).contains(r"\v"));
    }

    #[test]
    fn hidden_markers() {
        let text = "a **b** [l](u)";
        let hidden: Vec<String> = runs(text.len(), &markdown::highlight(text), true)
            .into_iter()
            .filter(|r| r.2.hidden)
            .map(|r| text[r.0..r.1].to_string())
            .collect();
        assert_eq!(hidden, ["**", "**", "[", "](u)"]);
        // Hidden markers keep the symbol colour, for when they show again.
        let runs = runs(4, &markdown::highlight("# ab"), true);
        assert!(runs[0].2.hidden && runs[0].2.color == SYMBOL && runs[0].2.heading == 1);
        assert!(!runs[1].2.hidden);
        // Other symbols never hide.
        let text = "> - [x] `` ``` ``\n---\n![i](p)";
        let all = super::runs(text.len(), &markdown::highlight(text), true);
        let hidden: Vec<&str> = all
            .iter()
            .filter(|r| r.2.hidden)
            .map(|r| &text[r.0..r.1])
            .collect();
        assert_eq!(hidden, ["``", "``", "[", "](p)"]);
    }

    #[test]
    fn document_hides_all_but_the_caret_line() {
        let text = "**a**\n**b**";
        // The caret on the second line, units 6 to 11.
        let doc = hidden_rtf(text, 6..11);
        assert_eq!(doc.matches(r"\v").count(), 2, "{doc}");
        assert!(doc.contains(r"{\b\v\cf2 **}{\b a}{\b\v\cf2 **}"), "{doc}");
        assert!(doc.contains(r"{\b\cf2 **}{\b b}{\b\cf2 **}"), "{doc}");
        // The whole note on the caret line: nothing hidden.
        assert!(!hidden_rtf(text, 0..11).contains(r"\v"));
        // An empty last line reveals nothing.
        assert_eq!(hidden_rtf("# h\n", 4..4).matches(r"\v").count(), 1);
        for text in ["# T\n**b** [l](u) `c`\n_u_", "", "a\n\n"] {
            assert_eq!(extract(&hidden_rtf(text, 0..0)), text);
        }
    }

    #[test]
    fn round_trips_plain_text() {
        let mut sample = String::new();
        for n in 1..=5 {
            let _ = writeln!(
                sample,
                "- item {n} with **bold** and `code` and a [link](https://example.com/{n}) and *italic*"
            );
        }
        let cases = [
            sample.as_str(),
            "```rust\nfn main() { println!(\"{}\", 1); }\n```\n",
            "# Heading\n\n\tindented\t tab\n  spaced  \n",
            r"\par \u123? {\b not bold} \\ \",
            "é😀 日本語 עברית العربية 👩‍👩‍👧",
            "- [x] done\n☑ done\n☐ todo\n> quote\n// comment\n---\n",
            "1. one\n2. two\n\n\n",
            " leading space\nnext",
            "\u{1}\u{7f}\u{a0}\u{feff}\u{fffd}",
        ];
        for text in cases {
            assert_eq!(extract(&rtf(text)), text, "round trip of {text:?}");
        }
    }
}
