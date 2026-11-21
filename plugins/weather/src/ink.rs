//! The drawing kit of the tiles: their size, faded colours and faint icons. Pure, so it is
//! tested natively.
//!
//! An icon node always takes the theme's text colour, and nodes have no opacity; only canvas
//! commands do. So a faded icon is one of the Lucide SVGs the plugin ships (`icons/`, ISC,
//! see `icons/LICENSE-LUCIDE`), coloured with the theme's text colour, sent in the frame and
//! drawn on a canvas at an opacity. A bright icon stays an icon node.

use tomakast_plugin::{Color, Draw, FrameBuilder, Icon, Node, NodeRef, WidgetContext, WidgetImage};
#[cfg(test)]
use tomakast_plugin::{NodeKind, WidgetFrame};

/// The opacity of a part already over, and of a past hour of a chart.
pub const PAST: f32 = 0.4;
/// The opacity of a muted icon.
pub const MUTED: f32 = 0.6;

/// The shipped icons, by name.
const ICONS: [(&str, &str); 22] = [
    ("sun", include_str!("../icons/sun.svg")),
    ("moon", include_str!("../icons/moon.svg")),
    ("sunrise", include_str!("../icons/sunrise.svg")),
    ("sunset", include_str!("../icons/sunset.svg")),
    ("cloud", include_str!("../icons/cloud.svg")),
    ("cloud-sun", include_str!("../icons/cloud-sun.svg")),
    ("cloud-moon", include_str!("../icons/cloud-moon.svg")),
    ("cloud-fog", include_str!("../icons/cloud-fog.svg")),
    ("cloud-drizzle", include_str!("../icons/cloud-drizzle.svg")),
    ("cloud-rain", include_str!("../icons/cloud-rain.svg")),
    ("cloud-snow", include_str!("../icons/cloud-snow.svg")),
    (
        "cloud-sun-rain",
        include_str!("../icons/cloud-sun-rain.svg"),
    ),
    (
        "cloud-moon-rain",
        include_str!("../icons/cloud-moon-rain.svg"),
    ),
    (
        "cloud-lightning",
        include_str!("../icons/cloud-lightning.svg"),
    ),
    ("cloud-hail", include_str!("../icons/cloud-hail.svg")),
    ("thermometer", include_str!("../icons/thermometer.svg")),
    ("wind", include_str!("../icons/wind.svg")),
    ("droplets", include_str!("../icons/droplets.svg")),
    ("umbrella", include_str!("../icons/umbrella.svg")),
    ("gauge", include_str!("../icons/gauge.svg")),
    ("leaf", include_str!("../icons/leaf.svg")),
    ("flower-2", include_str!("../icons/flower-2.svg")),
];

/// The prefix of a faint icon's image id.
const PREFIX: &str = "lucide-";

/// The tile sizes a widget can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Quarter,
    Single,
    Double,
}

/// The size of a tile, from its width and height: the render context gives no size, and the
/// strip's height is fixed. A quarter is 173.5 x 120, a single 355 x 120, a double about
/// 718 x 120.
pub fn size_of(context: &WidgetContext) -> Size {
    if context.width < 2. * context.height {
        Size::Quarter
    } else if context.width < 4. * context.height {
        Size::Single
    } else {
        Size::Double
    }
}

/// The theme's text colour with its alpha times `alpha`.
pub fn faded(context: &WidgetContext, alpha: f32) -> Color {
    let text = context.theme.colors.text;
    let a = ((text & 0xff) as f32 * alpha.clamp(0., 1.)).round() as u32;
    Color::Rgba(text & !0xff | a)
}

/// The shipped SVG of the icon `name`.
pub fn svg(name: &str) -> Option<&'static str> {
    ICONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, text)| *text)
}

/// The icon `name`, `size` logical pixels, under `parent`, at opacity `alpha`. Bright (or
/// not shipped), it is an icon node; faded, a canvas drawing the shipped SVG in the theme's
/// text colour. The SVG goes with the frame once, as the image `lucide-<name>`. The id does
/// not name the colour: the host keeps every image id a plugin sends for its whole run, so a
/// theme change sends the same id again with new bytes, which replaces it.
pub fn icon(
    frame: FrameBuilder,
    parent: NodeRef,
    name: &str,
    size: f32,
    alpha: f32,
    context: &WidgetContext,
) -> FrameBuilder {
    let shipped = svg(name).filter(|_| alpha < 1.);
    let Some(text) = shipped else {
        return frame
            .child(parent, Node::icon(Icon::Named(name.into()), size))
            .0;
    };
    let rgb = format!("#{:06x}", context.theme.colors.text >> 8);
    let id = format!("{PREFIX}{name}");
    let canvas = Node::canvas(
        "",
        vec![Draw::svg(id.as_str(), 0., 0., size, size).opacity(alpha.max(0.))],
    )
    .size(size, size);
    // The builder shows nothing it holds, so a built copy tells whether the image is in.
    let sent = frame.clone().build().images.iter().any(|i| i.id == id);
    let frame = frame.child(parent, canvas).0;
    if sent {
        frame
    } else {
        frame.image(WidgetImage::svg(id, text.replace("currentColor", &rgb)))
    }
}

/// What `frame` shows, in order: texts, `icon:<name>` for icon nodes, `faint:<name>` for the
/// SVG commands of a canvas, and `chart` for a canvas without SVG, followed by its texts.
#[cfg(test)]
pub fn shown(frame: &WidgetFrame) -> Vec<String> {
    let mut out = Vec::new();
    for node in &frame.nodes {
        match &node.kind {
            NodeKind::Text(t) => out.push(t.text.clone()),
            NodeKind::Icon(i) => {
                if let Icon::Named(name) = &i.icon {
                    out.push(format!("icon:{name}"));
                }
            }
            NodeKind::Canvas(c) => {
                let faint: Vec<String> = c
                    .commands
                    .iter()
                    .filter_map(|draw| match draw {
                        Draw::Svg(image) => {
                            let name = image.image.strip_prefix(PREFIX)?;
                            Some(format!("faint:{name}"))
                        }
                        _ => None,
                    })
                    .collect();
                if faint.is_empty() {
                    out.push("chart".into());
                    out.extend(c.commands.iter().filter_map(|draw| match draw {
                        Draw::Text(t) => Some(t.text.clone()),
                        _ => None,
                    }));
                } else {
                    out.extend(faint);
                }
            }
            _ => {}
        }
    }
    out
}

/// The dark context the tile tests render in: text `0xECECF0FF`, `width` x 120.
#[cfg(test)]
pub fn context(width: f32) -> WidgetContext {
    use tomakast_plugin::{TextSizes, ThemeColors, WidgetTheme};

    WidgetContext {
        width,
        height: 120.,
        scale: 1.,
        theme: WidgetTheme {
            dark: true,
            colors: ThemeColors {
                text: 0xECECF0FF,
                muted: 0,
                panel: 0,
                border: 0,
                selected: 0,
                error: 0,
                success: 0,
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
        query: None,
        params: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use tomakast_plugin::Layout;

    use super::*;

    const NAMES: [&str; 22] = [
        "sun",
        "moon",
        "sunrise",
        "sunset",
        "cloud",
        "cloud-sun",
        "cloud-moon",
        "cloud-fog",
        "cloud-drizzle",
        "cloud-rain",
        "cloud-snow",
        "cloud-sun-rain",
        "cloud-moon-rain",
        "cloud-lightning",
        "cloud-hail",
        "thermometer",
        "wind",
        "droplets",
        "umbrella",
        "gauge",
        "leaf",
        "flower-2",
    ];

    #[test]
    fn sizes_from_the_context() {
        assert_eq!(size_of(&context(173.5)), Size::Quarter);
        assert_eq!(size_of(&context(355.)), Size::Single);
        assert_eq!(size_of(&context(718.)), Size::Double);
    }

    #[test]
    fn faded_keeps_the_colour() {
        // 0xFF x 0.4 = 102 = 0x66.
        assert!(matches!(
            faded(&context(355.), 0.4),
            Color::Rgba(0xECECF066)
        ));
        assert!(matches!(faded(&context(355.), 1.), Color::Rgba(0xECECF0FF)));
    }

    #[test]
    fn every_shipped_icon_parses() {
        for name in NAMES {
            let text = svg(name).unwrap_or_else(|| panic!("{name} is not shipped"));
            assert!(
                text.contains("<svg") && text.contains("currentColor"),
                "{name}"
            );
        }
        assert_eq!(svg("nope"), None);
        // A name with a hyphen reads back whole.
        let ctx = context(355.);
        let frame = WidgetFrame::new(Node::row(Layout::default()));
        let root = frame.root();
        let frame = icon(frame, root, "cloud-sun-rain", 20., MUTED, &ctx).build();
        assert_eq!(shown(&frame), ["faint:cloud-sun-rain"]);
    }

    #[test]
    fn faint_icons_ride_in_the_frame() {
        let ctx = context(355.);
        let frame = WidgetFrame::new(Node::row(Layout::default()));
        let root = frame.root();
        let frame = icon(frame, root, "cloud", 20., PAST, &ctx);
        let frame = icon(frame, root, "cloud", 20., PAST, &ctx);
        let frame = icon(frame, root, "cloud", 20., 1., &ctx).build();

        assert_eq!(frame.images.len(), 1);
        let image = &frame.images[0];
        assert_eq!(image.id, "lucide-cloud");
        let text = String::from_utf8(image.bytes.clone()).unwrap();
        assert!(text.contains(r##"stroke="#ececf0""##), "{text}");
        assert!(!text.contains("currentColor"));

        let canvases: Vec<_> = frame
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Canvas(c) => Some((n, c)),
                _ => None,
            })
            .collect();
        assert_eq!(canvases.len(), 2);
        for (node, canvas) in canvases {
            assert_eq!((node.width, node.height), (Some(20.), Some(20.)));
            let [Draw::Svg(cmd)] = canvas.commands.as_slice() else {
                panic!("{:?}", canvas.commands);
            };
            assert_eq!(cmd.image, "lucide-cloud");
            assert_eq!((cmd.x, cmd.y, cmd.width, cmd.height), (0., 0., 20., 20.));
            assert_eq!(cmd.opacity, 0.4);
        }
        let named = frame
            .nodes
            .iter()
            .filter(|n| matches!(&n.kind, NodeKind::Icon(i) if matches!(&i.icon, Icon::Named(name) if name == "cloud")))
            .count();
        assert_eq!(named, 1);
        assert_eq!(shown(&frame), ["faint:cloud", "faint:cloud", "icon:cloud"]);
    }
}
