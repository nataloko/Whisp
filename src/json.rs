//! A small JSON reader and writer, enough for Whisp's `metadata.json`. The
//! writer gives exactly what Python's `json.dumps(value)` gives, so a file
//! that goes through here changes only where it was edited.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number as it was written, so it is never changed.
    Number(String),
    String(String),
    Array(Vec<Value>),
    /// Members in file order.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// The value of `key` if this is an object that has it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Writes the text Python's `json.dumps` writes.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            Value::Number(raw) => out.push_str(raw),
            Value::String(s) => write_string(s, out),
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Value::Object(members) => {
                out.push('{');
                for (i, (key, value)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write_string(key, out);
                    out.push_str(": ");
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// A string as Python writes it with `ensure_ascii`.
fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

/// Parses strict JSON. A leading byte order mark and whitespace around the
/// value are fine; anything after the value is not.
pub fn parse(text: &str) -> Option<Value> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut p = Parser {
        bytes: text.as_bytes(),
        pos: 0,
        depth: 0,
    };
    p.skip_space();
    let value = p.value()?;
    p.skip_space();
    (p.pos == p.bytes.len()).then_some(value)
}

/// Nesting deeper than this is refused rather than risking the stack.
const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        let found = self.peek() == Some(b);
        if found {
            self.pos += 1;
        }
        found
    }

    fn word(&mut self, word: &str, value: Value) -> Option<Value> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Some(value)
        } else {
            None
        }
    }

    fn value(&mut self) -> Option<Value> {
        match self.peek()? {
            b'n' => self.word("null", Value::Null),
            b't' => self.word("true", Value::Bool(true)),
            b'f' => self.word("false", Value::Bool(false)),
            b'"' => self.string().map(Value::String),
            b'[' => self.nested(Self::array),
            b'{' => self.nested(Self::object),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn nested(&mut self, parse: fn(&mut Self) -> Option<Value>) -> Option<Value> {
        if self.depth >= MAX_DEPTH {
            return None;
        }
        self.depth += 1;
        let value = parse(self);
        self.depth -= 1;
        value
    }

    fn array(&mut self) -> Option<Value> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.eat(b']') {
            return Some(Value::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value()?);
            self.skip_space();
            if self.eat(b']') {
                return Some(Value::Array(items));
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }

    fn object(&mut self) -> Option<Value> {
        self.pos += 1;
        let mut members = Vec::new();
        self.skip_space();
        if self.eat(b'}') {
            return Some(Value::Object(members));
        }
        loop {
            self.skip_space();
            if self.peek()? != b'"' {
                return None;
            }
            let key = self.string()?;
            self.skip_space();
            if !self.eat(b':') {
                return None;
            }
            self.skip_space();
            members.push((key, self.value()?));
            self.skip_space();
            if self.eat(b'}') {
                return Some(Value::Object(members));
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.pos;
        self.eat(b'-');
        if self.eat(b'0') {
            // No leading zeros.
        } else if self.digits() == 0 {
            return None;
        }
        if self.eat(b'.') && self.digits() == 0 {
            return None;
        }
        if self.eat(b'e') || self.eat(b'E') {
            let _ = self.eat(b'+') || self.eat(b'-');
            if self.digits() == 0 {
                return None;
            }
        }
        let raw = std::str::from_utf8(&self.bytes[start..self.pos]).ok()?;
        Some(Value::Number(raw.to_string()))
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.bytes.get(self.pos..self.pos + 4)?;
        let mut n = 0;
        for &d in digits {
            n = n * 16 + char::from(d).to_digit(16)?;
        }
        self.pos += 4;
        Some(n)
    }

    fn string(&mut self) -> Option<String> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            // Copy the plain run up to the next quote, backslash or control
            // character in one go; the input is a &str, so it stays UTF-8.
            let start = self.pos;
            while self
                .peek()
                .is_some_and(|b| b != b'"' && b != b'\\' && b >= 0x20)
            {
                self.pos += 1;
            }
            out.push_str(std::str::from_utf8(&self.bytes[start..self.pos]).ok()?);
            match self.peek()? {
                b'"' => {
                    self.pos += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let escape = self.peek()?;
                    self.pos += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode()?),
                        _ => return None,
                    }
                }
                _ => return None,
            }
        }
    }

    /// The character after `\u`, joining a surrogate pair. A lone surrogate
    /// becomes U+FFFD.
    fn unicode(&mut self) -> Option<char> {
        let first = self.hex4()?;
        if (0xd800..0xdc00).contains(&first) && self.bytes[self.pos..].starts_with(b"\\u") {
            let save = self.pos;
            self.pos += 2;
            match self.hex4() {
                Some(second) if (0xdc00..0xe000).contains(&second) => {
                    let code = 0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00);
                    return Some(char::from_u32(code).unwrap_or('\u{fffd}'));
                }
                // Not the other half: leave it to be read on its own.
                _ => self.pos = save,
            }
        }
        Some(char::from_u32(first).unwrap_or('\u{fffd}'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisp_sample_round_trips_byte_for_byte() {
        let text = r#"{"a.md": {"pinned": true, "wysiwyg": false}, "caf\u00e9.md": {}}"#;
        let value = parse(text).unwrap();
        assert_eq!(value.dump(), text);
        let note = value.get("a.md").unwrap();
        assert_eq!(note.get("pinned"), Some(&Value::Bool(true)));
        assert!(value.get("café.md").is_some());
    }

    #[test]
    fn writer_matches_python_dumps() {
        let v = Value::Array(vec![
            Value::Null,
            Value::Number("1.50e3".into()),
            Value::Array(vec![]),
            Value::Object(vec![]),
            Value::String("q\"\\\n\r\t\u{8}\u{c}\u{1}é😀/".into()),
        ]);
        assert_eq!(
            v.dump(),
            r#"[null, 1.50e3, [], {}, "q\"\\\n\r\t\b\f\u0001\u00e9\ud83d\ude00/"]"#
        );
    }

    #[test]
    fn escapes_and_surrogates_round_trip() {
        let value = parse(r#""\ud83d\ude00 \u00e9 \/ \b""#).unwrap();
        assert_eq!(value, Value::String("😀 é / \u{8}".into()));
        let dumped = value.dump();
        assert_eq!(dumped, r#""\ud83d\ude00 \u00e9 / \b""#);
        assert_eq!(parse(&dumped), Some(value));
        // Lone surrogates become U+FFFD.
        assert_eq!(
            parse(r#""\ud800x\udc00""#),
            Some(Value::String("\u{fffd}x\u{fffd}".into()))
        );
    }

    #[test]
    fn tolerates_bom_and_whitespace() {
        assert_eq!(parse("\u{feff} \n[ 1 , 2 ]\r\n"), parse("[1,2]"));
        assert!(parse("\u{feff} {}").is_some());
    }

    #[test]
    fn invalid_inputs_are_none() {
        for bad in [
            "",
            " ",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "{a:1}",
            "{\"a\" 1}",
            "[1 2]",
            "{} x",
            "[] []",
            "'a'",
            "\"abc",
            "\"a\nb\"",
            "\"\\x\"",
            "\"\\u12\"",
            "01",
            "-",
            "1.",
            "1e",
            ".5",
            "tru",
            "nul",
            "+1",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
        assert_eq!(parse(&"[".repeat(1000)), None);
    }

    #[test]
    fn numbers_keep_their_text() {
        let v = parse("[0, -1.5E+10, 12]").unwrap();
        assert_eq!(v.dump(), "[0, -1.5E+10, 12]");
    }
}
