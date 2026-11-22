//! Hand-written lexer.

use super::currency;

/// A lexical token. Words (`sqrt`, `km`, `of`, `x`) are all `Ident`; the parser decides.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// The value and its source text (for error messages).
    Num(f64, String),
    /// `17:42`: one or two digits, a colon and two digits, as written (`25:00` too; the
    /// parser decides whether it is a time), with the source text.
    Clock(u8, u8, String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
    Percent,
    Assign,
    Comma,
    /// `#ff8800`: the hex digits of a colour, without the `#` (only from [`lex_colors`]).
    Hex(String),
}

impl Tok {
    /// How the token is quoted in an error message.
    pub fn describe(&self) -> String {
        match self {
            Tok::Num(_, text) | Tok::Clock(_, _, text) => text.clone(),
            Tok::Ident(s) => s.clone(),
            Tok::Plus => "+".into(),
            Tok::Minus => "-".into(),
            Tok::Star => "*".into(),
            Tok::Slash => "/".into(),
            Tok::Caret => "^".into(),
            Tok::LParen => "(".into(),
            Tok::RParen => ")".into(),
            Tok::Percent => "%".into(),
            Tok::Assign => "=".into(),
            Tok::Comma => ",".into(),
            Tok::Hex(digits) => format!("#{digits}"),
        }
    }
}

/// Splits one line into tokens. A `#` starts a comment that runs to the end of the line.
pub fn lex(line: &str) -> Result<Vec<Tok>, String> {
    lex_with(line, false)
}

/// As [`lex`], but a `#` followed by exactly 3, 4, 6 or 8 hex digits and then no letter,
/// digit or `_` is a colour ([`Tok::Hex`]); any other `#` still starts a comment.
pub fn lex_colors(line: &str) -> Result<Vec<Tok>, String> {
    lex_with(line, true)
}

fn lex_with(line: &str, colors: bool) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = line.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '#' => match hex_color(&chars, i).filter(|_| colors) {
                Some(end) => {
                    toks.push(Tok::Hex(chars[i + 1..end].iter().collect()));
                    i = end;
                }
                None => break,
            },
            ',' => push(&mut toks, Tok::Comma, &mut i),
            c if c.is_whitespace() => i += 1,
            '+' => push(&mut toks, Tok::Plus, &mut i),
            '-' => push(&mut toks, Tok::Minus, &mut i),
            '*' => push(&mut toks, Tok::Star, &mut i),
            '/' => push(&mut toks, Tok::Slash, &mut i),
            '^' => push(&mut toks, Tok::Caret, &mut i),
            '(' => push(&mut toks, Tok::LParen, &mut i),
            ')' => push(&mut toks, Tok::RParen, &mut i),
            '%' => push(&mut toks, Tok::Percent, &mut i),
            '=' => push(&mut toks, Tok::Assign, &mut i),
            c if c.is_ascii_digit() && clock(&chars, i).is_some() => {
                let (hour, minute, end) = clock(&chars, i).expect("checked by the guard");
                toks.push(Tok::Clock(hour, minute, chars[i..end].iter().collect()));
                i = end;
            }
            c if c.is_ascii_digit() || (c == '.' && next_is_digit(&chars, i)) => {
                let (n, end) = match radix_literal(&chars, i)? {
                    Some(found) => found,
                    None => number(&chars, i)?,
                };
                toks.push(Tok::Num(n, chars[i..end].iter().collect()));
                i = end;
            }
            c if c.is_alphabetic() || c == '_' || c == '°' => {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                // `L/100km` written with no spaces is one unit, fuel economy (with spaces it
                // stays volume over length).
                if matches!(chars[start..i], ['l' | 'L']) && word_at(&chars, i, "/100km") {
                    i += "/100km".len();
                    toks.push(Tok::Ident("L/100km".into()));
                    continue;
                }
                toks.push(Tok::Ident(chars[start..i].iter().collect()));
            }
            c if currency::is_symbol(c.encode_utf8(&mut [0; 4])) => {
                push(&mut toks, Tok::Ident(c.into()), &mut i);
            }
            c => return Err(format!("unexpected character '{c}'")),
        }
    }
    Ok(toks)
}

/// Whether `word` is written at `start` and no letter, digit or `_` follows it.
fn word_at(chars: &[char], start: usize, word: &str) -> bool {
    let end = start + word.chars().count();
    chars
        .get(start..end)
        .is_some_and(|c| c.iter().copied().eq(word.chars()))
        && !chars
            .get(end)
            .is_some_and(|&c| c.is_alphanumeric() || c == '_')
}

/// The end of a colour `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa` whose `#` is at `start`.
fn hex_color(chars: &[char], start: usize) -> Option<usize> {
    let digits = chars[start + 1..]
        .iter()
        .take_while(|c| c.is_ascii_hexdigit())
        .count();
    let end = start + 1 + digits;
    let glued = chars
        .get(end)
        .is_some_and(|&c| c.is_alphanumeric() || c == '_');
    (matches!(digits, 3 | 4 | 6 | 8) && !glued).then_some(end)
}

fn push(toks: &mut Vec<Tok>, tok: Tok, i: &mut usize) {
    toks.push(tok);
    *i += 1;
}

/// Reads `\d{1,2}:\d{2}` at `start`, not followed by another digit: the hour, the minute
/// and the index after it.
fn clock(chars: &[char], start: usize) -> Option<(u8, u8, usize)> {
    let digit = |i: usize| chars.get(i).and_then(|c| c.to_digit(10)).map(|d| d as u8);
    let (hour, colon) = match (digit(start)?, digit(start + 1)) {
        (tens, Some(ones)) => (tens * 10 + ones, start + 2),
        (ones, None) => (ones, start + 1),
    };
    if chars.get(colon) != Some(&':') || digit(colon + 3).is_some() {
        return None;
    }
    let minute = digit(colon + 1)? * 10 + digit(colon + 2)?;
    Some((hour, minute, colon + 3))
}

fn next_is_digit(chars: &[char], i: usize) -> bool {
    chars.get(i + 1).is_some_and(char::is_ascii_digit)
}

/// Reads `0x…`, `0b…` or `0o…` (at least one digit) starting at `start`. Returns `None` when
/// the text is not one, so that `0x` stays `0` and the word `x`.
fn radix_literal(chars: &[char], start: usize) -> Result<Option<(f64, usize)>, String> {
    let base = match (chars[start], chars.get(start + 1)) {
        ('0', Some('x')) => 16,
        ('0', Some('b')) => 2,
        ('0', Some('o')) => 8,
        _ => return Ok(None),
    };
    let mut i = start + 2;
    let mut value = 0_u64;
    while let Some(digit) = chars.get(i).and_then(|c| c.to_digit(base)) {
        // At most 2^53 before this step, so it cannot overflow a u64.
        value = value * u64::from(base) + u64::from(digit);
        if value > MAX_EXACT {
            return Err("too large".into());
        }
        i += 1;
    }
    Ok((i > start + 2).then_some((value as f64, i)))
}

/// The largest whole number a based literal may hold: 2^53, where `f64` stops being exact.
const MAX_EXACT: u64 = 1 << 53;

/// Reads a number starting at `start`: digits with `_` grouping between digits, an optional
/// fraction and an optional exponent. Returns the value and the index after it.
fn number(chars: &[char], start: usize) -> Result<(f64, usize), String> {
    let mut text = String::new();
    let mut i = start;
    digits(chars, &mut i, &mut text);
    if chars.get(i) == Some(&'.') {
        text.push('.');
        i += 1;
        digits(chars, &mut i, &mut text);
    }
    // An exponent needs digits after `e` (and an optional sign); otherwise the `e` is left
    // for the parser, which rejects `2e` like any other number glued to a word.
    if matches!(chars.get(i), Some('e' | 'E')) {
        let mut j = i + 1;
        let sign = matches!(chars.get(j), Some('+' | '-'));
        if sign {
            j += 1;
        }
        if chars.get(j).is_some_and(char::is_ascii_digit) {
            text.push('e');
            if sign {
                text.push(chars[i + 1]);
            }
            i = j;
            digits(chars, &mut i, &mut text);
        }
    }
    // A huge literal parses to infinity, and a huge exponent too.
    match text.parse::<f64>() {
        Ok(n) if n.is_finite() => Ok((n, i)),
        Ok(_) => Err("too large".into()),
        Err(_) => Err(format!("bad number '{text}'")),
    }
}

/// Appends a run of digits, allowing a single `_` between two digits.
fn digits(chars: &[char], i: &mut usize, text: &mut String) {
    while let Some(&c) = chars.get(*i) {
        if c.is_ascii_digit() {
            text.push(c);
            *i += 1;
        } else if c == '_'
            && chars.get(*i + 1).is_some_and(char::is_ascii_digit)
            && chars
                .get(i.wrapping_sub(1))
                .is_some_and(char::is_ascii_digit)
        {
            *i += 1;
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(
            lex("1_000.5e-1"),
            Ok(vec![Tok::Num(100.05, "1_000.5e-1".into())])
        );
        assert_eq!(lex(".5"), Ok(vec![Tok::Num(0.5, ".5".into())]));
        assert_eq!(lex("5."), Ok(vec![Tok::Num(5.0, "5.".into())]));
        assert_eq!(
            lex("2e"),
            Ok(vec![Tok::Num(2.0, "2".into()), Tok::Ident("e".into())])
        );
        assert_eq!(lex("1e999"), Err("too large".into()));
        assert_eq!(
            lex("1__0"),
            Ok(vec![Tok::Num(1.0, "1".into()), Tok::Ident("__0".into())])
        );
    }

    #[test]
    fn based_literals_and_words() {
        for (text, n) in [("0xff", 255.0), ("0b1010", 10.0), ("0o17", 15.0)] {
            assert_eq!(lex(text), Ok(vec![Tok::Num(n, text.into())]));
        }
        assert_eq!(
            lex("0x"),
            Ok(vec![Tok::Num(0.0, "0".into()), Tok::Ident("x".into())])
        );
        assert_eq!(lex("0x20000000000000").map(|t| t.len()), Ok(1));
        assert_eq!(lex("0x20000000000001"), Err("too large".into()));
        for word in ["°C", "m²", "São"] {
            assert_eq!(lex(word), Ok(vec![Tok::Ident(word.into())]));
        }
    }

    #[test]
    fn clock_times() {
        assert_eq!(lex("17:42"), Ok(vec![Tok::Clock(17, 42, "17:42".into())]));
        assert_eq!(
            lex("5:30pm"),
            Ok(vec![
                Tok::Clock(5, 30, "5:30".into()),
                Tok::Ident("pm".into())
            ])
        );
        assert_eq!(lex("25:00"), Ok(vec![Tok::Clock(25, 0, "25:00".into())]));
        assert_eq!(
            lex("12:345"),
            Err("unexpected character ':'".into()),
            "three minute digits are not a clock"
        );
        assert_eq!(lex("123:45"), Err("unexpected character ':'".into()));
        assert_eq!(lex("5:3"), Err("unexpected character ':'".into()));
    }

    #[test]
    fn comment_and_symbols() {
        assert_eq!(lex("# only"), Ok(vec![]));
        assert_eq!(
            lex("x = (1 + 2) % # c"),
            Ok(vec![
                Tok::Ident("x".into()),
                Tok::Assign,
                Tok::LParen,
                Tok::Num(1.0, "1".into()),
                Tok::Plus,
                Tok::Num(2.0, "2".into()),
                Tok::RParen,
                Tok::Percent,
            ])
        );
        assert_eq!(lex("@"), Err("unexpected character '@'".into()));
        assert_eq!(lex("$"), Ok(vec![Tok::Ident("$".into())]));
    }
}
