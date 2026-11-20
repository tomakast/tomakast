//! What calc shows: the search answer and its tile, the scratchpad of a sheet and the
//! history list. Pure functions of their inputs, so they run in native tests, but for the
//! last search answer, which its tile's captions are drawn from. Every string the
//! host caps (ids, one-line titles and tile texts of at most 1 KiB, annotation lines below
//! 10 000) is made to fit here: the host rejects a whole effect that breaks a cap.

use std::cell::RefCell;

use tomakast_plugin::{
    Align, Color, ColorToken, InvalidId, Justify, Layout, LineAnnotation, ListRow, ListView, Node,
    ScratchpadView, SearchMode, TextAlign, TextStyle, View, WidgetAnswer, WidgetContext,
    WidgetFrame,
};

use crate::engine::{self, Context, Dim, LineResult, Sheet, Value, zones};
use crate::rates;

/// The view id of the history list.
pub const HISTORY: &str = "history";
/// The view id of sheet `<id>` is this prefix and the id.
pub const SHEET_PREFIX: &str = "sheet:";

/// Titles, subtitles, tile texts and annotation texts are at most this many bytes.
const MAX_LABEL_BYTES: usize = 1024;
/// Annotations name lines below this.
const MAX_ANNOTATION_LINES: usize = 10_000;
/// The most text a pushed pad may hold.
const MAX_PAD_BYTES: usize = 256 * 1024;
/// The most of a sheet that is evaluated: what a pad the launcher accepts can hold.
const MAX_EVAL_BYTES: usize = MAX_PAD_BYTES;

/// Rates older than this (a day, in seconds) are said to be old in the answer.
pub const STALE_AFTER: i64 = 86_400;

/// The widget calc answers math with (`[[widgets]]` of `plugin.toml`).
pub const ANSWER: &str = "answer";

thread_local! {
    /// The parts of the last search answer, by its payload: the tile's captions say what
    /// that search read (old rates, the zone), even when it is drawn again later.
    static LAST: RefCell<Option<(String, Answer)>> = const { RefCell::new(None) };
}

/// What the answer tile shows: the input as calc read it and the result, each with a
/// caption, as Raycast's answer card does.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// The query as one line.
    pub input: String,
    pub result: String,
    /// The note of what the answer read that does not show (its zone, old rates), or what
    /// was computed: "Conversion" or "Expression".
    pub input_caption: String,
    /// The kind of the result in words: its unit's dimension, "Date and time" or "Number".
    pub result_caption: String,
}

/// The parts of the answer for `query`, if it is math.
pub fn answer_parts(query: &str, ctx: &Context) -> Option<Answer> {
    let value = engine::eval_query(query, ctx)?;
    let input = one_line(query.trim());
    let input_caption = note(&value, ctx).unwrap_or_else(|| computed(&input).into());
    Some(Answer {
        input,
        result: engine::format(&value),
        input_caption,
        result_caption: result_caption(&value).into(),
    })
}

/// What `input` computes: "Conversion" when it holds ` to ` or ` in ` (in any case), else
/// "Expression".
fn computed(input: &str) -> &'static str {
    let lower = input.to_ascii_lowercase();
    if lower.contains(" to ") || lower.contains(" in ") {
        "Conversion"
    } else {
        "Expression"
    }
}

/// The zone and the rates' age an answer should name, joined with " · ": a date names its
/// own zone (`today in Tokyo`: Asia/Tokyo), a time shown in another zone than the user's
/// names the user's when it read it (`5pm in Tokyo`: UTC, as 5pm there), and any other
/// value that used the user's zone (a duration counted from today) names that one.
fn note(value: &Value, ctx: &Context) -> Option<String> {
    let home = ctx.used().zone.then_some(ctx.zone);
    let zone = match value {
        Value::Moment(m) if m.date_only => Some(m.zone),
        Value::Moment(m) if m.zone != ctx.zone => home,
        Value::Moment(_) => None,
        _ => home,
    };
    joined([
        zone.map(|zone| zones::name(zone).to_owned()),
        stale_rates(ctx),
    ])
}

/// The notes there are, joined with " · ", or `None` when there is none.
fn joined(notes: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    let notes: Vec<String> = notes.into_iter().flatten().collect();
    (!notes.is_empty()).then(|| notes.join(" · "))
}

fn result_caption(value: &Value) -> &'static str {
    match value {
        Value::Num {
            unit: Some(unit), ..
        } => dim_words(unit.dim),
        Value::Moment(_) => "Date and time",
        Value::Num { unit: None, .. } | Value::Based { .. } => "Number",
    }
}

/// A dimension in words, as the result's caption.
pub fn dim_words(dim: Dim) -> &'static str {
    match dim {
        Dim::Length => "Length",
        Dim::Mass => "Mass",
        Dim::Time => "Duration",
        Dim::Temperature => "Temperature",
        Dim::Area => "Area",
        Dim::Volume => "Volume",
        Dim::Speed => "Speed",
        Dim::Data => "Data",
        Dim::Angle => "Angle",
        Dim::Currency => "Currency",
    }
}

/// The search answer for `query`, if it is math: the `answer` tile, whose payload is the
/// result and the input on two lines (what the tile shows and what its actions copy).
pub fn answer(query: &str, ctx: &Context) -> Result<Option<WidgetAnswer>, InvalidId> {
    let Some(parts) = answer_parts(query, ctx) else {
        return Ok(None);
    };
    let payload = format!("{}\n{}", parts.result, parts.input);
    LAST.with(|last| *last.borrow_mut() = Some((payload.clone(), parts)));
    let answer = WidgetAnswer::new(ANSWER, payload)?
        .action("copy", "Copy Result")?
        .action("copy-both", "Copy Input + Result")?;
    Ok(Some(answer))
}

/// The result and the input of an answer's payload.
pub fn payload_parts(payload: &str) -> Option<(&str, &str)> {
    payload.split_once('\n')
}

/// The answer tile: the input and its caption on the left, the result and its caption on
/// the right. The result and the input are the payload's, what search computed and Enter
/// copies (`now` moves on, rates change). The captions are those of the last search answer
/// when the payload is its; only for another payload is the query read again.
pub fn answer_tile(wctx: &WidgetContext, ctx: &Context) -> Result<WidgetFrame, String> {
    let no_answer = || "no answer".to_owned();
    let payload = wctx.payload.as_deref().ok_or_else(no_answer)?;
    let (result, input) = payload_parts(payload).ok_or_else(no_answer)?;
    let searched = LAST.with(|last| {
        last.borrow()
            .as_ref()
            .filter(|(last, _)| last == payload)
            .map(|(_, parts)| parts.clone())
    });
    let parts = match searched {
        Some(parts) => parts,
        None => wctx
            .query
            .as_deref()
            .and_then(|query| answer_parts(query, ctx))
            .ok_or_else(no_answer)?,
    };
    let half = |align| {
        Node::column(
            Layout::default()
                .justify(Justify::SpaceBetween)
                .align(align),
        )
        .grow(1.)
    };
    let caption =
        |text: String| Node::text(text, TextStyle::Caption).color(Color::token(ColorToken::Muted));
    let frame = WidgetFrame::new(Node::row(Layout::default().padding(16.).gap(16.)));
    let root = frame.root();
    let (frame, left) = frame.child(root, half(Align::Start));
    let (frame, right) = frame.child(root, half(Align::End));
    let (frame, _) = frame.child(
        left,
        Node::text(one_line(input), TextStyle::Value)
            .mono()
            .max_lines(1)
            .color(Color::token(ColorToken::Text)),
    );
    let (frame, _) = frame.child(left, caption(parts.input_caption));
    let (frame, _) = frame.child(
        right,
        Node::text(one_line(result), TextStyle::Value)
            .max_lines(1)
            .align(TextAlign::End),
    );
    let (frame, _) = frame.child(right, caption(parts.result_caption).align(TextAlign::End));
    Ok(frame.build())
}

/// The age of the rates the evaluation used, when it is worth saying: the oldest feed's, if
/// it is older than [`STALE_AFTER`], or unknown when a feed is dated after now.
fn stale_rates(ctx: &Context) -> Option<String> {
    let used = ctx.used();
    let tables: Vec<_> = [
        (used.fiat, &ctx.rates.fiat),
        (used.crypto, &ctx.rates.crypto),
    ]
    .into_iter()
    .filter_map(|(used, table)| table.as_ref().filter(|_| used))
    .collect();
    if tables.iter().any(|t| rates::from_the_future(t, ctx.now)) {
        return Some("rates of unknown age".into());
    }
    tables
        .iter()
        .map(|table| table.fetched)
        .min()
        .filter(|fetched| ctx.now - fetched > STALE_AFTER)
        .map(|fetched| format!("rates from {}", relative(ctx.now - fetched)))
}

/// The scratchpad of sheet `id` showing `text`, evaluated: the view to push. It shows at
/// most the first 256 KiB of `text` (the host rejects a longer pad), which is also as far
/// as `text` is evaluated.
pub fn pad(id: &str, text: &str, ctx: &Context) -> Result<ScratchpadView, InvalidId> {
    pad_view(id, truncate(text, MAX_PAD_BYTES), text, 0, ctx)
}

/// The update of sheet `id`'s scratchpad for `text`, the edit of `revision`. The launcher
/// ignores an update's text (which could also be over its cap), so it carries none: only the
/// annotations, the title and the actions change, and the actions are sent again because an
/// update replaces them.
pub fn pad_update(
    id: &str,
    text: &str,
    revision: u64,
    ctx: &Context,
) -> Result<ScratchpadView, InvalidId> {
    pad_view(id, "", text, revision, ctx)
}

fn pad_view(
    id: &str,
    shown: &str,
    text: &str,
    revision: u64,
    ctx: &Context,
) -> Result<ScratchpadView, InvalidId> {
    let pad = View::scratchpad(format!("{SHEET_PREFIX}{id}"), "Sheet", shown)?
        .revision(revision)
        .annotations(annotations(text, ctx))
        .action("copy-last", "Copy Last Result")?
        .action("new", "New Sheet")?
        .action("history", "History")?;
    Ok(pad)
}

/// The part of `text` that is evaluated: its first 10 000 lines (the lines an annotation
/// can name), and at most 256 KiB of whole lines. Later lines cannot change an earlier
/// line's value, and evaluating them would only spend the call's deadline (a pasted file).
fn evaluated(text: &str) -> &str {
    let mut text = text;
    if text.len() > MAX_EVAL_BYTES {
        let cut = truncate(text, MAX_EVAL_BYTES);
        // Whole lines only (the cut may fall right before a newline: then its last line is
        // whole); a single line this long is an error anyway.
        text = if text.as_bytes()[cut.len()] == b'\n' {
            cut
        } else {
            cut.rfind('\n').map_or(cut, |end| &cut[..end])
        };
    }
    match text.match_indices('\n').nth(MAX_ANNOTATION_LINES - 1) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

/// One annotation per non-empty line: its value, or its error marked as one. Lines from
/// 10 000 on get none (the contract's limit), and are not evaluated.
pub fn annotations(text: &str, ctx: &Context) -> Vec<LineAnnotation> {
    Sheet::eval(evaluated(text), ctx)
        .into_iter()
        .take(MAX_ANNOTATION_LINES)
        .enumerate()
        .filter_map(|(line, result)| {
            let line = line as u32;
            match result {
                LineResult::Empty => None,
                LineResult::Value(v) | LineResult::Assign(_, v) => {
                    Some(LineAnnotation::note(line, one_line(&engine::format(&v))))
                }
                LineResult::Error(e) => Some(LineAnnotation::error(line, one_line(&e))),
            }
        })
        .collect()
}

/// The value of the last line of `text` that has one, formatted; only the lines that get
/// annotations count.
pub fn last_result(text: &str, ctx: &Context) -> Option<String> {
    Sheet::eval(evaluated(text), ctx)
        .into_iter()
        .rev()
        .find_map(|r| match r {
            LineResult::Value(v) | LineResult::Assign(_, v) => Some(engine::format(&v)),
            LineResult::Empty | LineResult::Error(_) => None,
        })
}

/// A sheet as the history lists it.
pub struct Entry {
    pub id: String,
    pub text: String,
    /// When it was last edited, in unix seconds.
    pub edited: Option<i64>,
}

/// The history list: one row per sheet, in the order given (newest first).
pub fn history(entries: &[Entry], now: i64) -> Result<ListView, InvalidId> {
    let rows = entries
        .iter()
        .map(|entry| {
            let mut row = ListRow::new(&entry.id, title(&entry.text))?
                .action("open", "Open Sheet")?
                .action("delete", "Delete Sheet")?;
            if let Some(edited) = entry.edited {
                row = row.subtitle(format!("Edited {}", relative(now - edited)));
            }
            Ok(row)
        })
        .collect::<Result<Vec<_>, InvalidId>>()?;
    Ok(View::list(HISTORY, "History")?
        .placeholder("Filter sheets")
        .search(SearchMode::CoreFilter)
        .rows(rows))
}

/// The first non-empty line of `text`, or "Empty sheet".
fn title(text: &str) -> String {
    match text.lines().map(str::trim).find(|line| !line.is_empty()) {
        Some(line) => one_line(line),
        None => "Empty sheet".into(),
    }
}

/// Sheet `text` as a toast names it: its first non-empty line in quotes, cut to `max`
/// characters, or "an empty sheet".
pub fn sheet_name(text: &str, max: usize) -> String {
    if text.trim().is_empty() {
        return "an empty sheet".into();
    }
    let line = title(text);
    match line.char_indices().nth(max) {
        Some((end, _)) => format!("“{}…”", line[..end].trim_end()),
        None => format!("“{line}”"),
    }
}

/// `seconds` ago, in words.
fn relative(seconds: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    if seconds < MINUTE {
        "just now".into()
    } else if seconds < HOUR {
        format!("{} min ago", seconds / MINUTE)
    } else if seconds < DAY {
        format!("{} h ago", seconds / HOUR)
    } else if seconds < 2 * DAY {
        "yesterday".into()
    } else {
        format!("{} days ago", seconds / DAY)
    }
}

/// `text` as one display line: control characters become spaces, and it is cut to 1 KiB on
/// a character boundary.
pub fn one_line(text: &str) -> String {
    let line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    truncate(&line, MAX_LABEL_BYTES).to_owned()
}

/// The longest prefix of `text` of at most `max` bytes that ends on a character boundary.
pub fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Whether `id` is one calc made: a base-36 counter.
pub fn is_sheet_id(id: &str) -> bool {
    (1..=13).contains(&id.len()) && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'z'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Rates, Table, ZoneId};
    use tomakast_plugin::{NodeKind, TextNode, TextSizes, ThemeColors, WidgetTheme};

    /// 2026-11-19T14:42:00Z, in Moscow, with no rates.
    fn ctx() -> Context<'static> {
        static NONE: std::sync::OnceLock<Rates> = std::sync::OnceLock::new();
        Context::new(
            1_795_099_320,
            zones::find("Europe/Moscow").unwrap(),
            NONE.get_or_init(Rates::default),
        )
    }

    /// The context of the `answer` tile for `query`, with the payload search gave it.
    fn tile_ctx(query: Option<&str>, payload: Option<&str>) -> WidgetContext {
        WidgetContext {
            width: 718.,
            height: 120.,
            scale: 2.,
            theme: WidgetTheme {
                dark: true,
                colors: ThemeColors {
                    text: 0xffffffff,
                    muted: 0x999999ff,
                    panel: 0x202020ff,
                    border: 0x404040ff,
                    selected: 0x303060ff,
                    error: 0xff4040ff,
                    success: 0x40ff40ff,
                },
                ui_font: "Inter".into(),
                mono_font: "Mono".into(),
                sizes: TextSizes {
                    title: 16.,
                    value: 28.,
                    body: 14.,
                    caption: 12.,
                },
            },
            millis_shown: 0,
            focused: None,
            pointer: None,
            query: query.map(Into::into),
            payload: payload.map(Into::into),
        }
    }

    /// The tile `answer` gives for `query`, rendered with that answer's payload.
    fn tile(query: &str, ctx: &Context) -> WidgetFrame {
        let answer = answer(query, ctx).unwrap().expect("an answer");
        answer_tile(&tile_ctx(Some(query), Some(&answer.payload)), ctx).unwrap()
    }

    fn texts(frame: &WidgetFrame) -> Vec<&TextNode> {
        frame
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                NodeKind::Text(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    fn parts(query: &str, ctx: &Context) -> (String, String, String, String) {
        let a = answer_parts(query, ctx).expect("an answer");
        (a.input, a.result, a.input_caption, a.result_caption)
    }

    #[test]
    fn answer_parts_names_what_was_computed() {
        assert_eq!(
            parts("2+2", &ctx()),
            (
                "2+2".into(),
                "4".into(),
                "Expression".into(),
                "Number".into()
            )
        );
        let km = answer_parts("5 km to m", &ctx()).unwrap();
        assert_eq!(
            (km.result.as_str(), km.result_caption.as_str()),
            ("5000 m", "Length")
        );
        assert_eq!(km.input_caption, "Conversion");
        // Either word, in any case, makes a conversion; a date or time has its own caption.
        assert_eq!(parts("255 in hex", &ctx()).2, "Conversion");
        assert_eq!(parts("255 in hex", &ctx()).3, "Number");
        assert_eq!(computed("5 km TO m"), "Conversion");
        assert_eq!(computed("now In Tokyo"), "Conversion");
        assert_eq!(computed("tokyo"), "Expression");
        assert_eq!(parts("now", &ctx()).3, "Date and time");
        assert_eq!(parts("1 h to min", &ctx()).3, "Duration");
        // The input is the query as one trimmed line.
        assert_eq!(parts("  2\t+ 2 ", &ctx()).0, "2 + 2");
    }

    #[test]
    fn every_dimension_has_words() {
        for (dim, words) in [
            (Dim::Length, "Length"),
            (Dim::Mass, "Mass"),
            (Dim::Time, "Duration"),
            (Dim::Temperature, "Temperature"),
            (Dim::Area, "Area"),
            (Dim::Volume, "Volume"),
            (Dim::Speed, "Speed"),
            (Dim::Data, "Data"),
            (Dim::Angle, "Angle"),
            (Dim::Currency, "Currency"),
        ] {
            assert_eq!(dim_words(dim), words);
        }
    }

    #[test]
    fn the_answer_carries_result_and_input_in_its_payload() {
        let answer = answer("2+2", &ctx()).unwrap().expect("math");
        assert_eq!(answer.id, "answer");
        assert_eq!(answer.payload, "4\n2+2");
        let actions: Vec<_> = answer
            .actions
            .iter()
            .map(|a| (a.id.as_str(), a.title.as_str()))
            .collect();
        assert_eq!(
            actions,
            [
                ("copy", "Copy Result"),
                ("copy-both", "Copy Input + Result")
            ]
        );
    }

    #[test]
    fn no_math_no_answer() {
        assert!(answer("firefox", &ctx()).unwrap().is_none());
        assert!(answer_parts("firefox", &ctx()).is_none());
        // A tile with no query, no payload or no math draws nothing.
        for (query, payload) in [
            (None, Some("4\n2+2")),
            (Some("2+2"), None),
            (Some("firefox"), Some("4\n2+2")),
        ] {
            assert_eq!(
                answer_tile(&tile_ctx(query, payload), &ctx()).unwrap_err(),
                "no answer"
            );
        }
    }

    #[test]
    fn the_tile_is_two_halves() {
        let frame = tile("2+2", &ctx());
        let n = &frame.nodes;
        let NodeKind::Row(row) = &n[0].kind else {
            panic!("the root is a row: {:?}", n[0]);
        };
        assert_eq!(
            (row.padding.left, row.padding.top, row.gap),
            (16., 16., 16.)
        );
        for half in [&n[1], &n[2]] {
            let NodeKind::Column(column) = &half.kind else {
                panic!("a half is a column: {half:?}");
            };
            assert_eq!((half.parent, half.grow), (Some(0), 1.));
            assert_eq!(column.justify, Justify::SpaceBetween);
        }
        let NodeKind::Column(right) = &n[2].kind else {
            unreachable!()
        };
        assert_eq!(right.align, Align::End);
        let texts = texts(&frame);
        let shown: Vec<_> = texts.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(shown, ["2+2", "Expression", "4", "Number"]);
        let [input, input_caption, result, result_caption] = texts[..] else {
            unreachable!()
        };
        assert!(input.mono && !result.mono);
        assert_eq!((input.style, input.max_lines), (TextStyle::Value, 1));
        assert_eq!((result.style, result.max_lines), (TextStyle::Value, 1));
        assert!(matches!(input.color, Color::Token(ColorToken::Text)));
        assert_eq!(result.align, TextAlign::End);
        for caption in [input_caption, result_caption] {
            assert_eq!(caption.style, TextStyle::Caption);
            assert!(matches!(caption.color, Color::Token(ColorToken::Muted)));
        }
        assert_eq!(result_caption.align, TextAlign::End);
        // Input on the left, result on the right.
        let parents: Vec<_> = n[3..].iter().map(|node| node.parent).collect();
        assert_eq!(parents, [Some(1), Some(1), Some(2), Some(2)]);
    }

    #[test]
    fn the_tile_draws_what_search_computed() {
        // The payload is what Enter copies: the tile shows it, even if the query would
        // evaluate to something else by now (`now`, new rates).
        let frame = answer_tile(&tile_ctx(Some("2+2"), Some("5\n2+2")), &ctx()).unwrap();
        let shown: Vec<_> = texts(&frame).iter().map(|t| t.text.clone()).collect();
        assert_eq!(shown, ["2+2", "Expression", "5", "Number"]);
    }

    /// The tile's captions are what search read: rendered later, with newer rates, it still
    /// says the rates its result came from were old. Another payload is read again.
    #[test]
    fn the_tile_keeps_the_captions_search_computed() {
        let now = 1_795_099_320;
        let rates = |fetched| Rates {
            fiat: Some(Table {
                fetched,
                values: [("EUR", 2.0)].into_iter().collect(),
            }),
            crypto: None,
        };
        let (old, fresh) = (rates(now - 3 * 86_400), rates(now - 60));
        let at_search = Context::new(now, zones::utc(), &old);
        let later = Context::new(now, zones::utc(), &fresh);
        let query = "1 eur in usd";
        let answer = answer(query, &at_search).unwrap().expect("an answer");

        let frame = answer_tile(&tile_ctx(Some(query), Some(&answer.payload)), &later).unwrap();

        assert_eq!(texts(&frame)[1].text, "rates from 3 days ago");
        let other = answer_tile(&tile_ctx(Some(query), Some("9\n1 eur in usd")), &later).unwrap();
        assert_eq!(texts(&other)[1].text, "Conversion");
    }

    #[test]
    fn the_tile_stays_small() {
        // Spaces inside, so that trimming keeps the query at 1 KiB.
        let query = format!("2{}+2", " ".repeat(1021));
        assert_eq!(query.len(), 1024);
        let frame = tile(&query, &ctx());
        assert!(frame.nodes.len() <= 16, "{}", frame.nodes.len());
        let texts = texts(&frame);
        assert_eq!(texts[0].text.len(), 1024);
        assert_eq!(texts[2].text, "4");
        for text in texts {
            assert!(text.text.len() <= MAX_LABEL_BYTES, "{}", text.text.len());
        }
    }

    #[test]
    fn answers_name_the_zone_they_used() {
        let shown = |q: &str| {
            let a = answer_parts(q, &ctx()).expect("an answer");
            (a.result, a.input_caption)
        };
        assert_eq!(shown("today + 1 day").1, "Europe/Moscow");
        assert_eq!(shown("days until 31 dec").1, "Europe/Moscow");
        assert_eq!(shown("now in Tokyo").1, "Conversion");
        assert_eq!(
            shown("now"),
            ("Thu, 19 Nov 2026, 17:42 MSK".into(), "Expression".into())
        );
        assert_eq!(
            shown("today"),
            ("Thu, 19 Nov 2026".into(), "Europe/Moscow".into())
        );
        assert_eq!(shown("2+2").1, "Expression");
        // A date in another zone names that zone.
        assert_eq!(
            shown("today in Tokyo"),
            ("Thu, 19 Nov 2026".into(), "Asia/Tokyo".into())
        );
        assert_eq!(shown("31 dec in Tokyo").1, "Asia/Tokyo");
        // A clock time read in the home zone and shown in another names the home zone.
        assert_eq!(
            shown("3pm in London"),
            ("Thu, 19 Nov 2026, 12:00 GMT".into(), "Europe/Moscow".into())
        );
        let utc = Context::new(1_795_099_320, zones::utc(), ctx().rates);
        assert_eq!(
            answer_parts("5pm in Tokyo", &utc).map(|a| (a.result, a.input_caption)),
            Some(("Fri, 20 Nov 2026, 02:00 JST".into(), "UTC".into()))
        );
        // `now` is the same instant in every zone, and nothing else is read from home.
        assert_eq!(
            shown("now to unix"),
            ("1795099320".into(), "Conversion".into())
        );
        assert_eq!(shown("5pm London in Moscow").1, "Conversion");
        // A 1 KiB query keeps the zone, and its input stays within the cap.
        let long = format!("today{}+ 1 day", " ".repeat(1010));
        assert_eq!(long.len(), 1022);
        let a = answer_parts(&long, &ctx()).unwrap();
        assert!(a.input.len() <= MAX_LABEL_BYTES, "{}", a.input.len());
        assert_eq!(a.input_caption, "Europe/Moscow");
    }

    #[test]
    fn old_and_future_rates_say_so() {
        let now = 1_795_099_320;
        let caption = |fiat: i64, crypto: i64, query: &str, zone: ZoneId| {
            let table = |fetched, code| Table {
                fetched,
                values: [(code, 2.0)].into_iter().collect(),
            };
            let rates = Rates {
                fiat: Some(table(fiat, "EUR")),
                crypto: Some(table(crypto, "BTC")),
            };
            let ctx = Context::new(now, zone, &rates);
            answer_parts(query, &ctx).unwrap().input_caption
        };
        let utc = zones::utc();
        assert_eq!(
            caption(now - 60, now - 60, "1 eur in usd", utc),
            "Conversion"
        );
        assert_eq!(
            caption(now - 3 * 86_400, now - 60, "1 eur in usd", utc),
            "rates from 3 days ago"
        );
        // A table dated after now (the clock was ahead when it came) has no age to show.
        assert_eq!(
            caption(now + 3600, now - 60, "1 eur in usd", utc),
            "rates of unknown age"
        );
        assert_eq!(
            caption(now - 3 * 86_400, now + 3600, "1 eur in btc", utc),
            "rates of unknown age"
        );
        // Within the refresh slack it is fresh; an unused table says nothing.
        assert_eq!(
            caption(now + 60, now - 60, "1 eur in usd", utc),
            "Conversion"
        );
        assert_eq!(
            caption(now - 60, now + 3600, "1 eur in usd", utc),
            "Conversion"
        );
        assert_eq!(caption(now - 60, now - 60, "days until 31 dec", utc), "UTC");
        // A query that reads both the zone and old rates says both.
        assert_eq!(
            caption(
                now - 3 * 86_400,
                now - 60,
                "(days until 31 dec) / (1 day) * 1 eur in usd",
                utc
            ),
            "UTC · rates from 3 days ago"
        );
        assert_eq!(
            joined([Some("UTC".into()), Some("rates from 3 days ago".into())]).as_deref(),
            Some("UTC · rates from 3 days ago")
        );
        assert_eq!(
            joined([None, Some("rates of unknown age".into())]).as_deref(),
            Some("rates of unknown age")
        );
        assert_eq!(joined([None, None]), None);
    }

    #[test]
    fn annotations_skip_empty_lines_and_mark_errors() {
        let notes = annotations("1+1\n\nx = 3\n2 *\n", &ctx());
        let got: Vec<_> = notes
            .iter()
            .map(|a| (a.line, a.text.as_str(), a.is_error))
            .collect();
        assert_eq!(got[..2], [(0, "2", false), (2, "3", false)]);
        assert_eq!((got[2].0, got[2].2), (3, true));
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn annotations_stop_below_ten_thousand_lines() {
        let text = "1\n".repeat(12_000);
        let notes = annotations(&text, &ctx());
        assert_eq!(notes.len(), 10_000);
        assert_eq!(notes.last().unwrap().line, 9_999);
    }

    #[test]
    fn evaluation_stops_at_what_can_be_shown() {
        let lines = "1\n".repeat(12_000);
        assert_eq!(evaluated(&lines).lines().count(), 10_000);
        assert_eq!(evaluated("1\n2"), "1\n2");
        // Over 256 KiB: whole lines only.
        let long = format!("{}\n{}", "1+1 ".repeat(40_000), "2+2\n".repeat(30_000));
        let cut = evaluated(&long);
        assert!(
            cut.len() <= MAX_EVAL_BYTES && cut.ends_with("2+2"),
            "{}",
            cut.len()
        );
        assert_eq!(last_result(&long, &ctx()).as_deref(), Some("4"));
        // The cut lands right before a newline: the last line is whole, and kept.
        let blank = format!("{}\n", " ".repeat(999));
        let head = blank.repeat(MAX_EVAL_BYTES / 1000);
        let last = format!("2+2{}", " ".repeat(MAX_EVAL_BYTES - head.len() - 3));
        let exact = format!("{head}{last}\n9");
        assert_eq!(exact.as_bytes()[MAX_EVAL_BYTES], b'\n');
        assert!(evaluated(&exact).ends_with(&last));
        assert_eq!(last_result(&exact, &ctx()).as_deref(), Some("4"));
    }

    #[test]
    fn last_result_skips_errors_and_empty_lines() {
        assert_eq!(
            last_result("1+1\n3*3\nnope\n", &ctx()).as_deref(),
            Some("9")
        );
        assert_eq!(last_result("a = 4\n", &ctx()).as_deref(), Some("4"));
        assert_eq!(last_result("nope\n\n", &ctx()), None);
    }

    #[test]
    fn relative_times() {
        for (seconds, want) in [
            (-5, "just now"),
            (0, "just now"),
            (59, "just now"),
            (60, "1 min ago"),
            (3599, "59 min ago"),
            (3600, "1 h ago"),
            (86_399, "23 h ago"),
            (86_400, "yesterday"),
            (2 * 86_400 - 1, "yesterday"),
            (2 * 86_400, "2 days ago"),
            (400 * 86_400, "400 days ago"),
        ] {
            assert_eq!(relative(seconds), want, "{seconds}");
        }
    }

    #[test]
    fn titles_fit_one_line() {
        assert_eq!(title("\n \n  total = 5 \nx"), "total = 5");
        assert_eq!(title(" \n\t\n"), "Empty sheet");
        let long = format!("{}\t{}", "é".repeat(600), "x");
        let t = title(&long);
        assert!(t.len() <= MAX_LABEL_BYTES && !t.contains('\t'));
        assert!(t.chars().all(|c| c == 'é'));
    }

    #[test]
    fn sheet_names_are_short() {
        assert_eq!(sheet_name(" \n\t", 40), "an empty sheet");
        assert_eq!(sheet_name("\n rent = 1200 \nfood", 40), "“rent = 1200”");
        assert_eq!(
            sheet_name(&"é".repeat(41), 40),
            format!("“{}…”", "é".repeat(40))
        );
        assert_eq!(
            sheet_name(&"é".repeat(40), 40),
            format!("“{}”", "é".repeat(40))
        );
        assert_eq!(sheet_name("abc def", 4), "“abc…”");
    }

    #[test]
    fn pushed_pads_fit_the_cap() {
        let text = "1+1\n".repeat(MAX_PAD_BYTES / 4) + "é2+2";
        let pad = pad("0", &text, &ctx()).unwrap();
        assert_eq!(pad.text.len(), MAX_PAD_BYTES);
        assert!(text.starts_with(&pad.text));
        assert_eq!(pad.annotations.len(), MAX_ANNOTATION_LINES);
    }

    #[test]
    fn truncate_keeps_characters_whole() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("aé", 2), "a");
        assert_eq!(truncate("aé", 3), "aé");
    }

    #[test]
    fn sheet_ids() {
        assert!(is_sheet_id("0") && is_sheet_id("2s"));
        assert!(!is_sheet_id("") && !is_sheet_id("A") && !is_sheet_id("a,b"));
        assert!(!is_sheet_id("../x") && !is_sheet_id(&"1".repeat(14)));
    }
}
