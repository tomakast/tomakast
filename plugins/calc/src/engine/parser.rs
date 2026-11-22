//! Pratt parser: tokens to an expression tree.
//!
//! Binding powers, loosest first: `in`/`to` (1), `+ -` (2), `* / mod of` (4), unary minus
//! (6), `^` (8, right-associative). A postfix `%` and a unit directly after a number literal
//! bind tighter than any operator, and so do the words of a date (`31 dec 2026`) or a clock
//! time (`5pm London`). `days until <expr>` takes the sum after it (`+ -` and tighter).

use super::color::{self, Form};
use super::currency;
use super::lexer::Tok;
use super::time;
use super::units::{self, Dim, Unit};
use super::zones::{self, ZoneId};
use super::{Base, Value};

/// The deepest expression tree that is evaluated, and the deepest the parser recurses.
/// Every node counts: operators, `%`, unary minus, calls and `in` conversions; parentheses
/// only group. A long chain such as `1+1+1+…` is a deep tree.
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    Sqrt,
    Abs,
    Round,
    Floor,
    Ceil,
    Ln,
    Log,
    Sin,
    Cos,
    Tan,
}

impl Func {
    pub fn lookup(name: &str) -> Option<Func> {
        Some(match name {
            "sqrt" => Func::Sqrt,
            "abs" => Func::Abs,
            "round" => Func::Round,
            "floor" => Func::Floor,
            "ceil" => Func::Ceil,
            "ln" => Func::Ln,
            "log" => Func::Log,
            "sin" => Func::Sin,
            "cos" => Func::Cos,
            "tan" => Func::Tan,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Func::Sqrt => "sqrt",
            Func::Abs => "abs",
            Func::Round => "round",
            Func::Floor => "floor",
            Func::Ceil => "ceil",
            Func::Ln => "ln",
            Func::Log => "log",
            Func::Sin => "sin",
            Func::Cos => "cos",
            Func::Tan => "tan",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A number, with its unit when it was written `5 km`.
    Lit(Value),
    /// A variable, `prev`/`ans`, or a constant; resolved at evaluation time.
    Name(String),
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    /// `15%`: a fraction on its own, a share of the left side after `+`/`-`.
    Percent(Box<Expr>),
    /// `15% of 80`.
    Of(Box<Expr>, Box<Expr>),
    Call(Func, Box<Expr>),
    /// A literal written `0xff`, `0b1010` or `0o17`.
    Radix(f64),
    /// `5 km in mi`, `255 in hex`.
    Convert(Box<Expr>, Target),
    /// `17:42`, `5pm London`: today at that wall time, in the zone or the user's.
    Clock {
        hour: u8,
        minute: u8,
        zone: Option<ZoneId>,
    },
    /// `31 dec`, `dec 31 2027`: midnight in the user's zone; no year is this year.
    Date {
        day: u8,
        month: u8,
        year: Option<i32>,
    },
    /// `days until 31 dec`.
    Until(Unit, Box<Expr>),
    /// `rgb(255, 136, 0)`, `hsl(32 100% 50% / 0.5)`: three values and an optional alpha.
    ColorCall {
        func: ColorFn,
        args: Vec<Expr>,
        alpha: Option<Box<Expr>>,
    },
}

/// A colour function; `rgba` and `hsla` are `rgb` and `hsl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorFn {
    Rgb,
    Hsl,
    Oklch,
}

impl ColorFn {
    fn lookup(name: &str) -> Option<ColorFn> {
        Some(match name {
            "rgb" | "rgba" => ColorFn::Rgb,
            "hsl" | "hsla" => ColorFn::Hsl,
            "oklch" => ColorFn::Oklch,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            ColorFn::Rgb => "rgb",
            ColorFn::Hsl => "hsl",
            ColorFn::Oklch => "oklch",
        }
    }
}

/// What `in` / `to` converts to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Unit(Unit),
    /// A base, or `None` for `decimal`.
    Base(Option<Base>),
    /// The same instant in another zone.
    Zone(ZoneId),
    /// A plain number read as unix seconds.
    Date,
    /// A moment as unix seconds.
    Unix,
    /// A colour written in another form (`hex` is [`Target::Base`], which a colour reads
    /// as its hex form).
    Color(Form),
}

/// The targets named by a keyword, which come before units and zones.
const TARGET_WORDS: [(&str, Target); 12] = [
    ("hex", Target::Base(Some(Base::HEX))),
    ("bin", Target::Base(Some(Base::BIN))),
    ("oct", Target::Base(Some(Base::OCT))),
    ("hexadecimal", Target::Base(Some(Base::HEX))),
    ("binary", Target::Base(Some(Base::BIN))),
    ("octal", Target::Base(Some(Base::OCT))),
    ("decimal", Target::Base(None)),
    ("date", Target::Date),
    ("unix", Target::Unix),
    ("rgb", Target::Color(Form::Rgb)),
    ("hsl", Target::Color(Form::Hsl)),
    ("oklch", Target::Color(Form::Oklch)),
];

/// The keyword of a colour form, for messages: `hex`, `rgb`, `hsl` or `oklch`.
pub fn form_word(form: Form) -> &'static str {
    match form {
        Form::Hex => "hex",
        Form::Rgb => "rgb",
        Form::Hsl => "hsl",
        Form::Oklch => "oklch",
    }
}

/// The target a keyword names (`hex`, `decimal`, `date`).
fn target_word(word: &str) -> Option<Target> {
    TARGET_WORDS
        .iter()
        .find(|(w, _)| *w == word)
        .map(|(_, target)| *target)
}

/// The name of a base target, for messages: `hex`, `bin`, `oct`, `decimal` or `base 36`.
pub fn base_word(base: Option<Base>) -> String {
    match (
        base,
        TARGET_WORDS.iter().find(|(_, t)| *t == Target::Base(base)),
    ) {
        (_, Some((word, _))) => (*word).into(),
        (Some(Base(radix)), None) => format!("base {radix}"),
        (None, None) => "decimal".into(),
    }
}

/// `base 36` at the start of `toks`: the target and its two tokens, an error for a base
/// outside 2–36 or a missing number, or `None` when the first word is not `base`.
fn base_target(toks: &[Tok]) -> Option<Result<(Target, usize), String>> {
    if !matches!(toks.first(), Some(Tok::Ident(w)) if w == "base") {
        return None;
    }
    Some(match toks.get(1) {
        Some(Tok::Num(n, text))
            if !is_radix(text) && n.fract() == 0.0 && (2.0..=36.0).contains(n) =>
        {
            Ok((Target::Base(Some(Base(*n as u32))), 2))
        }
        Some(Tok::Num(..)) => Err("base must be 2 to 36".into()),
        _ => Err("expected a number after 'base'".into()),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Assign(String, Expr),
    Expr(Expr),
}

/// Words that mean something to the parser and so cannot name a variable.
fn reserved(name: &str) -> Option<String> {
    if units::reserved(name) {
        Some(format!("'{name}' is a unit name and cannot be a variable"))
    } else if Func::lookup(name).is_some() {
        Some(format!(
            "'{name}' is a function name and cannot be a variable"
        ))
    } else if matches!(
        name,
        "pi" | "e" | "prev" | "ans" | "of" | "mod" | "to" | "until"
    ) {
        Some(format!("'{name}' is reserved and cannot be a variable"))
    } else {
        None
    }
}

/// Parses one line's tokens: an assignment `name = expr`, or an expression.
pub fn parse(toks: Vec<Tok>) -> Result<Stmt, String> {
    let toks = split_inch_conversion(merge_compound_units(toks));
    let mut p = Parser {
        toks: &toks,
        pos: 0,
        depth: 0,
    };
    let assign = match (toks.first(), toks.get(1)) {
        (Some(Tok::Ident(name)), Some(Tok::Assign)) => {
            if let Some(why) = reserved(name) {
                return Err(why);
            }
            p.pos = 2;
            Some(name.clone())
        }
        _ => None,
    };
    let (expr, _) = p.expr(0)?;
    if let Some(tok) = p.peek() {
        return Err(format!("unexpected '{}'", tok.describe()));
    }
    Ok(match assign {
        Some(name) => Stmt::Assign(name, expr),
        None => Stmt::Expr(expr),
    })
}

/// `km / h` and `m/s` are speed units: `Ident(a) Slash Ident(b)` becomes `Ident("a/b")` when
/// that is a unit. `10 km / 500 m` stays a ratio.
fn merge_compound_units(toks: Vec<Tok>) -> Vec<Tok> {
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if let [Tok::Ident(a), Tok::Slash, Tok::Ident(b)] = &toks[i..toks.len().min(i + 3)] {
            let joined = format!("{a}/{b}");
            if units::lookup(&joined).is_some() {
                out.push(Tok::Ident(joined));
                i += 3;
                continue;
            }
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

/// `12 in cm` means inches converted to centimetres, since a plain number cannot be converted.
/// Rewrite it to `12 in in cm` (inch, then the `in` operator) so the parser needs no special
/// case and `2 * 12 in cm` groups like `2 * 12 km in m`.
fn split_inch_conversion(toks: Vec<Tok>) -> Vec<Tok> {
    let mut out = Vec::with_capacity(toks.len() + 1);
    for tok in toks {
        let after_inch = matches!(
            out.as_slice(),
            [.., Tok::Num(..), Tok::Ident(w)] if w == "in"
        );
        if after_inch && matches!(&tok, Tok::Ident(u) if u != "in" && units::lookup(u).is_some()) {
            out.push(Tok::Ident("in".into()));
        }
        out.push(tok);
    }
    out
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<Tok> {
        let tok = self.toks.get(self.pos).cloned();
        self.pos += 1;
        tok
    }

    /// Runs `f` one nesting level deeper, failing past [`MAX_DEPTH`].
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        if self.depth >= MAX_DEPTH {
            return Err("too deeply nested".into());
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        result
    }

    /// Parses an expression and returns it with the depth of its tree.
    fn expr(&mut self, min_bp: u8) -> Result<(Expr, usize), String> {
        let (mut lhs, mut depth) = self.prefix()?;
        loop {
            let (lbp, rbp, op) = match self.peek() {
                Some(Tok::Plus) => (2, 3, Infix::Op(Op::Add)),
                Some(Tok::Minus) => (2, 3, Infix::Op(Op::Sub)),
                Some(Tok::Star) => (4, 5, Infix::Op(Op::Mul)),
                Some(Tok::Slash) => (4, 5, Infix::Op(Op::Div)),
                Some(Tok::Caret) => (8, 8, Infix::Op(Op::Pow)),
                Some(Tok::Ident(w)) => match w.as_str() {
                    "mod" => (4, 5, Infix::Op(Op::Mod)),
                    "of" => (4, 5, Infix::Of),
                    "in" | "to" => (1, 2, Infix::Convert),
                    _ => break,
                },
                _ => break,
            };
            if lbp < min_bp {
                break;
            }
            let word = self.bump().map(|t| t.describe()).unwrap_or_default();
            lhs = match op {
                Infix::Convert => {
                    depth = up(depth)?;
                    Expr::Convert(Box::new(lhs), self.target_after(&word)?)
                }
                Infix::Of | Infix::Op(_) => {
                    let (rhs, rhs_depth) = if matches!(op, Infix::Op(Op::Pow)) {
                        self.nested(|p| p.expr(rbp))?
                    } else {
                        self.expr(rbp)?
                    };
                    depth = up(depth.max(rhs_depth))?;
                    let (l, r) = (Box::new(lhs), Box::new(rhs));
                    match op {
                        Infix::Op(op) => Expr::Bin(op, l, r),
                        _ => Expr::Of(l, r),
                    }
                }
            };
        }
        Ok((lhs, depth))
    }

    /// The target that must follow `in` / `to`: a keyword (`hex`, `decimal`, `date`,
    /// `unix`), then a unit, then a currency, then a zone (which may take several words).
    fn target_after(&mut self, word: &str) -> Result<Target, String> {
        let Some(Tok::Ident(name)) = self.peek() else {
            return Err(format!("expected a unit after '{word}'"));
        };
        if let Some(found) = base_target(&self.toks[self.pos..]) {
            let (target, len) = found?;
            self.pos += len;
            return Ok(target);
        }
        let (target, len) = match target_at(&self.toks[self.pos..]) {
            Some(found) => found,
            None => return Err(format!("unknown unit '{name}'")),
        };
        self.pos += len;
        Ok(target)
    }

    /// Whether the next token is `in` and the words after it name a target that is not a
    /// physical unit, so that `255 in hex`, `12 in Tokyo` and `12 in usd` are conversions and
    /// not inches.
    fn conversion_follows(&self) -> bool {
        matches!(self.peek(), Some(Tok::Ident(w)) if w == "in")
            && (base_target(&self.toks[self.pos + 1..]).is_some()
                || match target_at(&self.toks[self.pos + 1..]) {
                    Some((Target::Unit(unit), _)) => unit.dim == Dim::Currency,
                    Some(_) => true,
                    None => false,
                })
    }

    /// After a number or a clock: `am` or `pm`.
    fn meridiem(&mut self) -> Option<bool> {
        let pm = match self.peek() {
            Some(Tok::Ident(w)) if w == "am" => false,
            Some(Tok::Ident(w)) if w == "pm" => true,
            _ => return None,
        };
        self.pos += 1;
        Some(pm)
    }

    /// A clock time at `hour:minute`, with the `am`/`pm` and the zone that may follow.
    fn clock(&mut self, hour: u8, minute: u8) -> Result<(Expr, usize), String> {
        let hour = match self.meridiem() {
            None if hour <= 23 => hour,
            Some(pm) if (1..=12).contains(&hour) => hour % 12 + if pm { 12 } else { 0 },
            _ => return Err(NOT_A_TIME.into()),
        };
        if minute > 59 {
            return Err(NOT_A_TIME.into());
        }
        let zone = zones::at(&self.toks[self.pos..]).map(|(zone, len)| {
            self.pos += len;
            zone
        });
        Ok((Expr::Clock { hour, minute, zone }, 1))
    }

    /// A date whose day (as written) and month are read; a 4-digit year may follow.
    fn date(&mut self, day_text: &str, month: u8) -> Result<(Expr, usize), String> {
        let day = match day_text.parse::<u8>() {
            Ok(day @ 1..=31) if day_text.len() <= 2 && all_digits(day_text) => day,
            _ => return Err(NOT_A_DATE.into()),
        };
        let year = match self.peek() {
            Some(Tok::Num(_, text)) if text.len() == 4 && all_digits(text) => {
                let year = text.parse().ok();
                self.pos += 1;
                year
            }
            _ => None,
        };
        Ok((Expr::Date { day, month, year }, 1))
    }

    /// A value: a number (with its unit), a name, a call, a parenthesised expression or a
    /// negation, followed by any number of `%`.
    fn prefix(&mut self) -> Result<(Expr, usize), String> {
        let (mut e, mut depth) = match self.bump() {
            Some(Tok::Num(number, text)) if is_radix(&text) => (Expr::Radix(number), 1),
            Some(Tok::Hex(digits)) => {
                let color = color::from_hex(&digits).ok_or_else(|| "not a color".to_string())?;
                (Expr::Lit(Value::Color(color)), 1)
            }
            Some(Tok::Num(number, text)) => self.after_number(number, &text)?,
            Some(Tok::Clock(hour, minute, _)) => self.clock(hour, minute)?,
            Some(Tok::Minus) => {
                let (inner, d) = self.nested(|p| p.expr(6))?;
                (Expr::Neg(Box::new(inner)), up(d)?)
            }
            Some(Tok::LParen) => self.parenthesised()?,
            Some(Tok::Ident(name)) => self.name(name)?,
            Some(tok) => return Err(format!("unexpected '{}'", tok.describe())),
            None => return Err("expected a value at the end of the line".into()),
        };
        while self.peek() == Some(&Tok::Percent) {
            self.pos += 1;
            depth = up(depth)?;
            e = Expr::Percent(Box::new(e));
        }
        Ok((e, depth))
    }

    /// After a decimal number: its unit (`5 km`), a clock time (`5pm`), a date (`31 dec`), or
    /// the number alone.
    fn after_number(&mut self, number: f64, text: &str) -> Result<(Expr, usize), String> {
        let Some(Tok::Ident(word)) = self.peek() else {
            return Ok((Expr::Lit(Value::plain(number)), 1));
        };
        if self.conversion_follows() {
            return Ok((Expr::Lit(Value::plain(number)), 1));
        }
        if let Some(unit) = units::lookup(word).or_else(|| currency::lookup(word)) {
            self.pos += 1;
            return Ok((Expr::Lit(Value::with_unit(number, Some(unit))), 1));
        }
        if matches!(word.as_str(), "am" | "pm") {
            let hour = match text.parse::<u8>() {
                Ok(hour) if text.len() <= 2 && all_digits(text) => hour,
                _ => return Err(NOT_A_TIME.into()),
            };
            return self.clock(hour, 0);
        }
        if let Some(month) = time::month(word) {
            self.pos += 1;
            return self.date(text, month);
        }
        Ok((Expr::Lit(Value::plain(number)), 1))
    }

    /// After `(`: an expression and the closing `)`.
    fn parenthesised(&mut self) -> Result<(Expr, usize), String> {
        let inner = self.nested(|p| p.expr(0))?;
        match self.bump() {
            Some(Tok::RParen) => Ok(inner),
            _ => Err("expected ')'".into()),
        }
    }

    fn name(&mut self, name: String) -> Result<(Expr, usize), String> {
        if let Some(func) = ColorFn::lookup(&name)
            && self.peek() == Some(&Tok::LParen)
        {
            self.pos += 1;
            return self.nested(|p| p.color_call(func));
        }
        if let Some(func) = Func::lookup(&name) {
            // `tan in hex`: a function name alone may be a colour.
            if self.peek() != Some(&Tok::LParen) && color::named(&name).is_some() {
                return Ok((Expr::Name(name), 1));
            }
            if self.peek() != Some(&Tok::LParen) {
                return Err(format!("'{name}' needs parentheses"));
            }
            self.pos += 1;
            let (arg, depth) = self.parenthesised()?;
            return Ok((Expr::Call(func, Box::new(arg)), up(depth)?));
        }
        // `days until 31 dec`.
        if let Some(unit) = units::lookup(&name).filter(|u| u.dim == Dim::Time)
            && matches!(self.peek(), Some(Tok::Ident(w)) if w == "until")
        {
            self.pos += 1;
            let (target, depth) = self.nested(|p| p.expr(2))?;
            return Ok((Expr::Until(unit, Box::new(target)), up(depth)?));
        }
        // `dec 31`: a month name is a date only before a day; alone it is a name.
        if let Some(month) = time::month(&name)
            && let Some(Tok::Num(_, day)) = self.peek()
        {
            let day = day.clone();
            self.pos += 1;
            return self.date(&day, month);
        }
        // `$100`: a currency symbol before a number.
        if currency::is_symbol(&name)
            && let Some(Tok::Num(number, text)) = self.peek()
            && !is_radix(text)
            && let Some(unit) = currency::lookup(&name)
        {
            let number = *number;
            self.pos += 1;
            return Ok((Expr::Lit(Value::with_unit(number, Some(unit))), 1));
        }
        // A soft unit name is a plain name here: the evaluator reads the variable, or says
        // it is a unit.
        if units::reserved(&name) {
            return Err(format!("'{name}' is a unit; put a number before it"));
        }
        if matches!(name.as_str(), "of" | "mod" | "to" | "until") {
            return Err(format!("unexpected '{name}'"));
        }
        Ok((Expr::Name(name), 1))
    }
}

impl Parser<'_> {
    /// After `rgb(`: three values and an optional alpha, then `)`. With a comma after the
    /// first value, values are comma-separated expressions and a fourth is the alpha;
    /// otherwise they are separated by spaces, bind tighter than `+ - * /` (so
    /// `rgb(255 -1 0)` is three values), and `/` comes before the alpha.
    fn color_call(&mut self, func: ColorFn) -> Result<(Expr, usize), String> {
        let wrong = || format!("{} needs 3 or 4 values", func.name());
        let commas = self.comma_before_close();
        let mut args = Vec::new();
        let mut alpha = None;
        let mut depth = 0;
        loop {
            match self.peek() {
                Some(Tok::RParen) => {
                    self.pos += 1;
                    break;
                }
                None => return Err("expected ')'".into()),
                Some(Tok::Slash) if !commas && args.len() == 3 && alpha.is_none() => {
                    self.pos += 1;
                    let (a, d) = self.expr(6)?;
                    depth = depth.max(d);
                    alpha = Some(Box::new(a));
                    continue;
                }
                _ => {}
            }
            if (commas && !args.is_empty()) && self.bump() != Some(Tok::Comma) {
                return Err(wrong());
            }
            if alpha.is_some() {
                return Err(wrong());
            }
            let (arg, d) = self.expr(if commas { 0 } else { 6 })?;
            depth = depth.max(d);
            args.push(arg);
        }
        if commas && args.len() == 4 {
            alpha = args.pop().map(Box::new);
        }
        if args.len() != 3 {
            return Err(wrong());
        }
        Ok((Expr::ColorCall { func, args, alpha }, up(depth)?))
    }

    /// Whether a comma comes before the `)` that closes the call just opened.
    fn comma_before_close(&self) -> bool {
        let mut open = 0usize;
        for tok in &self.toks[self.pos..] {
            match tok {
                Tok::LParen => open += 1,
                Tok::RParen if open == 0 => return false,
                Tok::RParen => open -= 1,
                Tok::Comma if open == 0 => return true,
                _ => {}
            }
        }
        false
    }
}

const NOT_A_TIME: &str = "not a time";
const NOT_A_DATE: &str = "not a date";

/// The target at the start of `toks` (a keyword, then a unit, then a currency, then a zone),
/// and how many tokens it takes.
fn target_at(toks: &[Tok]) -> Option<(Target, usize)> {
    let Some(Tok::Ident(name)) = toks.first() else {
        return None;
    };
    if let Some(target) = target_word(name) {
        return Some((target, 1));
    }
    // A zone of several words wins over a one-word unit: `in St Louis` is no stone.
    let zone = zones::at(toks);
    if let Some((zone, len)) = zone.filter(|&(_, len)| len > 1) {
        return Some((Target::Zone(zone), len));
    }
    if let Some(unit) = units::lookup(name).or_else(|| currency::lookup(name)) {
        return Some((Target::Unit(unit), 1));
    }
    zone.map(|(zone, len)| (Target::Zone(zone), len))
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// Whether a number token was written `0x…`, `0b…` or `0o…`.
fn is_radix(text: &str) -> bool {
    matches!(text.as_bytes(), [b'0', b'x' | b'b' | b'o', ..])
}

/// The depth of a node one level above a child of depth `depth`.
fn up(depth: usize) -> Result<usize, String> {
    if depth >= MAX_DEPTH {
        Err("too deeply nested".into())
    } else {
        Ok(depth + 1)
    }
}

enum Infix {
    Op(Op),
    Of,
    Convert,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::lexer::lex;

    fn parse_str(s: &str) -> Result<Stmt, String> {
        parse(lex(s)?)
    }

    fn lit(n: f64) -> Box<Expr> {
        Box::new(Expr::Lit(Value::plain(n)))
    }

    #[test]
    fn precedence_shapes() {
        assert_eq!(
            parse_str("1 + 2 * 3"),
            Ok(Stmt::Expr(Expr::Bin(
                Op::Add,
                lit(1.0),
                Box::new(Expr::Bin(Op::Mul, lit(2.0), lit(3.0)))
            )))
        );
        assert_eq!(
            parse_str("-2 ^ 2"),
            Ok(Stmt::Expr(Expr::Neg(Box::new(Expr::Bin(
                Op::Pow,
                lit(2.0),
                lit(2.0)
            )))))
        );
        assert_eq!(
            parse_str("2 ^ 3 ^ 4"),
            Ok(Stmt::Expr(Expr::Bin(
                Op::Pow,
                lit(2.0),
                Box::new(Expr::Bin(Op::Pow, lit(3.0), lit(4.0)))
            )))
        );
    }

    #[test]
    fn assignment_and_reserved_names() {
        assert!(matches!(parse_str("x = 1"), Ok(Stmt::Assign(n, _)) if n == "x"));
        assert!(parse_str("m = 1").is_err());
        assert!(parse_str("ln = 1").is_err());
        assert!(parse_str("e = 1").is_err());
    }

    #[test]
    fn compound_units_and_bases() {
        assert_eq!(parse_str("10 km / h"), parse_str("10 km/h"));
        assert!(matches!(
            parse_str("10 km / h"),
            Ok(Stmt::Expr(Expr::Lit(Value::Num { unit: Some(u), .. }))) if u.name == "km/h"
        ));
        assert!(matches!(
            parse_str("10 km / 500 m"),
            Ok(Stmt::Expr(Expr::Bin(Op::Div, ..)))
        ));
        assert!(matches!(
            parse_str("255 in hex"),
            Ok(Stmt::Expr(Expr::Convert(_, Target::Base(Some(Base::HEX)))))
        ));
        assert!(matches!(parse_str("0xff"), Ok(Stmt::Expr(Expr::Radix(n))) if n == 255.0));
    }

    #[test]
    fn dates_clocks_and_until() {
        let date = Expr::Date {
            day: 31,
            month: 12,
            year: None,
        };
        assert_eq!(parse_str("dec 31"), Ok(Stmt::Expr(date.clone())));
        assert_eq!(parse_str("31 December"), Ok(Stmt::Expr(date.clone())));
        assert!(matches!(
            parse_str("31 dec 2027"),
            Ok(Stmt::Expr(Expr::Date {
                year: Some(2027),
                ..
            }))
        ));
        // `until` takes the sum after it; `in` applies to the whole.
        let Ok(Stmt::Expr(Expr::Convert(until, Target::Unit(weeks)))) =
            parse_str("days until 31 dec + 1 day in weeks")
        else {
            panic!("a conversion of `until`");
        };
        assert_eq!(weeks.name, "week");
        assert!(matches!(*until, Expr::Until(u, ref e) if u.name == "day"
            && matches!(**e, Expr::Bin(Op::Add, ..))));
        assert!(matches!(
            parse_str("5:30 pm Tokyo"),
            Ok(Stmt::Expr(Expr::Clock {
                hour: 17,
                minute: 30,
                zone: Some(_)
            }))
        ));
        assert!(matches!(
            parse_str("12 in Tokyo"),
            Ok(Stmt::Expr(Expr::Convert(_, Target::Zone(_))))
        ));
        assert!(matches!(
            parse_str("1700000000 to date"),
            Ok(Stmt::Expr(Expr::Convert(_, Target::Date)))
        ));
        assert_eq!(parse_str("0:30 am"), Err("not a time".into()));
        assert_eq!(parse_str("32 dec"), Err("not a date".into()));
    }

    #[test]
    fn inch_conversion_is_split() {
        let want = parse_str("12 in in cm");
        assert!(want.is_ok());
        assert_eq!(parse_str("12 in cm"), want);
        // A lone `in` after a number is still the inch unit.
        assert!(matches!(
            parse_str("12 in"),
            Ok(Stmt::Expr(Expr::Lit(Value::Num { unit: Some(u), .. }))) if u.name == "in"
        ));
    }
}
