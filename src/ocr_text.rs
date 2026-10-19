//! Text from OCR word boxes, keeping indentation: a port of Whisp's
//! `extract_text_from_image` (editor_expansions.py).
//!
//! Whisp also drops Tesseract words below 35% confidence. Windows OCR gives
//! no per word confidence, so that filter is left out.

/// One recognised word: its text and horizontal position in pixels.
pub struct Word {
    pub text: String,
    pub left: f64,
    pub width: f64,
}

/// Most leading spaces an indented line gets.
const MAX_INDENT: i64 = 8;
/// A space is this many median character widths.
const SPACE_WIDTHS: f64 = 1.2;
/// Median character width when there are no words, as in Whisp.
const DEFAULT_CHAR_WIDTH: f64 = 20.0;

/// The text of `lines` (each a list of words, in reading order), with each
/// line indented by its distance from the leftmost line. `None` when it
/// does not look like text.
pub fn reconstruct(lines: &[Vec<Word>]) -> Option<String> {
    let lines: Vec<Vec<(&str, &Word)>> = lines
        .iter()
        .map(|words| {
            words
                .iter()
                .map(|w| (w.text.trim(), w))
                .filter(|(text, _)| !text.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|words| !words.is_empty())
        .collect();

    let mut char_widths: Vec<f64> = lines
        .iter()
        .flatten()
        .map(|(text, w)| w.width / text.chars().count() as f64)
        .collect();
    char_widths.sort_by(f64::total_cmp);
    // The upper median, as Whisp takes it.
    let median = char_widths
        .get(char_widths.len() / 2)
        .copied()
        .unwrap_or(DEFAULT_CHAR_WIDTH);
    let space = median * SPACE_WIDTHS;
    let min_left = lines
        .iter()
        .map(|words| words[0].1.left)
        .fold(f64::INFINITY, f64::min);

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for words in &lines {
        // Python's round() rounds halves to even.
        let mut indent = ((words[0].1.left - min_left) / space).round_ties_even() as i64;
        if indent > MAX_INDENT {
            indent = MAX_INDENT;
        } else if indent == 1 {
            indent = 0;
        }
        let mut line = " ".repeat(indent.max(0) as usize);
        let texts: Vec<&str> = words.iter().map(|(text, _)| *text).collect();
        line.push_str(&texts.join(" "));
        out.push(line);
    }
    let text = out.join("\n");
    looks_like_text(&text).then(|| text.trim().to_string())
}

/// Whisp's check: a run of two ASCII letters or digits somewhere, and at
/// least three of them in all.
fn looks_like_text(text: &str) -> bool {
    let mut run = 0;
    let mut has_run = false;
    let mut total = 0;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            run += 1;
            total += 1;
            has_run |= run >= 2;
        } else {
            run = 0;
        }
    }
    !text.trim().is_empty() && has_run && total >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line of words in a monospace font: `columns` is each word's
    /// starting column, every character 10 px wide.
    fn line(words: &[(&str, usize)]) -> Vec<Word> {
        words
            .iter()
            .map(|&(text, column)| Word {
                text: text.to_string(),
                left: 50.0 + column as f64 * 10.0,
                width: text.chars().count() as f64 * 10.0,
            })
            .collect()
    }

    #[test]
    fn keeps_code_indentation() {
        let lines = [
            line(&[("fn", 0), ("main()", 3), ("{", 10)]),
            line(&[("let", 4), ("x", 8), ("=", 10), ("1;", 12)]),
            line(&[("}", 0)]),
        ];
        // A space is 12 px, so 40 px of indent is round(3.33) = 3 spaces.
        assert_eq!(
            reconstruct(&lines).unwrap(),
            "fn main() {\n   let x = 1;\n}"
        );
    }

    #[test]
    fn indent_of_one_is_dropped_and_large_indents_capped() {
        let lines = [
            line(&[("base", 0)]),
            line(&[("one", 1)]),
            line(&[("far", 40)]),
        ];
        assert_eq!(reconstruct(&lines).unwrap(), "base\none\n        far");
    }

    #[test]
    fn rounds_halves_to_even() {
        // Median char width 10, space 12: 30 px is 2.5 spaces, rounded to 2;
        // 42 px is 3.5, rounded to 4.
        let mut lines = vec![
            line(&[("abcd", 0)]),
            line(&[("efgh", 3)]),
            line(&[("ijkl", 0)]),
        ];
        assert_eq!(reconstruct(&lines).unwrap(), "abcd\n  efgh\nijkl");
        lines[1][0].left = 50.0 + 42.0;
        assert_eq!(reconstruct(&lines).unwrap(), "abcd\n    efgh\nijkl");
    }

    #[test]
    fn indent_is_relative_to_the_leftmost_line() {
        let lines = [line(&[("indented", 8)]), line(&[("left", 4)])];
        // The first line is 40 px right of the leftmost: 3 spaces, but the
        // result is stripped like Whisp's, which removes them.
        assert_eq!(reconstruct(&lines).unwrap(), "indented\nleft");
        let lines = [line(&[("left", 4)]), line(&[("indented", 8)])];
        assert_eq!(reconstruct(&lines).unwrap(), "left\n   indented");
    }

    #[test]
    fn median_char_width_drives_the_space() {
        // Wide words would make the mean misleading; the median stays 10.
        let mut lines = vec![line(&[("aaaa", 0)]), line(&[("bbbb", 5)])];
        lines[0].push(Word {
            text: "W".into(),
            left: 200.0,
            width: 500.0,
        });
        assert_eq!(reconstruct(&lines).unwrap(), "aaaa W\n    bbbb");
    }

    #[test]
    fn empty_words_and_lines_are_skipped() {
        let mut lines = vec![
            line(&[(" ", 0), ("hello", 2)]),
            vec![],
            line(&[("world", 2)]),
        ];
        lines[0][0].text = "  ".into();
        assert_eq!(reconstruct(&lines).unwrap(), "hello\nworld");
    }

    #[test]
    fn rejects_text_that_is_not_text() {
        assert_eq!(reconstruct(&[]), None);
        assert_eq!(reconstruct(&[line(&[("a", 0), ("b", 2), ("c", 4)])]), None);
        assert_eq!(reconstruct(&[line(&[("ab", 0)])]), None);
        assert_eq!(reconstruct(&[line(&[("-", 0), ("—", 2)])]), None);
        assert_eq!(
            reconstruct(&[line(&[("ab", 0), ("c", 3)])]).unwrap(),
            "ab c"
        );
        assert_eq!(reconstruct(&[line(&[("x1", 0), ("é", 3)])]), None);
    }
}
