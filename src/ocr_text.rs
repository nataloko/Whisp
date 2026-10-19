//! Text from OCR word boxes, keeping indentation: a port of Whisp's
//! `extract_text_from_image` (editor_expansions.py).
//!
//! Whisp also drops Tesseract words below 35% confidence. Windows OCR gives
//! no per word confidence, so that filter is left out.

/// One recognised word: its text and box in pixels.
#[derive(Clone)]
pub struct Word {
    pub text: String,
    pub left: f64,
    pub width: f64,
    pub top: f64,
    pub height: f64,
}

/// Most leading spaces an indented line gets. Whisp stops at 8, which
/// flattens deeply nested code.
const MAX_INDENT: i64 = 40;
/// Character width when there are no words, as in Whisp.
const DEFAULT_CHAR_WIDTH: f64 = 20.0;
/// How far a pair of words may be from a pitch and still agree with it.
const PITCH_TOLERANCE: f64 = 0.12;

/// The text of `lines` (each a list of words, as the OCR grouped them),
/// with each line indented by its distance from the leftmost line. `None`
/// when it does not look like text.
pub fn reconstruct(lines: &[Vec<Word>]) -> Option<String> {
    let lines = visual_lines(lines);
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

    let space = pitch(&lines);
    let min_left = lines
        .iter()
        .map(|words| words[0].1.left)
        .fold(f64::INFINITY, f64::min);

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for words in &lines {
        // One space per character column. Whisp counts a space as 1.2 ink
        // widths and turns an indent of 1 into 0, which turned 4 column
        // indents into 3 and lost single space indents.
        let indent = ((words[0].1.left - min_left) / space).round() as i64;
        let mut line = " ".repeat(indent.clamp(0, MAX_INDENT) as usize);
        let texts: Vec<&str> = words.iter().map(|(text, _)| *text).collect();
        line.push_str(&texts.join(" "));
        out.push(line);
    }
    let text = out.join("\n");
    looks_like_text(&text).then(|| text.trim().to_string())
}

/// Lines as they appear: OCR lines that share a height merged into one,
/// words left to right, lines top to bottom. Windows OCR returns pieces of
/// one line (a list marker and its text, words far apart) as separate
/// lines, in an order that is not always the reading order.
fn visual_lines(lines: &[Vec<Word>]) -> Vec<Vec<Word>> {
    let mut groups: Vec<(f64, f64, Vec<Word>)> = lines
        .iter()
        .filter(|words| !words.is_empty())
        .map(|words| {
            let top = words.iter().map(|w| w.top).fold(f64::INFINITY, f64::min);
            let bottom = words
                .iter()
                .map(|w| w.top + w.height)
                .fold(f64::NEG_INFINITY, f64::max);
            (top, bottom, words.clone())
        })
        .collect();
    groups.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64, Vec<Word>)> = Vec::new();
    for (top, bottom, words) in groups {
        if let Some(last) = merged.last_mut() {
            let overlap = last.1.min(bottom) - last.0.max(top);
            if overlap >= 0.5 * (last.1 - last.0).min(bottom - top) {
                last.0 = last.0.min(top);
                last.1 = last.1.max(bottom);
                last.2.extend(words);
                continue;
            }
        }
        merged.push((top, bottom, words));
    }
    merged
        .into_iter()
        .map(|(_, _, mut words)| {
            words.sort_by(|a, b| a.left.total_cmp(&b.left));
            words
        })
        .collect()
}

/// The width of one character column. Neighbouring words on a line are
/// usually one space apart, so the distance between their starts divided
/// by the first word's characters plus one is the pitch, including the
/// side bearings that a word's ink width leaves out. OCR may also split a
/// word at punctuation (no space between the pieces) and code may align
/// with several spaces, so each pair suggests a pitch for no space and for
/// one space, and the suggestion most pairs agree with wins. Falls back to
/// the median ink width per character.
fn pitch(lines: &[Vec<(&str, &Word)>]) -> f64 {
    let pairs: Vec<(f64, f64)> = lines
        .iter()
        .flat_map(|words| words.windows(2))
        .map(|pair| {
            let distance = pair[1].1.left - pair[0].1.left;
            (distance, pair[0].0.chars().count() as f64)
        })
        .filter(|&(distance, _)| distance > 0.0)
        .collect();
    // The column count for `pitch` that a pair agrees with, if any.
    let columns = |pitch: f64, (distance, chars): (f64, f64)| {
        [chars, chars + 1.0]
            .into_iter()
            .find(|&n| (distance / n - pitch).abs() <= pitch * PITCH_TOLERANCE)
    };
    let best = pairs
        .iter()
        .flat_map(|&(distance, chars)| [distance / chars, distance / (chars + 1.0)])
        .max_by_key(|&p| {
            pairs
                .iter()
                .filter(|&&pair| columns(p, pair).is_some())
                .count()
        });
    if let Some(best) = best {
        let agreeing: Vec<f64> = pairs
            .iter()
            .filter_map(|&pair| columns(best, pair).map(|n| pair.0 / n))
            .collect();
        if let Some(pitch) = median(agreeing) {
            return pitch;
        }
    }
    let widths = lines
        .iter()
        .flatten()
        .map(|(text, w)| w.width / text.chars().count() as f64)
        .collect();
    median(widths).unwrap_or(DEFAULT_CHAR_WIDTH)
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
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

    /// A line of words in a monospace font on text row `row`: `columns` is
    /// each word's starting column, every character 10 px wide, rows 30 px
    /// apart and 20 px high.
    fn line(row: usize, words: &[(&str, usize)]) -> Vec<Word> {
        words
            .iter()
            .map(|&(text, column)| Word {
                text: text.to_string(),
                left: 50.0 + column as f64 * 10.0,
                width: text.chars().count() as f64 * 10.0,
                top: 40.0 + row as f64 * 30.0,
                height: 20.0,
            })
            .collect()
    }

    #[test]
    fn keeps_code_indentation() {
        let lines = [
            line(0, &[("fn", 0), ("main()", 3), ("{", 10)]),
            line(1, &[("let", 4), ("x", 8), ("=", 10), ("1;", 12)]),
            line(2, &[("}", 0)]),
        ];
        assert_eq!(
            reconstruct(&lines).unwrap(),
            "fn main() {\n    let x = 1;\n}"
        );
    }

    #[test]
    fn pitch_comes_from_word_starts_not_ink() {
        // Ink 8 px per character in 10 px columns, as glyphs leave side
        // bearings: the ink width alone would make 40 px five spaces.
        let mut lines = vec![
            line(0, &[("def", 0), ("total", 4)]),
            line(1, &[("for", 4), ("item", 8), ("in", 13), ("items:", 16)]),
            line(2, &[("result", 8), ("+=", 15), ("price", 18)]),
        ];
        for word in lines.iter_mut().flatten() {
            word.width = word.text.chars().count() as f64 * 8.0;
        }
        assert_eq!(
            reconstruct(&lines).unwrap(),
            "def total\n    for item in items:\n        result += price"
        );
    }

    #[test]
    fn pitch_survives_punctuation_splits_and_alignment() {
        // Words split at punctuation (no space) and aligned with several
        // spaces; most pairs still agree on 10 px.
        let lines = [
            line(
                0,
                &[
                    ("print", 0),
                    ("(", 5),
                    ("item", 6),
                    (".", 10),
                    ("price", 11),
                    (")", 16),
                ],
            ),
            line(1, &[("x", 4), ("=", 10), ("1", 12)]),
            line(2, &[("long_name", 4), ("=", 14), ("2", 16)]),
        ];
        assert_eq!(
            reconstruct(&lines).unwrap(),
            "print ( item . price )\n    x = 1\n    long_name = 2"
        );
    }

    #[test]
    fn keeps_single_spaces_and_caps_large_indents() {
        let lines = [
            line(0, &[("base", 0)]),
            line(1, &[("one", 1)]),
            line(2, &[("far", 60)]),
        ];
        let far = format!("{}far", " ".repeat(MAX_INDENT as usize));
        assert_eq!(reconstruct(&lines).unwrap(), format!("base\n one\n{far}"));
    }

    #[test]
    fn rounds_to_the_nearest_column() {
        // A first glyph's ink may start a little right of its column.
        let mut lines = vec![
            line(0, &[("abcd", 0)]),
            line(1, &[("efgh", 3)]),
            line(2, &[("ijkl", 0)]),
        ];
        lines[1][0].left += 4.0;
        assert_eq!(reconstruct(&lines).unwrap(), "abcd\n   efgh\nijkl");
        lines[1][0].left += 2.0;
        assert_eq!(reconstruct(&lines).unwrap(), "abcd\n    efgh\nijkl");
    }

    #[test]
    fn indent_is_relative_to_the_leftmost_line() {
        let lines = [line(0, &[("indented", 8)]), line(1, &[("left", 4)])];
        // The first line is 40 px right of the leftmost: 4 spaces, but the
        // result is stripped like Whisp's, which removes them.
        assert_eq!(reconstruct(&lines).unwrap(), "indented\nleft");
        let lines = [line(2, &[("left", 4)]), line(3, &[("indented", 8)])];
        assert_eq!(reconstruct(&lines).unwrap(), "left\n    indented");
    }

    #[test]
    fn falls_back_to_ink_width_without_word_pairs() {
        // One word per line: no pairs, so the median ink width (10) counts.
        let mut lines = vec![
            line(0, &[("aaaa", 0)]),
            line(1, &[("bbbb", 5)]),
            line(2, &[("cccc", 0)]),
        ];
        lines[2][0].width = 500.0;
        assert_eq!(reconstruct(&lines).unwrap(), "aaaa\n     bbbb\ncccc");
    }

    #[test]
    fn merges_pieces_of_one_line_and_orders_lines() {
        // The OCR returned row 2 before row 1, and row 1 in two pieces with
        // its list marker last.
        let lines = [
            line(0, &[("-", 0), ("dash", 2), ("item", 7)]),
            line(2, &[("plus", 2), ("item", 7)]),
            line(1, &[("star", 2), ("item", 7)]),
            line(1, &[("*", 0)]),
        ];
        assert_eq!(
            reconstruct(&lines).unwrap(),
            "- dash item\n* star item\n  plus item"
        );
    }

    #[test]
    fn slightly_offset_pieces_still_merge() {
        // A piece 6 px lower (taller punctuation, a different baseline)
        // overlaps the rest of its line by more than half.
        let mut lines = vec![line(0, &[("total", 0)]), line(0, &[("(items):", 5)])];
        lines[1][0].top += 6.0;
        assert_eq!(reconstruct(&lines).unwrap(), "total (items):");
    }

    #[test]
    fn empty_words_and_lines_are_skipped() {
        let mut lines = vec![
            line(0, &[(" ", 0), ("hello", 2)]),
            vec![],
            line(1, &[("world", 2)]),
        ];
        lines[0][0].text = "  ".into();
        assert_eq!(reconstruct(&lines).unwrap(), "hello\nworld");
    }

    #[test]
    fn rejects_text_that_is_not_text() {
        assert_eq!(reconstruct(&[]), None);
        assert_eq!(
            reconstruct(&[line(0, &[("a", 0), ("b", 2), ("c", 4)])]),
            None
        );
        assert_eq!(reconstruct(&[line(1, &[("ab", 0)])]), None);
        assert_eq!(reconstruct(&[line(2, &[("-", 0), ("—", 2)])]), None);
        assert_eq!(
            reconstruct(&[line(3, &[("ab", 0), ("c", 3)])]).unwrap(),
            "ab c"
        );
        assert_eq!(reconstruct(&[line(4, &[("x1", 0), ("é", 3)])]), None);
    }
}
