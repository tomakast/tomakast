//! The host's checks of what a plugin's widgets send (`plugin.wit`, `widget-answer`,
//! `widget-frame` and `widget-image`): the strings of a frame, the widget answers of a
//! `search`, the images and the plugin's image budget. The structure and the numbers of a frame
//! are [`tomakast_core::check_frame`]'s.
//!
//! An SVG is parsed and checked here, before the launcher sees it: it may name nothing outside
//! itself but `data:` PNG and JPEG images.

use std::collections::{HashMap, HashSet};

use roxmltree::{Document, NodeType, ParsingOptions};
use tomakast_core::widget::{
    Draw, Frame, ImageFormat, MAX_IMAGE_BYTES, MAX_IMAGES, MAX_IMAGES_BYTES, MAX_PAYLOAD_BYTES,
    MAX_SVG_BYTES, MAX_WIDGET_ANSWERS, NodeKind, WidgetAnswer, WidgetDecl, WidgetImage,
};

use crate::caps::{CapError, Checked, actions, bytes, fail, icon, id, label, opt_label};

/// Bytes of a text node's text.
pub const MAX_TEXT_NODE_BYTES: usize = 4 * 1024;
pub use tomakast_core::widget::{
    MAX_SVG_CONVOLVE_ORDER, MAX_SVG_ELEMENTS, MAX_SVG_FILTERS, MAX_SVG_IMAGES,
    MAX_SVG_MORPHOLOGY_RADIUS, MAX_SVG_NODES, MAX_SVG_OCTAVES,
};

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPEG_MAGIC: &[u8] = b"\xff\xd8";
/// What an SVG `href` may start with.
const SVG_HREFS: [&str; 3] = ["#", "data:image/png;base64,", "data:image/jpeg;base64,"];

/// The widget answers of one `search`: at most [`MAX_WIDGET_ANSWERS`], each naming a declared
/// widget that no other answer names, with params of keys that widget declares, each once,
/// their values at most [`MAX_PAYLOAD_BYTES`] together, a size the widget offers (or none),
/// and valid actions. The params are checked as the plugin sent them, a repeated key included:
/// the core trusts what passes.
pub fn check_answers(declared: &[WidgetDecl], answers: &[WidgetAnswer]) -> Checked {
    if answers.len() > MAX_WIDGET_ANSWERS {
        return fail(format!(
            "search answered {} widget answers, at most {MAX_WIDGET_ANSWERS}",
            answers.len()
        ));
    }
    let mut seen = HashSet::new();
    for answer in answers {
        let decl = is_declared(declared, &answer.widget)?;
        if !seen.insert(answer.widget.as_str()) {
            return fail(format!("widget {:?} is answered twice", answer.widget));
        }
        params(decl, answer)?;
        if let Some(size) = answer.size
            && !decl.sizes.contains(&size)
        {
            return fail(format!(
                "widget {:?} is not offered in size {:?}",
                decl.id,
                size.as_str()
            ));
        }
        actions("widget answer", &answer.actions)?;
    }
    Ok(())
}

/// `answers` as the core takes them: [`check_answers`], then each answer's params sorted
/// by key ([`params`](tomakast_core::widget::params)), as the core's form wants.
pub fn checked_answers(
    declared: &[WidgetDecl],
    mut answers: Vec<WidgetAnswer>,
) -> Result<Vec<WidgetAnswer>, CapError> {
    check_answers(declared, &answers)?;
    for answer in &mut answers {
        answer.params = tomakast_core::widget::params(answer.params.iter().cloned());
    }
    Ok(answers)
}

/// The params of `answer`, a tile of `decl`: declared keys, each once, the values at most
/// [`MAX_PAYLOAD_BYTES`] together.
fn params(decl: &WidgetDecl, answer: &WidgetAnswer) -> Checked {
    let mut total = 0;
    for (at, (key, value)) in answer.params.iter().enumerate() {
        if !decl.params.contains(key) {
            return fail(format!(
                "widget {:?} declares no param {:?}",
                decl.id,
                key.chars().take(64).collect::<String>()
            ));
        }
        if answer.params[..at].iter().any(|(k, _)| k == key) {
            return fail(format!("widget {:?} has param {key:?} twice", decl.id));
        }
        total += value.len();
    }
    if total > MAX_PAYLOAD_BYTES {
        return fail(format!(
            "the params of widget {:?} are {total} bytes, at most {MAX_PAYLOAD_BYTES}",
            decl.id
        ));
    }
    Ok(())
}

/// The declaration of `widget` under `[[widgets]]`.
pub(crate) fn is_declared<'a>(
    declared: &'a [WidgetDecl],
    widget: &str,
) -> Result<&'a WidgetDecl, CapError> {
    match declared.iter().find(|d| d.id == widget) {
        Some(decl) => Ok(decl),
        None => fail(format!(
            "widget {:?} is not declared in plugin.toml",
            widget.chars().take(64).collect::<String>()
        )),
    }
}

/// The strings of `frame`, by the plugin-string rules: node ids, fonts and image names follow
/// the id rule; a text node holds at most [`MAX_TEXT_NODE_BYTES`] and no control character but
/// `\n`; button labels and canvas texts are labels; icons and actions follow their own rules.
/// The frame's `images` are [`check_image`]'s.
pub fn check_frame_text(frame: &Frame) -> Checked {
    for node in &frame.nodes {
        id("node", &node.id)?;
        match &node.kind {
            NodeKind::Text(text) => {
                bytes("text node", &text.text, MAX_TEXT_NODE_BYTES)?;
                if text.text.chars().any(|c| c != '\n' && c.is_control()) {
                    return fail("text node has control characters other than a newline".into());
                }
            }
            NodeKind::Icon(node) => icon(Some(&node.icon))?,
            NodeKind::Image(image) => id("image", &image.image)?,
            NodeKind::Button(button) => {
                opt_label("button label", &button.label)?;
                icon(button.icon.as_ref())?;
                id("button action", &button.action)?;
            }
            NodeKind::Canvas(canvas) => {
                for command in &canvas.commands {
                    match command {
                        Draw::Text(text) => {
                            label("canvas text", &text.text)?;
                            if let Some(font) = &text.font {
                                id("font", font)?;
                            }
                        }
                        Draw::Image(image) | Draw::Svg(image) => id("image", &image.image)?,
                        _ => {}
                    }
                }
            }
            NodeKind::Row(_)
            | NodeKind::Column(_)
            | NodeKind::Overlay(_)
            | NodeKind::Progress(_)
            | NodeKind::Spacer => {}
        }
    }
    actions("frame", &frame.actions)
}

/// One image: its id follows the id rule; a PNG starts with the PNG signature and a JPEG with
/// `FF D8`, each at most [`MAX_IMAGE_BYTES`]; an SVG is UTF-8 of at most [`MAX_SVG_BYTES`]
/// and passes [`check_svg`].
pub fn check_image(image: &WidgetImage) -> Checked {
    id("image", &image.id)?;
    let size = image.bytes.len();
    let (max, magic, name) = match image.format {
        ImageFormat::Png => (MAX_IMAGE_BYTES, PNG_MAGIC, "PNG"),
        ImageFormat::Jpeg => (MAX_IMAGE_BYTES, JPEG_MAGIC, "JPEG"),
        ImageFormat::Svg => (MAX_SVG_BYTES, &[][..], "SVG"),
    };
    if size > max {
        return fail(format!(
            "{name} image {:?} is {size} bytes, at most {max}",
            image.id
        ));
    }
    if !image.bytes.starts_with(magic) {
        return fail(format!("image {:?} is not a {name}", image.id));
    }
    if image.format == ImageFormat::Svg {
        check_svg(&image.bytes)?;
    }
    Ok(())
}

/// An SVG document that names nothing outside itself: it parses (no DTD, at most
/// [`MAX_SVG_NODES`] nodes); it has at most
/// [`MAX_SVG_ELEMENTS`] elements, [`MAX_SVG_FILTERS`] filter primitives and [`MAX_SVG_IMAGES`]
/// `<image>` elements; an `feTurbulence` has at most [`MAX_SVG_OCTAVES`] octaves, an
/// `feMorphology` a radius of at most [`MAX_SVG_MORPHOLOGY_RADIUS`] (in its own units: the
/// launcher checks it again in device pixels), an `feConvolveMatrix` an order of at most
/// [`MAX_SVG_CONVOLVE_ORDER`]; no `script` or `foreignObject`; every `href` (with or without the `xlink` prefix) starts with `#`,
/// `data:image/png;base64,` or `data:image/jpeg;base64,`; no attribute and no `<style>` text
/// holds `url(` but `url(#` or holds `@import`; no `xml-stylesheet` instruction.
pub fn check_svg(bytes: &[u8]) -> Checked {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return fail("an SVG image is not UTF-8".into());
    };
    // DTDs stay refused (the default).
    let options = ParsingOptions {
        nodes_limit: MAX_SVG_NODES as u32,
        ..ParsingOptions::default()
    };
    let doc = match Document::parse_with_options(text, options) {
        Ok(doc) => doc,
        Err(roxmltree::Error::NodesLimitReached) => {
            return fail(format!("an SVG image has more than {MAX_SVG_NODES} nodes"));
        }
        Err(err) => return fail(format!("an SVG image does not parse: {err}")),
    };
    let mut elements = 0;
    let mut filters = 0;
    let mut images = 0;
    for node in doc.descendants() {
        match node.node_type() {
            NodeType::Element => {
                elements += 1;
                if elements > MAX_SVG_ELEMENTS {
                    return fail(format!(
                        "an SVG image has more than {MAX_SVG_ELEMENTS} elements"
                    ));
                }
                // Local names: roxmltree resolves the prefixes (`svg:script`, `xlink:href`).
                let name = node.tag_name().name();
                if name.starts_with("fe") {
                    filters += 1;
                    if filters > MAX_SVG_FILTERS {
                        return fail(format!(
                            "an SVG image has more than {MAX_SVG_FILTERS} filter primitives"
                        ));
                    }
                }
                if name == "image" {
                    images += 1;
                    if images > MAX_SVG_IMAGES {
                        return fail(format!(
                            "an SVG image has more than {MAX_SVG_IMAGES} images"
                        ));
                    }
                }
                filter_cost(node)?;
                if name == "script" || name == "foreignObject" {
                    return fail(format!("an SVG image has a <{name}> element"));
                }
                for attr in node.attributes() {
                    let value = attr.value();
                    if attr.name() == "href" && !SVG_HREFS.iter().any(|p| value.starts_with(p)) {
                        return fail(format!(
                            "an SVG image links to {:?}: only #fragments and data: PNG or JPEG \
                             images may be named",
                            value.chars().take(64).collect::<String>()
                        ));
                    }
                    css(value)?;
                }
            }
            NodeType::Text
                if node
                    .parent_element()
                    .is_some_and(|p| p.tag_name().name() == "style") =>
            {
                css(node.text().unwrap_or_default())?;
            }
            NodeType::PI if node.pi().is_some_and(|pi| pi.target == "xml-stylesheet") => {
                return fail("an SVG image names a stylesheet".into());
            }
            _ => {}
        }
    }
    Ok(())
}

/// The numbers of an SVG filter primitive that set its cost: `numOctaves`, a morphology
/// `radius`, a convolution `order`. A value that is not a number is refused.
fn filter_cost(node: roxmltree::Node) -> Checked {
    let (attr, max) = match node.tag_name().name() {
        "feTurbulence" => ("numOctaves", MAX_SVG_OCTAVES as f32),
        "feMorphology" => ("radius", MAX_SVG_MORPHOLOGY_RADIUS),
        "feConvolveMatrix" => ("order", MAX_SVG_CONVOLVE_ORDER as f32),
        _ => return Ok(()),
    };
    let Some(value) = node.attribute(attr) else {
        return Ok(());
    };
    let numbers = value
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|part| !part.is_empty());
    for number in numbers {
        if !number.parse::<f32>().is_ok_and(|n| n <= max) {
            return fail(format!(
                "an SVG <{}> has {attr} {:?}: at most {max}",
                node.tag_name().name(),
                value.chars().take(32).collect::<String>()
            ));
        }
    }
    Ok(())
}

/// Style text or an attribute value: no `url(` but `url(#…)`, no `@import` (in any case).
fn css(text: &str) -> Checked {
    let lower = text.to_ascii_lowercase();
    if lower.contains("@import") {
        return fail("an SVG image imports a stylesheet".into());
    }
    for (at, _) in lower.match_indices("url(") {
        let rest = lower[at + 4..].trim_start();
        let rest = rest.strip_prefix(['"', '\'']).unwrap_or(rest);
        if !rest.starts_with('#') {
            return fail("an SVG image names a url() that is not a #fragment".into());
        }
    }
    Ok(())
}

/// The images a plugin has stored, by id: at most [`MAX_IMAGES`] and [`MAX_IMAGES_BYTES`] in
/// all. An image whose id is stored replaces it, and its old bytes are freed first.
#[derive(Debug, Default, Clone)]
pub struct ImageBudget {
    sizes: HashMap<String, usize>,
    total: usize,
}

impl ImageBudget {
    /// Counts `image` in, or says why it does not fit; a refused image changes nothing.
    pub fn admit(&mut self, image: &WidgetImage) -> Result<(), CapError> {
        let old = self.sizes.get(&image.id).copied();
        if old.is_none() && self.sizes.len() >= MAX_IMAGES {
            return fail(format!(
                "image {:?} would make more than {MAX_IMAGES} images",
                image.id
            ));
        }
        let total = self.total - old.unwrap_or(0) + image.bytes.len();
        if total > MAX_IMAGES_BYTES {
            return fail(format!(
                "image {:?} would make the images {total} bytes, at most {MAX_IMAGES_BYTES}",
                image.id
            ));
        }
        self.sizes.insert(image.id.clone(), image.bytes.len());
        self.total = total;
        Ok(())
    }

    /// Counts in every image of `images`, or none of them.
    pub(crate) fn admit_all(&mut self, images: &[WidgetImage]) -> Checked {
        if images.is_empty() {
            return Ok(());
        }
        let mut trial = self.clone();
        images.iter().try_for_each(|image| trial.admit(image))?;
        *self = trial;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tomakast_core::widget::{
        ButtonNode, CanvasNode, IconNode, ImageCmd, ImageFit, ImageNode, Layout, Node, TextAlign,
        TextCmd, TextNode, TextStyle, WidgetSize, params,
    };
    use tomakast_core::{Action, Icon};

    /// A widget in the sizes single and quarter, with the params `city` and `units`.
    fn decl(id: &str) -> WidgetDecl {
        WidgetDecl {
            id: id.into(),
            title: id.into(),
            sizes: vec![WidgetSize::Single, WidgetSize::Quarter],
            params: vec!["city".into(), "units".into()],
        }
    }

    fn answer(widget: &str) -> WidgetAnswer {
        WidgetAnswer {
            widget: widget.into(),
            params: params([("city".to_string(), "Oslo".to_string())]),
            size: None,
            actions: vec![],
        }
    }

    /// `answer("w")` with `pairs` as sent, in order, a repeated key kept.
    fn with_params(pairs: &[(&str, &str)]) -> WidgetAnswer {
        WidgetAnswer {
            params: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..answer("w")
        }
    }

    fn act(id: &str) -> Action {
        Action {
            id: id.into(),
            title: "A".into(),
            shortcut: None,
        }
    }

    #[test]
    fn answers_name_declared_widgets_once() {
        let declared: Vec<WidgetDecl> = (0..20).map(|i| decl(&format!("w{i}"))).collect();
        assert!(check_answers(&declared, &[answer("w0"), answer("w1")]).is_ok());
        let err = check_answers(&declared, &[answer("nope")]).unwrap_err();
        assert!(err.0.contains("\"nope\" is not declared"), "{err}");
        let err = check_answers(&declared, &[answer("w0"), answer("w0")]).unwrap_err();
        assert!(err.0.contains("answered twice"), "{err}");
    }

    #[test]
    fn at_most_16_answers() {
        let declared: Vec<WidgetDecl> = (0..17).map(|i| decl(&format!("w{i}"))).collect();
        let answers: Vec<WidgetAnswer> = (0..17).map(|i| answer(&format!("w{i}"))).collect();
        assert!(check_answers(&declared, &answers[..16]).is_ok());
        let err = check_answers(&declared, &answers).unwrap_err();
        assert!(err.0.contains("17 widget answers, at most 16"), "{err}");
    }

    #[test]
    fn an_undeclared_param_is_rejected() {
        let declared = [decl("w")];
        let both = with_params(&[("units", "metric"), ("city", "Oslo")]);
        assert!(check_answers(&declared, &[both]).is_ok());
        assert!(check_answers(&declared, &[with_params(&[])]).is_ok());
        let err = check_answers(&declared, &[with_params(&[("days", "3")])]).unwrap_err();
        assert!(err.0.contains("\"days\""), "{err}");
        assert!(err.0.contains("\"w\""), "{err}");
    }

    #[test]
    fn a_repeated_param_is_rejected() {
        let declared = [decl("w")];
        let twice = with_params(&[("city", "Oslo"), ("units", "metric"), ("city", "Rome")]);
        let err = check_answers(&declared, &[twice]).unwrap_err();
        assert!(err.0.contains("\"city\" twice"), "{err}");
    }

    /// The caps check sees what the plugin sent: the conversion keeps a repeated key.
    #[test]
    fn a_repeated_param_from_the_plugin_reaches_the_check() {
        use crate::bindings::tomakast::plugin::types as wit;
        let sent = wit::WidgetAnswer {
            id: "w".into(),
            params: ["city", "city"]
                .map(|key| wit::WidgetParam {
                    key: key.into(),
                    value: "Oslo".into(),
                })
                .into(),
            size: None,
            actions: vec![],
        };
        assert!(check_answers(&[decl("w")], &[WidgetAnswer::from(sent)]).is_err());
    }

    #[test]
    fn checked_answers_come_out_sorted_by_key() {
        let sent = with_params(&[("units", "metric"), ("city", "Oslo")]);
        let checked = checked_answers(&[decl("w")], vec![sent]).unwrap();
        assert_eq!(
            checked[0].params,
            params([
                ("city".to_string(), "Oslo".to_string()),
                ("units".to_string(), "metric".to_string()),
            ])
        );
        let keys: Vec<&str> = checked[0].params.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["city", "units"]);
        let twice = with_params(&[("city", "Oslo"), ("city", "Rome")]);
        assert!(checked_answers(&[decl("w")], vec![twice]).is_err());
    }

    #[test]
    fn params_over_4096_bytes_are_rejected() {
        let declared = [decl("w")];
        let half = "x".repeat(MAX_PAYLOAD_BYTES / 2);
        let full = with_params(&[("city", &half), ("units", &half)]);
        assert!(check_answers(&declared, &[full]).is_ok());
        let over = format!("{half}x");
        let err = check_answers(
            &declared,
            &[with_params(&[("city", &half), ("units", &over)])],
        )
        .unwrap_err();
        assert!(err.0.contains("4096"), "{err}");
    }

    #[test]
    fn an_unoffered_size_is_rejected() {
        let declared = [decl("w")];
        for size in [None, Some(WidgetSize::Single), Some(WidgetSize::Quarter)] {
            let a = WidgetAnswer {
                size,
                ..answer("w")
            };
            assert!(check_answers(&declared, &[a]).is_ok(), "{size:?}");
        }
        let a = WidgetAnswer {
            size: Some(WidgetSize::Double),
            ..answer("w")
        };
        let err = check_answers(&declared, &[a]).unwrap_err();
        assert!(err.0.contains("double"), "{err}");
    }

    #[test]
    fn actions_are_checked() {
        let declared = [decl("w")];
        let mut a = answer("w");
        a.actions = (0..17).map(|i| act(&format!("a{i}"))).collect();
        assert!(check_answers(&declared, &[a]).is_err());
        let mut a = answer("w");
        a.actions = vec![act("bad\u{202e}")];
        assert!(check_answers(&declared, &[a]).is_err());
        let mut a = answer("w");
        a.actions = vec![Action {
            shortcut: Some("ctrl+\nx".into()),
            ..act("a")
        }];
        assert!(check_answers(&declared, &[a]).is_err());
        // The launcher's own "Add to Home" cannot be forged.
        let mut a = answer("w");
        a.actions = vec![Action {
            title: "add to home".into(),
            ..act("a")
        }];
        assert!(check_answers(&declared, &[a]).is_err());
    }

    fn node(id: &str, parent: Option<u32>, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            parent,
            grow: 0.,
            width: None,
            height: None,
            kind,
        }
    }

    fn text_node(text: &str) -> NodeKind {
        NodeKind::Text(TextNode {
            text: text.into(),
            style: TextStyle::Body,
            color: tomakast_core::widget::Color::Rgba(0),
            align: TextAlign::Start,
            max_lines: 1,
            mono: false,
        })
    }

    fn frame_with(kind: NodeKind) -> Frame {
        Frame {
            nodes: vec![
                node(
                    "",
                    None,
                    NodeKind::Column(Layout {
                        padding: tomakast_core::widget::Edges {
                            top: 0.,
                            right: 0.,
                            bottom: 0.,
                            left: 0.,
                        },
                        gap: 0.,
                        align: tomakast_core::widget::Align::Start,
                        justify: tomakast_core::widget::Justify::Start,
                    }),
                ),
                node("n", Some(0), kind),
            ],
            animate: false,
            actions: vec![],
            images: vec![],
        }
    }

    fn text_cmd(text: &str, font: Option<&str>) -> NodeKind {
        NodeKind::Canvas(CanvasNode {
            focusable: false,
            commands: vec![Draw::Text(TextCmd {
                x: 0.,
                y: 0.,
                text: text.into(),
                size: 12.,
                color: tomakast_core::widget::Color::Rgba(0),
                font: font.map(str::to_string),
                mono: false,
                opacity: 1.,
            })],
        })
    }

    fn button(label: &str, icon: Option<Icon>, action: &str) -> NodeKind {
        NodeKind::Button(ButtonNode {
            label: Some(label.into()),
            icon,
            action: action.into(),
        })
    }

    fn image_cmd(image: &str) -> NodeKind {
        NodeKind::Canvas(CanvasNode {
            focusable: false,
            commands: vec![Draw::Svg(ImageCmd {
                image: image.into(),
                x: 0.,
                y: 0.,
                width: 1.,
                height: 1.,
                opacity: 1.,
            })],
        })
    }

    fn ok(kind: NodeKind) -> bool {
        check_frame_text(&frame_with(kind)).is_ok()
    }

    #[test]
    fn text_nodes_are_4_kib_with_newlines_only() {
        assert!(ok(text_node(&"x".repeat(MAX_TEXT_NODE_BYTES))));
        assert!(!ok(text_node(&"x".repeat(MAX_TEXT_NODE_BYTES + 1))));
        assert!(ok(text_node("two\nlines")));
        for bad in ["tab\there", "cr\r", "esc\u{1b}[31m", "nul\0"] {
            assert!(!ok(text_node(bad)), "{bad:?}");
        }
    }

    #[test]
    fn labels_are_one_line_of_1_kib() {
        assert!(ok(text_cmd(&"x".repeat(1024), None)));
        assert!(!ok(text_cmd(&"x".repeat(1025), None)));
        assert!(!ok(text_cmd("two\nlines", None)));
        assert!(ok(button("Go", None, "go")));
        assert!(!ok(button("two\nlines", None, "go")));
        assert!(!ok(button(&"x".repeat(1025), None, "go")));
    }

    #[test]
    fn ids_fonts_and_image_names_follow_the_id_rule() {
        let mut frame = frame_with(NodeKind::Spacer);
        frame.nodes[1].id = "n\u{200b}".into();
        assert!(check_frame_text(&frame).is_err());
        assert!(ok(text_cmd("t", Some("Inter"))));
        assert!(!ok(text_cmd("t", Some("Inter\n"))));
        assert!(!ok(button("Go", None, "go\u{202e}")));
        let image = |name: &str| {
            NodeKind::Image(ImageNode {
                image: name.into(),
                fit: ImageFit::Contain,
                radius: 0.,
            })
        };
        assert!(ok(image("logo")));
        assert!(!ok(image(&"x".repeat(257))));
        assert!(ok(image_cmd("logo")));
        assert!(!ok(image_cmd("logo\u{7}")));
    }

    #[test]
    fn icons_and_actions_follow_their_rules() {
        let icon_node = |icon: Icon| NodeKind::Icon(IconNode { icon, size: 16. });
        assert!(ok(icon_node(Icon::Named("face-smile".into()))));
        assert!(!ok(icon_node(Icon::Color("red".into()))));
        assert!(!ok(icon_node(Icon::PluginFile("../out.png".into()))));
        assert!(!ok(icon_node(Icon::File("/etc/passwd".into()))));
        assert!(!ok(button(
            "Go",
            Some(Icon::Emoji("\u{200b}".into())),
            "go"
        )));

        let mut frame = frame_with(NodeKind::Spacer);
        frame.actions = vec![act("a")];
        assert!(check_frame_text(&frame).is_ok());
        frame.actions = (0..17).map(|i| act(&format!("a{i}"))).collect();
        assert!(check_frame_text(&frame).is_err());
        frame.actions = vec![Action {
            title: "Add to Home".into(),
            ..act("a")
        }];
        assert!(check_frame_text(&frame).is_err());
    }

    fn image(format: ImageFormat, bytes: &[u8]) -> WidgetImage {
        WidgetImage {
            id: "i".into(),
            format,
            bytes: Arc::from(bytes),
        }
    }

    fn png(len: usize) -> Vec<u8> {
        let mut bytes = PNG_MAGIC.to_vec();
        bytes.resize(len, 0);
        bytes
    }

    #[test]
    fn images_are_what_their_format_says() {
        assert!(check_image(&image(ImageFormat::Png, &png(64))).is_ok());
        assert!(check_image(&image(ImageFormat::Jpeg, b"\xff\xd8\xff\xe0")).is_ok());
        assert!(check_image(&image(ImageFormat::Png, b"\xff\xd8\xff\xe0")).is_err());
        assert!(check_image(&image(ImageFormat::Jpeg, &png(64))).is_err());
        assert!(check_image(&image(ImageFormat::Png, b"")).is_err());
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#;
        assert!(check_image(&image(ImageFormat::Svg, svg)).is_ok());
        assert!(check_image(&image(ImageFormat::Svg, &png(64))).is_err());
        assert!(check_image(&image(ImageFormat::Svg, b"<svg>\xff</svg>")).is_err());
        let mut bad_id = image(ImageFormat::Png, &png(64));
        bad_id.id = "i\n".into();
        assert!(check_image(&bad_id).is_err());
    }

    #[test]
    fn images_are_at_most_2_mib_and_svg_512_kib() {
        assert!(check_image(&image(ImageFormat::Png, &png(MAX_IMAGE_BYTES))).is_ok());
        assert!(check_image(&image(ImageFormat::Png, &png(MAX_IMAGE_BYTES + 1))).is_err());
        let mut jpeg = JPEG_MAGIC.to_vec();
        jpeg.resize(MAX_IMAGE_BYTES + 1, 0);
        assert!(check_image(&image(ImageFormat::Jpeg, &jpeg)).is_err());
        let svg = |len: usize| {
            let head = r#"<svg xmlns="http://www.w3.org/2000/svg"><!--"#;
            let tail = "--></svg>";
            format!("{head}{}{tail}", "x".repeat(len - head.len() - tail.len()))
        };
        assert!(check_image(&image(ImageFormat::Svg, svg(MAX_SVG_BYTES).as_bytes())).is_ok());
        assert!(check_image(&image(ImageFormat::Svg, svg(MAX_SVG_BYTES + 1).as_bytes())).is_err());
    }

    fn svg(body: &str) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">{body}</svg>"#
        )
    }

    fn svg_ok(body: &str) -> bool {
        check_svg(svg(body).as_bytes()).is_ok()
    }

    #[test]
    fn svg_names_nothing_outside_itself() {
        assert!(svg_ok(r##"<use href="#a"/><use xlink:href="#a"/>"##));
        assert!(svg_ok(
            r#"<image href="data:image/png;base64,iVBORw0KGgo="/>"#
        ));
        assert!(svg_ok(
            r#"<image xlink:href="data:image/jpeg;base64,/9j/"/>"#
        ));
        for href in [
            "/etc/hostname",
            "file:///etc/hostname",
            "https://tracker.example/pixel.png",
            "data:image/svg+xml;base64,PHN2Zy8+",
            " #a",
            "other.svg#a",
        ] {
            assert!(!svg_ok(&format!(r#"<image href="{href}"/>"#)), "{href}");
            assert!(
                !svg_ok(&format!(r#"<image xlink:href="{href}"/>"#)),
                "{href}"
            );
        }
    }

    #[test]
    fn svg_has_no_script_or_foreign_object() {
        assert!(!svg_ok("<script>alert(1)</script>"));
        assert!(!svg_ok(
            r#"<s:script xmlns:s="http://www.w3.org/2000/svg">x</s:script>"#
        ));
        assert!(!svg_ok("<foreignObject><div/></foreignObject>"));
    }

    #[test]
    fn svg_css_has_no_external_url_or_import() {
        assert!(svg_ok(
            r##"<rect fill="url(#g)"/><rect fill="url( '#g')"/>"##
        ));
        assert!(svg_ok(r##"<style>rect { fill: url(#g) }</style>"##));
        for bad in [
            r#"<rect fill="url(https://x.example/a)"/>"#,
            r#"<rect style="fill: URL('/etc/hostname')"/>"#,
            r#"<rect filter="url(other.svg#f)"/>"#,
            "<style>rect { fill: url(https://x.example/a) }</style>",
            "<style><![CDATA[@import 'https://x.example/a.css';]]></style>",
            "<style>@IMPORT url(#a);</style>",
        ] {
            assert!(!svg_ok(bad), "{bad}");
        }
        let stylesheet = format!(
            r#"<?xml-stylesheet href="https://x.example/a.css"?>{}"#,
            svg("")
        );
        assert!(check_svg(stylesheet.as_bytes()).is_err());
    }

    #[test]
    fn svg_parses_without_a_dtd() {
        assert!(check_svg(b"<svg").is_err());
        let entity = format!(
            r#"<!DOCTYPE svg [<!ENTITY x "x">]>{}"#,
            svg("<text>&x;</text>")
        );
        assert!(check_svg(entity.as_bytes()).is_err());
    }

    #[test]
    fn svg_has_at_most_4096_elements_and_32_filter_primitives() {
        // The root counts.
        assert!(svg_ok(&"<g/>".repeat(MAX_SVG_ELEMENTS - 1)));
        assert!(!svg_ok(&"<g/>".repeat(MAX_SVG_ELEMENTS)));
        let filter = |n: usize| format!("<filter>{}</filter>", "<feFlood/>".repeat(n));
        assert!(svg_ok(&filter(MAX_SVG_FILTERS)));
        assert!(!svg_ok(&filter(MAX_SVG_FILTERS + 1)));
    }

    /// The cheap parts of the launcher's cost model: turbulence octaves,
    /// morphology radius, convolution order and `<image>` elements.
    #[test]
    fn svg_filters_and_images_stay_cheap() {
        let fe = |primitive: &str| format!("<filter>{primitive}</filter>");
        assert!(svg_ok(&fe(r#"<feTurbulence numOctaves="10"/>"#)));
        assert!(!svg_ok(&fe(r#"<feTurbulence numOctaves="11"/>"#)));
        assert!(!svg_ok(&fe(r#"<feTurbulence numOctaves="many"/>"#)));
        assert!(svg_ok(&fe(r#"<feMorphology radius="32 2"/>"#)));
        assert!(!svg_ok(&fe(r#"<feMorphology radius="2,33"/>"#)));
        assert!(svg_ok(&fe(
            r#"<feConvolveMatrix order="9" kernelMatrix="1"/>"#
        )));
        assert!(!svg_ok(&fe(
            r#"<feConvolveMatrix order="3 10" kernelMatrix="1"/>"#
        )));
        let png = format!(r#"<image href="data:image/png;base64,{}"/>"#, "AAAA");
        assert!(svg_ok(&png.repeat(MAX_SVG_IMAGES)));
        let err = check_svg(svg(&png.repeat(MAX_SVG_IMAGES + 1)).as_bytes()).unwrap_err();
        assert!(err.0.contains("more than 16 images"), "{err}");
    }

    #[test]
    fn svg_parsing_stops_at_the_node_limit() {
        // A formatted document at the element cap parses: its text nodes and comments fit.
        let mut body = "\n  <g/>".repeat(MAX_SVG_ELEMENTS - 2);
        body.push_str("\n  <g><!-- one --><!-- two --></g>\n");
        let formatted = svg(&body);
        assert!(check_svg(formatted.as_bytes()).is_ok());
        // A hostile one stops parsing at the node limit, before the element count.
        let hostile = svg(&"<g/>".repeat(100_000));
        let err = check_svg(hostile.as_bytes()).unwrap_err();
        assert!(
            err.0.contains(&format!("more than {MAX_SVG_NODES} nodes")),
            "{err}"
        );
    }

    fn named(id: &str, len: usize) -> WidgetImage {
        WidgetImage {
            id: id.into(),
            format: ImageFormat::Png,
            bytes: Arc::from(png(len)),
        }
    }

    #[test]
    fn the_budget_holds_64_images() {
        let mut budget = ImageBudget::default();
        for i in 0..MAX_IMAGES {
            budget.admit(&named(&format!("i{i}"), 16)).unwrap();
        }
        let err = budget.admit(&named("one-more", 16)).unwrap_err();
        assert!(err.0.contains("more than 64 images"), "{err}");
        // Replacing a stored id is no new image.
        budget.admit(&named("i0", 32)).unwrap();
        budget.admit(&named("i0", 32)).unwrap();
        assert_eq!(budget.sizes.len(), MAX_IMAGES);
    }

    #[test]
    fn the_budget_holds_8_mib_and_a_replacement_frees_first() {
        let mut budget = ImageBudget::default();
        let quarter = MAX_IMAGES_BYTES / 4;
        for i in 0..4 {
            budget.admit(&named(&format!("i{i}"), quarter)).unwrap();
        }
        assert!(budget.admit(&named("more", 1)).is_err());
        // The same id, a byte larger: its old bytes are freed, then the new ones do not fit.
        assert!(budget.admit(&named("i0", quarter + 1)).is_err());
        assert_eq!(
            budget.total, MAX_IMAGES_BYTES,
            "a refused image changes nothing"
        );
        // Smaller: fits, and frees room for a new one.
        budget.admit(&named("i0", quarter - 10)).unwrap();
        budget.admit(&named("more", 10)).unwrap();
        assert_eq!(budget.total, MAX_IMAGES_BYTES);
    }

    #[test]
    fn admit_all_admits_every_image_or_none() {
        let mut budget = ImageBudget::default();
        let fits = [named("a", 16), named("b", 16)];
        budget.admit_all(&fits).unwrap();
        assert_eq!(budget.sizes.len(), 2);
        let too_big = [named("c", 16), named("d", MAX_IMAGES_BYTES)];
        assert!(budget.admit_all(&too_big).is_err());
        assert_eq!(
            budget.sizes.len(),
            2,
            "nothing of a refused list is counted"
        );
        assert_eq!(budget.total, 2 * (16));
    }
}
