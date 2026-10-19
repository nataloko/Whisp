//! The `::` expansions, ported from Whisp's `editor_expansions.py`:
//! inserts (`::today`, `::uuid`, ...) replace the `::word` token, text
//! commands (`::uppercase`, `::sort_lines_alpha`, ...) rewrite the note.

/// What the expansions need from the system.
pub trait Env {
    /// The local date `days` from today, in the user's short date format.
    fn date(&mut self, days: i64) -> String;
    /// The local time without seconds, in the user's format.
    fn time(&mut self) -> String;
    /// Random bytes. On Windows these come from the OS generator.
    fn fill(&mut self, bytes: &mut [u8]);
}

/// The suggestion list, with Whisp's descriptions, in Whisp's order.
/// `::paste` (auto paste) and `::random_wiki` (opens a browser) are left out.
pub const SUGGESTIONS: &[(&str, &str)] = &[
    ("::count", "Insert word count of current note"),
    ("::today", "Current date"),
    ("::today(5)", "Date offset (+/- days)"),
    ("::tomorrow", "Tomorrow's date"),
    ("::yesterday", "Yesterday's date"),
    ("::date", "Current date"),
    ("::date(5)", "Date offset (+/- days)"),
    ("::timestamp", "Current date and time"),
    ("::now", "Current time"),
    ("::time", "Current time"),
    ("::random(int,20)", "Random numbers"),
    ("::random(str,20)", "Random letters"),
    ("::random(alnum,20)", "Random alphanumeric"),
    ("::random_quote", "Insert a random inspirational quote"),
    ("::magic8ball", "Ask the Magic 8-Ball"),
    ("::lorem", "Insert standard placeholder text"),
    ("::password", "Generate a secure 16-char password"),
    ("::uuid", "Generate a standard developer UUID"),
    ("::roll(20)", "Roll a 20-sided die"),
    ("::roll(d20)", "Roll a d20 die"),
    ("::roll(4d6)", "Roll 4 d6 dice"),
    ("::uppercase", "Convert document to uppercase"),
    ("::lowercase", "Convert document to lowercase"),
    ("::sentence_case", "Convert document to sentence case"),
    ("::title_case", "Convert document to title case"),
    ("::capitalize_first", "Capitalize first letter"),
    ("::remove_quotes", "Strip surrounding quotes"),
    ("::append(text)", "Add text to the end of every line"),
    ("::prepend(text)", "Add text to the beginning of every line"),
    ("::replace(old,new)", "Find and replace text"),
    ("::sort_lines_alpha", "Sort lines alphabetically"),
    ("::sort_lines_number", "Sort lines by first number"),
    ("::sort_lines_reverse", "Reverse line order"),
    ("::remove_lines_empty", "Remove empty lines"),
    ("::remove_lines_with(text)", "Remove lines containing text"),
    (
        "::remove_lines_without(text)",
        "Remove lines not containing text",
    ),
    ("::keep_lines_with(text)", "Keep only lines containing text"),
    (
        "::keep_lines_without(text)",
        "Keep only lines not containing text",
    ),
    ("::trim_each_whitespace", "Trim whitespace from all lines"),
    ("::dedupe_lines", "Remove duplicate lines"),
    ("::get_dupes", "Find and group duplicate lines"),
    ("::commas_to_list", "Convert comma-separated items to list"),
    ("::commas_to(delim)", "Convert commas to custom delimiter"),
    ("::lines_to_commas", "Convert lines to comma-separated"),
    ("::lines_to(delim)", "Convert lines to custom delimiter"),
    ("::checked_to_bottom", "Move checked items to bottom"),
    ("::remove_checked", "Remove all checked items"),
];

const LOREM: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.";

const MAGIC_8_BALL: &[&str] = &[
    "It is certain.",
    "It is decidedly so.",
    "Without a doubt.",
    "Yes definitely.",
    "You may rely on it.",
    "As I see it, yes.",
    "Most likely.",
    "Outlook good.",
    "Yes.",
    "Signs point to yes.",
    "Reply hazy, try again.",
    "Ask again later.",
    "Better not tell you now.",
    "Cannot predict now.",
    "Concentrate and ask again.",
    "Don't count on it.",
    "My reply is no.",
    "My sources say no.",
    "Outlook not so good.",
    "Very doubtful.",
];

const QUOTES: &[&str] = &[
    "The only way to do great work is to love what you do. - Steve Jobs",
    "Simplicity is the ultimate sophistication. - Leonardo da Vinci",
    "Make it work, make it right, make it fast. - Kent Beck",
    "Talk is cheap. Show me the code. - Linus Torvalds",
    "Software is like sex: it's better when it's free. - Linus Torvalds",
    "First, solve the problem. Then, write the code. - John Johnson",
    "Experience is the name everyone gives to their mistakes. - Oscar Wilde",
    "In order to be irreplaceable, one must always be different. - Coco Chanel",
    "Fix the cause, not the symptom. - Steve Maguire",
    "Before software can be reusable it first has to be usable. - Ralph Johnson",
    "Truth can only be found in one place: the code. - Robert C. Martin",
    "It's not a bug. It's an undocumented feature! - Anonymous",
];

const DIGITS: &str = "0123456789";
const LETTERS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const PASSWORD_SYMBOLS: &str = "!@#$%^&*";
const PASSWORD_LENGTH: usize = 16;
const MAX_RANDOM_LENGTH: u64 = 1000;
const MAX_DICE: i64 = 100;

/// Commands that rewrite the note and take no arguments.
const TEXT_COMMANDS: &[&str] = &[
    "uppercase",
    "lowercase",
    "sentence_case",
    "title_case",
    "capitalize_first",
    "remove_quotes",
    "sort_lines_alpha",
    "sort_lines_number",
    "sort_lines_reverse",
    "remove_lines_empty",
    "trim_each_whitespace",
    "dedupe_lines",
    "get_dupes",
    "commas_to_list",
    "lines_to_commas",
    "checked_to_bottom",
    "remove_checked",
];

/// Commands that rewrite the note and take one argument.
const TEXT_COMMANDS_WITH_ARG: &[&str] = &[
    "append",
    "prepend",
    "remove_lines_with",
    "remove_lines_without",
    "keep_lines_with",
    "keep_lines_without",
    "commas_to",
    "lines_to",
];

/// The suggestions for the token before the caret: those starting with it,
/// ignoring case, while it starts with `::`.
pub fn suggestions(word: &str) -> Vec<(&'static str, &'static str)> {
    if !word.starts_with("::") {
        return Vec::new();
    }
    let word = word.to_lowercase();
    SUGGESTIONS
        .iter()
        .copied()
        .filter(|(command, _)| command.starts_with(&word))
        .collect()
}

fn is_separator(unit: u16) -> bool {
    // Whisp's space, newline and tab; RichEdit's \r and \v are newlines.
    matches!(unit, 0x20 | 0x0a | 0x0d | 0x09 | 0x0b)
}

/// The token around `caret`, as Whisp finds it: back to the previous and on
/// to the next space, line break or tab. Returns its start and end.
pub fn token_bounds(units: &[u16], caret: usize) -> (usize, usize) {
    let caret = caret.min(units.len());
    let start = units[..caret]
        .iter()
        .rposition(|&u| is_separator(u))
        .map_or(0, |i| i + 1);
    let end = units[caret..]
        .iter()
        .position(|&u| is_separator(u))
        .map_or(units.len(), |i| caret + i);
    (start, end)
}

/// The part of `old` that `new` changes: the common prefix and suffix are
/// left alone. Returns the start, the end in `old` and the end in `new`.
pub fn diff(old: &[u16], new: &[u16]) -> (usize, usize, usize) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    (prefix, old.len() - suffix, new.len() - suffix)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Expansion {
    /// Replace the token with this, followed by the key's character.
    Insert(String),
    /// Remove the token and give the note this text; `None` when the
    /// command leaves the text as it is.
    Text(Option<String>),
}

/// The expansion for `token`. `rest` is the note's text without the token,
/// with `\n` line breaks. `None` when the token is not a command.
pub fn expand(token: &str, rest: &str, env: &mut dyn Env) -> Option<Expansion> {
    let body = token.strip_prefix("::")?;
    let (name, args) = match body.find('(') {
        Some(open) => {
            let inner = body[open + 1..].strip_suffix(')')?;
            (body[..open].to_lowercase(), Some(inner))
        }
        None => (body.to_lowercase(), None),
    };
    let insert = |s: String| Some(Expansion::Insert(s));
    match (name.as_str(), args) {
        ("today" | "date" | "tomorrow" | "yesterday" | "timestamp", args) => {
            let mut days = match args {
                Some(a) => offset(a)?,
                None => 0,
            };
            match name.as_str() {
                "tomorrow" => days += 1,
                "yesterday" => days -= 1,
                _ => {}
            }
            let date = env.date(days);
            if name == "timestamp" {
                let time = env.time();
                return insert(format!("{date} {time}"));
            }
            insert(date)
        }
        ("count" | "words" | "word_count", None) => {
            insert(rest.split_whitespace().count().to_string())
        }
        ("time" | "now", None) => insert(env.time()),
        ("roll", Some(expr)) if !expr.is_empty() => insert(roll(&expr.to_lowercase(), env)?),
        ("random", Some(args)) => {
            let (kind, length) = args.split_once(',')?;
            let length = length.trim_start();
            if length.is_empty() || !length.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let length = length
                .parse::<u64>()
                .unwrap_or(u64::MAX)
                .min(MAX_RANDOM_LENGTH);
            let alphabet: String = match kind.to_lowercase().as_str() {
                "int" => DIGITS.into(),
                "str" => LETTERS.into(),
                "alnum" => format!("{LETTERS}{DIGITS}"),
                _ => return None,
            };
            insert(pick_chars(&alphabet, length as usize, env))
        }
        ("password", None) => {
            let alphabet = format!("{LETTERS}{DIGITS}{PASSWORD_SYMBOLS}");
            insert(pick_chars(&alphabet, PASSWORD_LENGTH, env))
        }
        ("uuid", None) => insert(uuid(env)),
        ("lorem", None) => insert(LOREM.into()),
        ("magic8ball", None) => insert(pick(MAGIC_8_BALL, env).into()),
        ("random_quote", None) => insert(pick(QUOTES, env).into()),
        (command, None) if TEXT_COMMANDS.contains(&command) => Some(Expansion::Text(changed(
            rest,
            text_command(command, &[], rest),
        ))),
        (command, Some(arg)) if TEXT_COMMANDS_WITH_ARG.contains(&command) => Some(Expansion::Text(
            changed(rest, text_command(command, &[arg], rest)),
        )),
        ("replace", Some(args)) => {
            let (old, new) = args.split_once(',')?;
            Some(Expansion::Text(changed(
                rest,
                text_command("replace", &[old, new], rest),
            )))
        }
        _ => None,
    }
}

fn changed(old: &str, new: String) -> Option<String> {
    (new != old).then_some(new)
}

/// `[-+]*\d+` as Python's int() reads it: one optional sign.
fn offset(text: &str) -> Option<i64> {
    let digits = text.trim_start_matches(['+', '-']);
    if digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || text.len() - digits.len() > 1
    {
        return None;
    }
    text.parse().ok()
}

/// Python's int(): surrounding whitespace and one sign allowed.
fn python_int(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}

/// `::roll(20)`, `::roll(d20)`, `::roll(4d6)`: the total, or `None` for an
/// invalid roll (as Whisp, which then leaves the token).
fn roll(expr: &str, env: &mut dyn Env) -> Option<String> {
    let (dice, sides) = match expr.contains('d') {
        true => {
            let mut parts = expr.split('d');
            let dice = parts.next()?;
            let dice = if dice.is_empty() {
                1
            } else {
                python_int(dice)?
            };
            (dice, python_int(parts.next()?)?)
        }
        false => (1, python_int(expr)?),
    };
    if sides < 1 || !(1..=MAX_DICE).contains(&dice) {
        return None;
    }
    let total: u128 = (0..dice)
        .map(|_| u128::from(below(env, sides as u64)) + 1)
        .sum();
    Some(total.to_string())
}

/// A uniform random number below `n` (n > 0).
pub fn below(env: &mut dyn Env, n: u64) -> u64 {
    // Rejection sampling avoids the bias of a plain modulo.
    let limit = u64::MAX - u64::MAX % n;
    loop {
        let mut bytes = [0u8; 8];
        env.fill(&mut bytes);
        let x = u64::from_le_bytes(bytes);
        if x < limit {
            return x % n;
        }
    }
}

fn pick<'a>(items: &[&'a str], env: &mut dyn Env) -> &'a str {
    items[below(env, items.len() as u64) as usize]
}

fn pick_chars(alphabet: &str, length: usize, env: &mut dyn Env) -> String {
    let chars: Vec<char> = alphabet.chars().collect();
    (0..length)
        .map(|_| chars[below(env, chars.len() as u64) as usize])
        .collect()
}

/// A random (version 4) UUID, lowercase with hyphens.
fn uuid(env: &mut dyn Env) -> String {
    let mut b = [0u8; 16];
    env.fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// Python's str.strip(): whitespace at both ends.
fn strip(s: &str) -> &str {
    s.trim()
}

/// Python's str.capitalize(): the first character upper, the rest lower.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

/// Python's str.title(): a letter after a cased letter goes lower case,
/// any other letter upper case.
fn title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut previous_cased = false;
    for c in s.chars() {
        if previous_cased {
            out.extend(c.to_lowercase());
        } else {
            out.extend(c.to_uppercase());
        }
        previous_cased = c.is_uppercase() || c.is_lowercase();
    }
    out
}

/// `re.split(r'(?<=[.!?])\s+', text)`.
fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    let mut previous = None;
    while let Some((i, c)) = chars.next() {
        if c.is_whitespace() && matches!(previous, Some('.' | '!' | '?')) {
            out.push(&text[start..i]);
            let mut end = i + c.len_utf8();
            while let Some(&(j, d)) = chars.peek() {
                if !d.is_whitespace() {
                    break;
                }
                end = j + d.len_utf8();
                chars.next();
            }
            start = end;
            previous = None;
            continue;
        }
        previous = Some(c);
    }
    out.push(&text[start..]);
    out
}

/// Whisp's heading pattern for sorting: `^\s*#{1,6}\s`.
fn is_heading(line: &str) -> bool {
    let rest = line.trim_start();
    let hashes = rest.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&hashes) && rest[hashes..].starts_with(char::is_whitespace)
}

/// Sorts each run of lines between headings with `sort`.
fn sort_sections(lines: Vec<&str>, sort: impl Fn(&mut Vec<&str>)) -> String {
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut chunk: Vec<&str> = Vec::new();
    for line in lines {
        if is_heading(line) {
            sort(&mut chunk);
            out.append(&mut chunk);
            out.push(line);
        } else {
            chunk.push(line);
        }
    }
    sort(&mut chunk);
    out.append(&mut chunk);
    out.join("\n")
}

/// The first number in a line (`\d+`), or infinity.
fn first_number(line: &str) -> f64 {
    let start = line.find(|c: char| c.is_ascii_digit());
    start.map_or(f64::INFINITY, |s| {
        let digits: String = line[s..].chars().take_while(char::is_ascii_digit).collect();
        digits.parse().unwrap_or(f64::INFINITY)
    })
}

fn is_checked(line: &str) -> bool {
    line.trim_start().starts_with('\u{2611}')
        || line.to_lowercase().contains("- [x]")
        || line.contains("- [X]")
}

/// Whisp's `execute_text_command` on `text` (the note without the token).
pub fn text_command(command: &str, args: &[&str], text: &str) -> String {
    let arg = args.first().copied().unwrap_or("");
    let lines = || text.split('\n');
    let stripped_lines = || text.trim_matches('\n').split('\n');
    let join = |v: Vec<String>| v.join("\n");
    match command {
        "uppercase" => text.to_uppercase(),
        "lowercase" => text.to_lowercase(),
        "title_case" => title(text),
        "sentence_case" => {
            let joined = split_sentences(text)
                .into_iter()
                .filter(|s| !s.is_empty())
                .map(capitalize)
                .collect::<Vec<_>>()
                .join(" ");
            if joined.is_empty() && !text.is_empty() {
                capitalize(text)
            } else {
                joined
            }
        }
        "capitalize_first" => match text.find(|c: char| c.is_ascii_alphabetic()) {
            Some(i) => format!(
                "{}{}{}",
                &text[..i],
                text[i..i + 1].to_uppercase(),
                &text[i + 1..]
            ),
            None => text.to_string(),
        },
        "remove_quotes" => text
            .chars()
            .filter(|c| {
                !matches!(
                    c,
                    '"' | '\'' | '\u{201c}' | '\u{201d}' | '\u{2018}' | '\u{2019}'
                )
            })
            .collect(),
        "append" => join(lines().map(|l| format!("{l}{arg}")).collect()),
        "prepend" => join(lines().map(|l| format!("{arg}{l}")).collect()),
        "replace" => text.replace(arg, args.get(1).copied().unwrap_or("")),
        "sort_lines_alpha" => sort_sections(stripped_lines().collect(), |c| {
            c.sort_by_key(|x| (strip(x).is_empty(), x.to_lowercase()))
        }),
        "sort_lines_number" => sort_sections(stripped_lines().collect(), |c| {
            c.sort_by(|a, b| {
                (strip(a).is_empty(), first_number(a))
                    .partial_cmp(&(strip(b).is_empty(), first_number(b)))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        }),
        "sort_lines_reverse" => sort_sections(stripped_lines().collect(), |c| c.reverse()),
        "remove_lines_empty" => join(
            lines()
                .filter(|l| !strip(l).is_empty())
                .map(String::from)
                .collect(),
        ),
        "remove_lines_with" | "keep_lines_without" => {
            let search = arg.to_lowercase();
            join(
                stripped_lines()
                    .filter(|l| !l.to_lowercase().contains(&search))
                    .map(String::from)
                    .collect(),
            )
        }
        "remove_lines_without" | "keep_lines_with" => {
            let search = arg.to_lowercase();
            join(
                stripped_lines()
                    .filter(|l| l.to_lowercase().contains(&search))
                    .map(String::from)
                    .collect(),
            )
        }
        "trim_each_whitespace" => join(lines().map(|l| strip(l).to_string()).collect()),
        "dedupe_lines" => {
            let mut seen = std::collections::HashSet::new();
            join(
                lines()
                    .filter(|l| seen.insert(*l))
                    .map(String::from)
                    .collect(),
            )
        }
        "get_dupes" => {
            let mut counts: Vec<(&str, usize)> = Vec::new();
            for line in stripped_lines().map(strip).filter(|l| !l.is_empty()) {
                match counts.iter_mut().find(|(l, _)| *l == line) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((line, 1)),
                }
            }
            let dupes: Vec<String> = counts
                .into_iter()
                .filter(|(_, n)| *n > 1)
                .map(|(l, n)| format!("{n}x: {l}"))
                .collect();
            if dupes.is_empty() {
                text.to_string()
            } else {
                format!("{text}\n\n--- Duplicates ---\n{}", dupes.join("\n"))
            }
        }
        "commas_to_list" | "commas_to" => {
            let joined = text.replace('\n', ",");
            let items: Vec<&str> = joined
                .split(',')
                .map(strip)
                .filter(|i| !i.is_empty())
                .collect();
            if command == "commas_to" {
                items.join(arg)
            } else {
                join(items.iter().map(|i| format!("- {i}")).collect())
            }
        }
        "lines_to_commas" | "lines_to" => {
            let items: Vec<&str> = lines().map(strip).filter(|i| !i.is_empty()).collect();
            items.join(if command == "lines_to" { arg } else { ", " })
        }
        "checked_to_bottom" => {
            let (checked, unchecked): (Vec<&str>, Vec<&str>) =
                stripped_lines().partition(|l| is_checked(l));
            [unchecked, checked].concat().join("\n")
        }
        "remove_checked" => stripped_lines()
            .filter(|l| !is_checked(l))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => text.to_string(),
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `(year, month, day)` moved by `days`, and its day of the week
/// (0 is Sunday).
pub fn add_days((year, month, day): (i64, i64, i64), days: i64) -> ((i64, i64, i64), i64) {
    let n = days_from_civil(year, month, day) + days;
    // 1970-01-01 was a Thursday.
    (civil_from_days(n), (n + 4).rem_euclid(7))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed clock and a counting "random" source.
    struct Fake {
        next: u8,
    }

    impl Env for Fake {
        fn date(&mut self, days: i64) -> String {
            let ((y, m, d), _) = add_days((2026, 10, 19), days);
            format!("{d:02}/{m:02}/{y}")
        }
        fn time(&mut self) -> String {
            "14:05".into()
        }
        fn fill(&mut self, bytes: &mut [u8]) {
            for b in bytes {
                *b = self.next;
                self.next = self.next.wrapping_add(1);
            }
        }
    }

    fn run(token: &str, rest: &str) -> Option<Expansion> {
        expand(token, rest, &mut Fake { next: 0 })
    }

    fn ins(token: &str) -> String {
        match run(token, "") {
            Some(Expansion::Insert(s)) => s,
            other => panic!("{token}: {other:?}"),
        }
    }

    fn text(token: &str, rest: &str) -> String {
        match run(token, rest) {
            Some(Expansion::Text(Some(s))) => s,
            Some(Expansion::Text(None)) => rest.to_string(),
            other => panic!("{token}: {other:?}"),
        }
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(ins("::today"), "19/10/2026");
        assert_eq!(ins("::date"), "19/10/2026");
        assert_eq!(ins("::TODAY(5)"), "24/10/2026");
        assert_eq!(ins("::date(-19)"), "30/09/2026");
        assert_eq!(ins("::today(+1)"), "20/10/2026");
        assert_eq!(ins("::tomorrow"), "20/10/2026");
        assert_eq!(ins("::yesterday"), "18/10/2026");
        assert_eq!(ins("::tomorrow(2)"), "22/10/2026");
        assert_eq!(ins("::timestamp"), "19/10/2026 14:05");
        assert_eq!(ins("::now"), "14:05");
        assert_eq!(ins("::time"), "14:05");
        assert_eq!(run("::today(--1)", ""), None);
        assert_eq!(run("::today(x)", ""), None);
        assert_eq!(run("::time(1)", ""), None);
    }

    #[test]
    fn date_arithmetic_across_boundaries() {
        assert_eq!(add_days((2024, 2, 28), 1).0, (2024, 2, 29), "leap year");
        assert_eq!(add_days((2023, 2, 28), 1).0, (2023, 3, 1));
        assert_eq!(
            add_days((1900, 2, 28), 1).0,
            (1900, 3, 1),
            "not a leap year"
        );
        assert_eq!(add_days((2000, 2, 28), 1).0, (2000, 2, 29), "a leap year");
        assert_eq!(add_days((2026, 12, 31), 1).0, (2027, 1, 1));
        assert_eq!(add_days((2027, 1, 1), -1).0, (2026, 12, 31));
        assert_eq!(add_days((2024, 3, 1), -1).0, (2024, 2, 29));
        assert_eq!(add_days((2026, 1, 31), 365).0, (2027, 1, 31));
        assert_eq!(add_days((2026, 10, 19), 0), ((2026, 10, 19), 1), "a Monday");
        assert_eq!(add_days((1970, 1, 1), 0).1, 4, "a Thursday");
    }

    #[test]
    fn count_words() {
        assert_eq!(
            run("::count", "one two\nthree  four").unwrap(),
            Expansion::Insert("4".into())
        );
        assert_eq!(ins("::words"), "0");
        assert_eq!(run("::count(1)", ""), None);
    }

    #[test]
    fn random_inserts() {
        assert_eq!(ins("::random(int,5)").len(), 5);
        assert!(ins("::random(int,5)").chars().all(|c| c.is_ascii_digit()));
        assert!(
            ins("::random(STR,8)")
                .chars()
                .all(|c| c.is_ascii_alphabetic())
        );
        assert!(
            ins("::random(alnum,8)")
                .chars()
                .all(|c| c.is_ascii_alphanumeric())
        );
        assert_eq!(ins("::random(int,5000)").len(), 1000, "capped");
        assert_eq!(run("::random(hex,5)", ""), None);
        assert_eq!(run("::random(int,)", ""), None);
        let password = ins("::password");
        assert_eq!(password.chars().count(), 16);
        assert!(
            password
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "!@#$%^&*".contains(c))
        );
        let id = ins("::uuid");
        assert_eq!(id, "00010203-0405-4607-8809-0a0b0c0d0e0f");
        assert_eq!(ins("::lorem"), LOREM);
        assert!(MAGIC_8_BALL.contains(&ins("::magic8ball").as_str()));
        assert!(QUOTES.contains(&ins("::random_quote").as_str()));
    }

    #[test]
    fn dice() {
        for token in ["::roll(20)", "::roll(d20)", "::roll(1d20)"] {
            let n: u32 = ins(token).parse().unwrap();
            assert!((1..=20).contains(&n), "{token}: {n}");
        }
        let n: u32 = ins("::roll(4d6)").parse().unwrap();
        assert!((4..=24).contains(&n));
        assert_eq!(ins("::roll(1)"), "1");
        for bad in [
            "::roll(0)",
            "::roll(101d6)",
            "::roll(0d6)",
            "::roll(d)",
            "::roll(x)",
            "::roll()",
        ] {
            assert_eq!(run(bad, ""), None, "{bad}");
        }
    }

    #[test]
    fn unbiased_choice_rejects_the_top_of_the_range() {
        /// Gives u64::MAX first, then 5.
        struct Source {
            first: bool,
        }
        impl Env for Source {
            fn date(&mut self, _: i64) -> String {
                String::new()
            }
            fn time(&mut self) -> String {
                String::new()
            }
            fn fill(&mut self, bytes: &mut [u8]) {
                let value: u64 = if self.first { u64::MAX } else { 5 };
                self.first = false;
                bytes.copy_from_slice(&value.to_le_bytes());
            }
        }
        // 2^64 - 1 is a multiple of 3, so u64::MAX is outside the fair range.
        assert_eq!(below(&mut Source { first: true }, 3), 2);
    }

    #[test]
    fn case_commands() {
        assert_eq!(text("::uppercase", "straße é ﬁ"), "STRASSE É FI");
        assert_eq!(text("::lowercase", "ÉCOLE ΟΔΟΣ"), "école οδος");
        assert_eq!(
            text("::title_case", "they're éCOLE 2nd"),
            "They'Re École 2Nd"
        );
        assert_eq!(
            text("::sentence_case", "hello WORLD. how are you?  fine"),
            "Hello world. How are you? Fine"
        );
        assert_eq!(
            text("::sentence_case", "line one.\nline two"),
            "Line one. Line two"
        );
        assert_eq!(text("::capitalize_first", "  42 éa bc"), "  42 éA bc");
        assert_eq!(
            text(
                "::remove_quotes",
                "\"a\" 'b' \u{201c}c\u{201d} \u{2018}d\u{2019}"
            ),
            "a b c d"
        );
    }

    #[test]
    fn line_commands() {
        assert_eq!(text("::append(;)", "a\nb"), "a;\nb;");
        assert_eq!(text("::prepend(- )", "a\nb"), "- a\n- b");
        assert_eq!(text("::replace(a,b,c)", "a-a"), "b,c-b,c");
        assert_eq!(text("::sort_lines_alpha", "b\n\nA\nc\n"), "A\nb\nc\n");
        assert_eq!(
            text("::sort_lines_alpha", "# H\nb\na\n## I\nd\nc"),
            "# H\na\nb\n## I\nc\nd"
        );
        assert_eq!(
            text("::sort_lines_number", "x 10\nno number\ny 9\nz 9.5"),
            "y 9\nz 9.5\nx 10\nno number"
        );
        assert_eq!(text("::sort_lines_reverse", "1\n2\n3\n"), "3\n2\n1");
        assert_eq!(text("::remove_lines_empty", "a\n \n\nb"), "a\nb");
        assert_eq!(text("::remove_lines_with(X)", "ax\nb\nc x"), "b");
        assert_eq!(text("::keep_lines_with(x)", "ax\nb"), "ax");
        assert_eq!(text("::remove_lines_without(x)", "ax\nb"), "ax");
        assert_eq!(text("::keep_lines_without(x)", "ax\nb"), "b");
        assert_eq!(text("::trim_each_whitespace", "  a \n\tb"), "a\nb");
        assert_eq!(text("::dedupe_lines", "a\nb\na\n b"), "a\nb\n b");
        assert_eq!(
            text("::get_dupes", "a\nb\na\n a"),
            "a\nb\na\n a\n\n--- Duplicates ---\n3x: a"
        );
        assert_eq!(run("::get_dupes", "a\nb"), Some(Expansion::Text(None)));
        assert_eq!(text("::commas_to_list", "a, b,,c\nd"), "- a\n- b\n- c\n- d");
        assert_eq!(text("::commas_to( | )", "a,b"), "a | b");
        assert_eq!(text("::lines_to_commas", "a\n\n b "), "a, b");
        assert_eq!(text("::lines_to(;)", "a\nb"), "a;b");
        let list = "\u{2611} done\n\u{2610} todo\n- [x] old\n- [X] older\nplain";
        assert_eq!(
            text("::checked_to_bottom", list),
            "\u{2610} todo\nplain\n\u{2611} done\n- [x] old\n- [X] older"
        );
        assert_eq!(text("::remove_checked", list), "\u{2610} todo\nplain");
    }

    #[test]
    fn unknown_or_excluded_tokens() {
        for token in [
            "::paste",
            "::random_wiki",
            "::nope",
            "today",
            ":today",
            "::append",
            "::uppercase(x)",
        ] {
            assert_eq!(run(token, "a"), None, "{token}");
        }
    }

    #[test]
    fn suggestion_matching() {
        assert_eq!(suggestions("::").len(), SUGGESTIONS.len());
        let s = suggestions("::To");
        assert_eq!(
            s.iter().map(|s| s.0).collect::<Vec<_>>(),
            ["::today", "::today(5)", "::tomorrow"]
        );
        assert!(suggestions("::zzz").is_empty());
        assert!(suggestions("today").is_empty());
        assert!(
            !SUGGESTIONS
                .iter()
                .any(|s| s.0 == "::paste" || s.0 == "::random_wiki")
        );
    }

    #[test]
    fn tokens_and_diffs() {
        let units: Vec<u16> = "a ::to\rb\tc::x y".encode_utf16().collect();
        assert_eq!(token_bounds(&units, 6), (2, 6));
        assert_eq!(token_bounds(&units, 4), (2, 6));
        assert_eq!(token_bounds(&units, 13), (9, 13));
        assert_eq!(token_bounds(&units, 0), (0, 1));
        let old: Vec<u16> = "abcXYdef".encode_utf16().collect();
        let new: Vec<u16> = "abcZdef".encode_utf16().collect();
        assert_eq!(diff(&old, &new), (3, 5, 4));
        let same: Vec<u16> = "aa".encode_utf16().collect();
        let longer: Vec<u16> = "aaa".encode_utf16().collect();
        assert_eq!(diff(&same, &longer), (2, 2, 3));
        assert_eq!(diff(&same, &same), (2, 2, 2));
    }
}
