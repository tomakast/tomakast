use std::sync::Arc;

use tomakast_core::*;

fn layout() -> Layout {
    Layout {
        padding: Edges {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        },
        gap: 0.0,
        align: Align::Start,
        justify: Justify::Start,
    }
}

fn node(parent: Option<u32>, kind: NodeKind) -> Node {
    Node {
        id: String::new(),
        parent,
        grow: 0.0,
        width: None,
        height: None,
        kind,
    }
}

fn with_id(mut n: Node, id: &str) -> Node {
    n.id = id.to_string();
    n
}

fn frame(nodes: Vec<Node>) -> Frame {
    Frame {
        nodes,
        animate: false,
        actions: vec![],
        images: vec![],
    }
}

fn root() -> Node {
    node(None, NodeKind::Row(layout()))
}

fn text(parent: u32, text: &str) -> Node {
    node(
        Some(parent),
        NodeKind::Text(TextNode {
            text: text.to_string(),
            style: TextStyle::Body,
            color: Color::Token(ColorToken::Text),
            align: TextAlign::Start,
            max_lines: 1,
            mono: false,
        }),
    )
}

fn text_under(parent: u32) -> Node {
    text(parent, "hi")
}

fn canvas_node(focusable: bool, commands: Vec<Draw>) -> Node {
    node(
        Some(0),
        NodeKind::Canvas(CanvasNode {
            focusable,
            commands,
        }),
    )
}

/// A frame whose root holds one canvas with `commands`.
fn canvas(commands: Vec<Draw>) -> Frame {
    frame(vec![root(), canvas_node(false, commands)])
}

fn style() -> ShapeStyle {
    ShapeStyle {
        paint: Paint::Solid(Color::Rgba(0xff00_00ff)),
        stroke: None,
        opacity: 1.0,
    }
}

fn rect() -> RectCmd {
    RectCmd {
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
        radius: 0.0,
        style: style(),
    }
}

fn rect_draw() -> Draw {
    Draw::Rect(rect())
}

fn clip() -> Draw {
    Draw::PushClip(ClipRect {
        x: 0.0,
        y: 0.0,
        width: 5.0,
        height: 5.0,
    })
}

fn transform() -> Draw {
    Draw::PushTransform(Transform {
        translate: Point { x: 0.0, y: 0.0 },
        scale: Point { x: 1.0, y: 1.0 },
        rotate: 0.0,
    })
}

fn path(segments: usize) -> Draw {
    Draw::Path(PathCmd {
        segments: vec![PathSegment::Close; segments],
        style: style(),
    })
}

fn err_of(f: &Frame) -> String {
    check_frame(f).unwrap_err().0
}

#[test]
fn a_frame_is_a_tree_rooted_at_node_zero() {
    assert!(check_frame(&frame(vec![root(), text_under(0)])).is_ok());
    // node 0 has a parent
    assert!(err_of(&frame(vec![text_under(0)])).contains("root"));
    assert!(err_of(&frame(vec![root(), text_under(1)])).contains("parent"));
    // a text is no parent
    assert!(err_of(&frame(vec![root(), text_under(0), text_under(1)])).contains("parent"));
    // a second root
    assert!(err_of(&frame(vec![root(), root()])).contains("root"));
}

#[test]
fn frame_caps_hold_at_the_limit_and_break_past_it() {
    let nodes = |n: usize| {
        let mut v = vec![root()];
        v.extend((1..n).map(|_| text_under(0)));
        v
    };
    assert!(check_frame(&frame(nodes(256))).is_ok());
    assert!(err_of(&frame(nodes(257))).contains("256"));

    // 1 rect + a path of 9 998 segments (itself 1 + 9 998) = 10 000
    assert!(check_frame(&canvas(vec![rect_draw(), path(9_998)])).is_ok());
    // a path of 9 999 segments + 1 rect counts 10 001
    assert!(err_of(&canvas(vec![rect_draw(), path(9_999)])).contains("10000"));
    // commands add up over all canvases
    let two = frame(vec![
        root(),
        canvas_node(false, vec![rect_draw(); 5_000]),
        canvas_node(false, vec![rect_draw(); 5_001]),
    ]);
    assert!(err_of(&two).contains("10000"));
    let two = frame(vec![
        root(),
        canvas_node(false, vec![rect_draw(); 5_000]),
        canvas_node(false, vec![rect_draw(); 5_000]),
    ]);
    assert!(check_frame(&two).is_ok());

    // frame_bytes: one text of exactly the cap minus the two nodes is fine, one more byte is not
    let fits = MAX_FRAME_BYTES - 2 * 16;
    assert!(check_frame(&frame(vec![root(), text(0, &"a".repeat(fits))])).is_ok());
    assert!(err_of(&frame(vec![root(), text(0, &"a".repeat(fits + 1))])).contains("bytes"));
}

#[test]
fn nodes_nest_at_most_32_deep() {
    // The root, then `n` nested columns: the last has `n` ancestors.
    let chain = |n: u32| {
        let mut nodes = vec![node(None, NodeKind::Column(layout()))];
        nodes.extend((1..=n).map(|ix| node(Some(ix - 1), NodeKind::Column(layout()))));
        nodes
    };
    assert_eq!(MAX_NODE_DEPTH, 32);
    assert!(check_frame(&frame(chain(32))).is_ok());
    assert!(err_of(&frame(chain(33))).contains("deeper than 32"));
    // A leaf counts as well.
    let leaf_at = |depth: u32| {
        let mut nodes = chain(depth - 1);
        nodes.push(text(depth - 1, "hi"));
        frame(nodes)
    };
    assert!(check_frame(&leaf_at(32)).is_ok());
    assert!(err_of(&leaf_at(33)).contains("deeper than 32"));
}

#[test]
fn clips_and_transforms_balance_and_nest_at_most_32() {
    assert!(err_of(&canvas(vec![clip()])).contains("unbalanced"));
    assert!(err_of(&canvas(vec![Draw::PopClip])).contains("unbalanced"));
    assert!(err_of(&canvas(vec![Draw::PopTransform])).contains("unbalanced"));
    let crossed = vec![clip(), transform(), Draw::PopClip, Draw::PopTransform];
    assert!(err_of(&canvas(crossed)).contains("unbalanced"));
    let nested = vec![clip(), transform(), Draw::PopTransform, Draw::PopClip];
    assert!(check_frame(&canvas(nested)).is_ok());

    let deep = |n: usize| {
        let mut v = vec![clip(); n];
        v.extend(vec![Draw::PopClip; n]);
        v
    };
    assert!(check_frame(&canvas(deep(32))).is_ok());
    assert!(err_of(&canvas(deep(33))).contains("nesting"));
    // clips and transforms count together
    let mut mixed = vec![clip(); 16];
    mixed.extend(vec![transform(); 17]);
    mixed.extend(vec![Draw::PopTransform; 17]);
    mixed.extend(vec![Draw::PopClip; 16]);
    assert!(err_of(&canvas(mixed)).contains("nesting"));
    // a balanced pair per canvas: one canvas's open clip does not carry into the next
    let split = frame(vec![
        root(),
        canvas_node(false, vec![clip()]),
        canvas_node(false, vec![Draw::PopClip]),
    ]);
    assert!(err_of(&split).contains("unbalanced"));
}

#[test]
fn numbers_are_finite_and_in_range() {
    let bad_rect = |f: fn(&mut RectCmd)| {
        let mut r = rect();
        f(&mut r);
        canvas(vec![Draw::Rect(r)])
    };
    assert!(check_frame(&canvas(vec![rect_draw()])).is_ok());
    assert!(err_of(&bad_rect(|r| r.x = f32::NAN)).contains("finite"));
    assert!(err_of(&bad_rect(|r| r.width = -1.0)).contains("negative"));
    assert!(err_of(&bad_rect(|r| r.style.opacity = 1.5)).contains("opacity"));
    assert!(err_of(&bad_rect(|r| r.style.stroke = Some(-0.5))).contains("stroke"));
    assert!(
        err_of(&bad_rect(|r| {
            r.style.paint = Paint::Linear(LinearGradient {
                angle: 0.0,
                start: ColorStop {
                    color: Color::Rgba(0),
                    at: 0.0,
                },
                end: ColorStop {
                    color: Color::Rgba(0),
                    at: 1.5,
                },
            })
        }))
        .contains("color stop")
    );

    let text_cmd = |size: f32| {
        canvas(vec![Draw::Text(TextCmd {
            x: 0.0,
            y: 0.0,
            text: "t".into(),
            size,
            color: Color::Token(ColorToken::Text),
            font: None,
            mono: false,
            opacity: 1.0,
        })])
    };
    assert!(check_frame(&text_cmd(1.0)).is_ok());
    assert!(check_frame(&text_cmd(200.0)).is_ok());
    assert!(err_of(&text_cmd(0.0)).contains("text size"));
    assert!(err_of(&text_cmd(201.0)).contains("text size"));

    let scaled = |x: f32, y: f32| {
        canvas(vec![
            Draw::PushTransform(Transform {
                translate: Point { x: 0.0, y: 0.0 },
                scale: Point { x, y },
                rotate: 0.0,
            }),
            Draw::PopTransform,
        ])
    };
    assert!(check_frame(&scaled(0.001, 100.0)).is_ok());
    assert!(err_of(&scaled(0.0, 1.0)).contains("scale"));
    assert!(err_of(&scaled(1.0, 101.0)).contains("scale"));

    let progress = |v: f32| frame(vec![root(), node(Some(0), NodeKind::Progress(v))]);
    assert!(check_frame(&progress(1.0)).is_ok());
    assert!(err_of(&progress(f32::INFINITY)).contains("finite"));
    assert!(err_of(&progress(-0.1)).contains("progress"));

    let mut grown = frame(vec![root(), text_under(0)]);
    grown.nodes[1].grow = -1.0;
    assert!(err_of(&grown).contains("grow"));
    let mut wide = frame(vec![root(), text_under(0)]);
    wide.nodes[1].width = Some(f32::NAN);
    assert!(err_of(&wide).contains("width"));
    let mut padded = frame(vec![root()]);
    if let NodeKind::Row(l) = &mut padded.nodes[0].kind {
        l.padding.left = -1.0;
    }
    assert!(err_of(&padded).contains("padding"));
}

#[test]
fn controls_have_unique_non_empty_ids() {
    let button = |id: &str| {
        with_id(
            node(
                Some(0),
                NodeKind::Button(ButtonNode {
                    label: Some("Go".into()),
                    icon: None,
                    action: "go".into(),
                }),
            ),
            id,
        )
    };
    assert!(check_frame(&frame(vec![root(), button("b")])).is_ok());
    assert!(err_of(&frame(vec![root(), button("")])).contains("id"));
    let focusable = |id: &str| with_id(canvas_node(true, vec![]), id);
    assert!(check_frame(&frame(vec![root(), focusable("c")])).is_ok());
    assert!(err_of(&frame(vec![root(), focusable("")])).contains("id"));
    let twice = frame(vec![
        root(),
        with_id(text_under(0), "a"),
        with_id(text_under(0), "a"),
    ]);
    assert!(err_of(&twice).contains("twice"));
    assert!(check_frame(&frame(vec![root(), text_under(0), text_under(0)])).is_ok());
}

#[test]
fn frame_bytes_counts_strings_images_and_items() {
    let mut f = frame(vec![with_id(root(), "r"), text(0, "hello")]);
    f.images.push(WidgetImage {
        id: "i".into(),
        format: ImageFormat::Png,
        bytes: Arc::from(vec![0u8; 100]),
    });
    // 1 + 5 + 1 + 100 + 16 * 2 nodes
    assert_eq!(frame_bytes(&f), 139);
    // a path of 3 segments is 16 for the command and 16 per segment
    let p = canvas(vec![path(3)]);
    assert_eq!(frame_bytes(&p), 2 * 16 + 16 + 3 * 16);
}

#[test]
fn a_list_of_items_is_a_search_answer_without_widgets() {
    let item = Item {
        id: ItemId::new("calc", "1"),
        title: "One".into(),
        subtitle: None,
        keywords: vec![],
        icon: None,
        kind: ItemKind::Command,
        source: None,
        actions: vec![Action {
            id: "open".into(),
            title: "Open".into(),
            shortcut: None,
        }],
    };
    let answer = SearchAnswer::from(vec![item.clone()]);
    assert_eq!(answer.items, vec![item]);
    assert_eq!(answer.widgets, vec![]);
}

#[test]
fn widget_keys_print_and_parse_as_provider_slash_widget() {
    assert_eq!(WidgetKey::new("calc", "answer").to_string(), "calc/answer");
    assert_eq!(
        WidgetKey::parse("calc/answer"),
        Some(WidgetKey::new("calc", "answer"))
    );
    for bad in ["calc", "/answer", "calc/", "a/b/c", ""] {
        assert_eq!(WidgetKey::parse(bad), None, "{bad:?}");
    }
}
