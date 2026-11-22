//! The `translation` tile: a double tile, like calc's answer. The text and its language on
//! the left, the translation and its language on the right. It draws from memory only.

use tomakast_plugin::{
    Align, Color, ColorToken, Justify, Layout, Node, TextAlign, TextStyle, WidgetContext,
    WidgetFrame,
};

use crate::google::{Failure, Translation};
use crate::langs;

/// The widget's id in `plugin.toml`.
pub const WIDGET: &str = "translation";
/// Its params.
pub const TEXT: &str = "text";
pub const FROM: &str = "from";
pub const TO: &str = "to";
/// Its actions: the first runs on Enter.
pub const ACTIONS: [(&str, &str); 2] =
    [("copy", "Copy Translation"), ("open", "Open in Translator")];

/// What the tile has for its request.
pub enum TileState<'a> {
    Busy,
    Done(&'a Translation),
    Failed(&'a Failure),
}

/// The tile's frame.
pub fn frame(
    _ctx: &WidgetContext,
    text: &str,
    from: Option<&str>,
    to: &str,
    state: TileState,
) -> Result<WidgetFrame, String> {
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
    let source = match (&state, from) {
        (_, Some(from)) => langs::name(from),
        (TileState::Done(t), None) => format!("{} (detected)", langs::name(&t.src)),
        (_, None) => "Detect language".to_string(),
    };
    let (result, result_caption, muted) = match state {
        TileState::Done(t) => (t.text.clone(), langs::name(to), false),
        TileState::Busy => ("Translating…".to_string(), langs::name(to), true),
        TileState::Failed(f) => ("No translation".to_string(), reason(f), true),
    };
    let frame = WidgetFrame::new(Node::row(Layout::default().padding(16.).gap(16.)));
    let root = frame.root();
    let (frame, left) = frame.child(root, half(Align::Start));
    let (frame, right) = frame.child(root, half(Align::End));
    let (frame, _) = frame.child(left, Node::text(text, TextStyle::Title).max_lines(2));
    let (frame, _) = frame.child(left, caption(source));
    let result = Node::text(result, TextStyle::Title)
        .max_lines(2)
        .align(TextAlign::End);
    let result = if muted {
        result.color(Color::token(ColorToken::Muted))
    } else {
        result
    };
    let (frame, _) = frame.child(right, result);
    let (mut frame, _) = frame.child(right, caption(result_caption).align(TextAlign::End));
    for (id, title) in ACTIONS {
        frame = frame.action(id, title)?;
    }
    Ok(frame.build())
}

fn reason(f: &Failure) -> String {
    match f {
        Failure::Refused => "Google refused".into(),
        Failure::Status(status) => format!("Google answered {status}"),
        Failure::Offline => "Offline".into(),
        Failure::Unexpected => "Unexpected answer".into(),
    }
}

/// The storage key of a Home tile's translation: `home:` and the FNV-1a 64 of
/// `from|to|text`, in hex.
pub fn home_key(from: Option<&str>, to: &str, text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{}|{to}|{text}", from.unwrap_or("")).bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("home:{hash:016x}")
}

/// A Home tile's stored translation: the detected source, a line break, the translation.
pub fn home_value(t: &Translation) -> String {
    format!("{}\n{}", t.src, t.text)
}

/// The translation in a stored Home value.
pub fn from_home_value(value: &str) -> Option<Translation> {
    let (src, text) = value.split_once('\n')?;
    Some(Translation {
        text: text.to_string(),
        src: src.to_string(),
        ..Translation::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::tests::{RUN, answer};
    use tomakast_plugin::NodeKind;

    fn context() -> WidgetContext {
        // Only the frame's texts are checked; the context is not read.
        crate::tile::tests::empty_context()
    }

    pub fn empty_context() -> WidgetContext {
        WidgetContext {
            width: 718.,
            height: 120.,
            scale: 2.,
            theme: tomakast_plugin::WidgetTheme {
                dark: true,
                colors: tomakast_plugin::ThemeColors {
                    text: 0,
                    muted: 0,
                    panel: 0,
                    border: 0,
                    selected: 0,
                    error: 0,
                    success: 0,
                },
                ui_font: String::new(),
                mono_font: String::new(),
                sizes: tomakast_plugin::TextSizes {
                    title: 16.,
                    value: 28.,
                    body: 14.,
                    caption: 12.,
                },
            },
            millis_shown: 0,
            focused: None,
            pointer: None,
            params: Vec::new(),
            query: None,
        }
    }

    fn texts(frame: &WidgetFrame) -> Vec<String> {
        frame
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_translation() {
        let run = answer(RUN);
        let frame = frame(&context(), "run", None, "ru", TileState::Done(&run)).unwrap();
        assert_eq!(
            texts(&frame),
            ["run", "English (detected)", "бегать", "Russian"]
        );
        let ids: Vec<&str> = frame.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["copy", "open"]);
    }

    #[test]
    fn waiting_and_failing() {
        let busy = frame(&context(), "hello", None, "ru", TileState::Busy).unwrap();
        assert!(texts(&busy).contains(&"Translating…".to_string()));
        let failed = frame(
            &context(),
            "hello",
            Some("en"),
            "ru",
            TileState::Failed(&Failure::Refused),
        )
        .unwrap();
        assert_eq!(
            texts(&failed),
            ["hello", "English", "No translation", "Google refused"]
        );
    }

    #[test]
    fn home_keys_and_values() {
        assert_eq!(home_key(None, "ru", "hi"), home_key(None, "ru", "hi"));
        assert_ne!(home_key(None, "ru", "hi"), home_key(None, "de", "hi"));
        assert_ne!(home_key(Some("en"), "ru", "hi"), home_key(None, "ru", "hi"));
        assert!(home_key(None, "ru", "hi").starts_with("home:"));
        let t = from_home_value(&home_value(&answer(RUN))).unwrap();
        assert_eq!((t.src.as_str(), t.text.as_str()), ("en", "бегать"));
        assert_eq!(from_home_value("no break"), None);
    }
}
