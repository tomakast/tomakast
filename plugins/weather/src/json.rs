//! A small JSON reader (RFC 8259) for the Open-Meteo answers. A copy of calc's
//! (`plugins/calc/src/json.rs`): each plugin is its own crate, and no parser crate keeps the
//! wasm small.

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// The longest body [`parse`] reads.
const MAX_BYTES: usize = 64 * 1024;
/// The deepest nesting of arrays and objects [`parse`] reads.
const MAX_DEPTH: usize = 16;

/// RFC 8259, with string escapes (`\uXXXX` and surrogate pairs included). `None` for invalid
/// JSON, trailing text, nesting deeper than 16, or a body over 64 KiB.
pub fn parse(text: &str) -> Option<Json> {
    if text.len() > MAX_BYTES {
        return None;
    }
    let mut reader = Reader { text, at: 0 };
    let value = reader.value(0)?;
    reader.skip_space();
    (reader.at == text.len()).then_some(value)
}

impl Json {
    /// The value of member `key` of an object (the first, if it repeats); `None` for a value
    /// that is not an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Reads `text` from byte `at` on.
struct Reader<'a> {
    text: &'a str,
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    /// Steps over `byte` if it is next.
    fn eat(&mut self, byte: u8) -> bool {
        let next = self.peek() == Some(byte);
        self.at += usize::from(next);
        next
    }

    /// Steps over `word` if it is next.
    fn word(&mut self, word: &str) -> bool {
        let next = self.text[self.at..].starts_with(word);
        if next {
            self.at += word.len();
        }
        next
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    /// Steps over a run of ASCII digits; whether there was one.
    fn digits(&mut self) -> bool {
        let start = self.at;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.at += 1;
        }
        self.at > start
    }

    /// A value inside `depth` arrays and objects.
    fn value(&mut self, depth: usize) -> Option<Json> {
        self.skip_space();
        match self.peek()? {
            b'{' | b'[' if depth == MAX_DEPTH => None,
            b'{' => self.object(depth + 1),
            b'[' => self.array(depth + 1),
            b'"' => self.string().map(Json::Str),
            b't' => self.word("true").then_some(Json::Bool(true)),
            b'f' => self.word("false").then_some(Json::Bool(false)),
            b'n' => self.word("null").then_some(Json::Null),
            _ => self.number().map(Json::Num),
        }
    }

    fn array(&mut self, depth: usize) -> Option<Json> {
        self.at += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.eat(b']') {
            return Some(Json::Arr(items));
        }
        loop {
            items.push(self.value(depth)?);
            self.skip_space();
            if self.eat(b']') {
                return Some(Json::Arr(items));
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }

    fn object(&mut self, depth: usize) -> Option<Json> {
        self.at += 1;
        let mut members = Vec::new();
        self.skip_space();
        if self.eat(b'}') {
            return Some(Json::Obj(members));
        }
        loop {
            self.skip_space();
            if self.peek() != Some(b'"') {
                return None;
            }
            let key = self.string()?;
            self.skip_space();
            if !self.eat(b':') {
                return None;
            }
            members.push((key, self.value(depth)?));
            self.skip_space();
            if self.eat(b'}') {
                return Some(Json::Obj(members));
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }

    /// A string, its opening quote next.
    fn string(&mut self) -> Option<String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            // Plain text up to the next quote, backslash or control character: all ASCII, so
            // `at` stays on a character boundary.
            let start = self.at;
            while self
                .peek()
                .is_some_and(|b| b != b'"' && b != b'\\' && b >= 0x20)
            {
                self.at += 1;
            }
            out.push_str(&self.text[start..self.at]);
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.at += 1;
                    out.push(self.escape()?);
                }
                _ => return None,
            }
        }
    }

    /// The character of an escape, its backslash read.
    fn escape(&mut self) -> Option<char> {
        let byte = self.peek()?;
        self.at += 1;
        Some(match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let unit = self.hex4()?;
                if (0xD800..0xDC00).contains(&unit) {
                    // A high surrogate: its low half must follow.
                    if !self.word("\\u") {
                        return None;
                    }
                    let low = self.hex4()?;
                    if !(0xDC00..0xE000).contains(&low) {
                        return None;
                    }
                    char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))?
                } else {
                    // A lone low surrogate is no character.
                    char::from_u32(unit)?
                }
            }
            _ => return None,
        })
    }

    /// Four hex digits.
    fn hex4(&mut self) -> Option<u32> {
        let digits = self.text.get(self.at..self.at + 4)?;
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        self.at += 4;
        u32::from_str_radix(digits, 16).ok()
    }

    /// A number as RFC 8259 writes it: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`.
    fn number(&mut self) -> Option<f64> {
        let start = self.at;
        self.eat(b'-');
        if !self.eat(b'0') && !self.digits() {
            return None;
        }
        if self.eat(b'.') && !self.digits() {
            return None;
        }
        if self.eat(b'e') || self.eat(b'E') {
            let _ = self.eat(b'+') || self.eat(b'-');
            if !self.digits() {
                return None;
            }
        }
        self.text[start..self.at].parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORECAST_FIXTURE: &str = include_str!("../tests/fixtures/forecast-berlin.json");
    const GEOCODING_FIXTURE: &str = include_str!("../tests/fixtures/geocoding-moscow-ru.json");

    #[test]
    fn reads_json() {
        let json =
            parse(r#"{"a":1,"b":{"c":-2.5e-3},"d":[true,null],"e":"x\"\u00e9\ud83d\ude00"}"#)
                .expect("valid JSON");
        assert_eq!(
            json,
            Json::Obj(vec![
                ("a".into(), Json::Num(1.0)),
                (
                    "b".into(),
                    Json::Obj(vec![("c".into(), Json::Num(-2.5e-3))])
                ),
                ("d".into(), Json::Arr(vec![Json::Bool(true), Json::Null])),
                ("e".into(), Json::Str("x\"é😀".into())),
            ])
        );
        assert_eq!(json.get("e"), Some(&Json::Str("x\"é😀".into())));
        assert_eq!(
            json.get("b").and_then(|b| b.get("c")),
            Some(&Json::Num(-2.5e-3))
        );
        assert_eq!(json.get("z"), None);
        assert_eq!(json.get("a").and_then(|a| a.get("a")), None);
        for bad in [
            "",
            "{",
            "{} x",
            "{\"a\":01}",
            "[\"\\ud800\"]",
            &"[".repeat(17),
            // Beyond the brief's list: what `f64::from_str` would take and RFC 8259 does not.
            "[1.]",
            "[.5]",
            "[+1]",
            "[inf]",
            "[NaN]",
            "[-]",
            "[1e]",
            "[\"\\udc00\\ud800\"]",
            "[\"a\u{1}\"]",
            "[\"\\x\"]",
            "[\"\\u+0ab\"]",
            "[1,]",
            "{\"a\" 1}",
            "{1:2}",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
        let deepest = format!("{}{}", "[".repeat(16), "]".repeat(16));
        assert!(parse(&deepest).is_some());
        let at_limit = format!("\"{}\"", "x".repeat(64 * 1024 - 2));
        assert!(parse(&at_limit).is_some());
        let over = format!("\"{}\"", "x".repeat(64 * 1024 - 1));
        assert_eq!(parse(&over), None);
        assert!(parse(FORECAST_FIXTURE).is_some());
        assert!(parse(GEOCODING_FIXTURE).is_some());
    }
}
