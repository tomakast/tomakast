//! Conversions between the WIT bindings and the core types. Written as exhaustive matches and
//! struct literals, so a WIT change that the core does not follow fails to compile.
//!
//! What comes from a plugin is converted structurally only: the caps and validity rules are
//! [`crate::caps`]'s job, and `plugin-file` icons stay relative until the plugin resolves them
//! against its folder.
//!
//! The widget records convert both ways (a frame comes from the plugin, a context goes to it,
//! and the tests round-trip both). The names that differ on purpose: core `WidgetAnswer.widget`
//! is `widget-answer.id`, `ButtonNode.action` is `button-node.action-id`, the `node` of
//! `PointerAt`, `PointerEvent` and `KeyEvent` is `node-id`, core `Frame` is `widget-frame`, and
//! the struct cases `PathSegment::{QuadTo, CubicTo}` and `WidgetEvent::Action` carry the
//! records `quad-to`, `cubic-to` and `widget-action`.

use std::path::PathBuf;

use tomakast_core::view as core_view;
use tomakast_core::widget as cw;
use tomakast_core::{
    Action, Effect, Icon, Item, ItemId, ItemKind, Modifiers, QueryReading, ReadingKind, Toast,
    ToastKind, ViewId,
};

use crate::bindings::tomakast::plugin::types as wit;

impl From<wit::Action> for Action {
    fn from(a: wit::Action) -> Action {
        Action {
            id: a.id,
            title: a.title,
            shortcut: a.shortcut,
        }
    }
}

impl From<Action> for wit::Action {
    fn from(a: Action) -> wit::Action {
        wit::Action {
            id: a.id,
            title: a.title,
            shortcut: a.shortcut,
        }
    }
}

fn actions(list: Vec<wit::Action>) -> Vec<Action> {
    list.into_iter().map(Action::from).collect()
}

impl From<wit::Icon> for Icon {
    fn from(icon: wit::Icon) -> Icon {
        match icon {
            wit::Icon::Named(name) => Icon::Named(name),
            // Relative here; resolved against the plugin folder after the caps check.
            wit::Icon::PluginFile(path) => Icon::PluginFile(PathBuf::from(path)),
            wit::Icon::Emoji(s) => Icon::Emoji(s),
            wit::Icon::Color(s) => Icon::Color(s),
        }
    }
}

/// `PluginFile` paths must already be relative to the plugin folder (see `WasmPlugin`). A
/// plugin's own items never carry a host `File` icon; one becomes no name at all.
impl From<Icon> for wit::Icon {
    fn from(icon: Icon) -> wit::Icon {
        match icon {
            Icon::Named(name) => wit::Icon::Named(name),
            Icon::PluginFile(path) => wit::Icon::PluginFile(path.to_string_lossy().into_owned()),
            Icon::File(_) => wit::Icon::Named(String::new()),
            Icon::Emoji(s) => wit::Icon::Emoji(s),
            Icon::Color(s) => wit::Icon::Color(s),
        }
    }
}

/// An item id without a `/` belongs to no provider: such an item is dropped like a foreign one.
#[derive(Debug)]
pub struct NoProvider(pub String);

/// The kind is always `Command` and the source unset: the plugin sets the source.
impl TryFrom<wit::Item> for Item {
    type Error = NoProvider;

    fn try_from(item: wit::Item) -> Result<Item, NoProvider> {
        let Some(id) = ItemId::parse(&item.id) else {
            return Err(NoProvider(item.id));
        };
        Ok(Item {
            id,
            title: item.title,
            subtitle: item.subtitle,
            keywords: item.keywords,
            icon: item.icon.map(Icon::from),
            kind: ItemKind::Command,
            actions: actions(item.actions),
            source: None,
        })
    }
}

/// The whole item, for `run`; kind and source stay on the host side.
impl From<Item> for wit::Item {
    fn from(item: Item) -> wit::Item {
        let Item {
            id,
            title,
            subtitle,
            keywords,
            icon,
            kind: _,
            actions,
            source: _,
        } = item;
        wit::Item {
            id: id.as_str().to_owned(),
            title,
            subtitle,
            keywords,
            icon: icon.map(wit::Icon::from),
            actions: actions.into_iter().map(wit::Action::from).collect(),
        }
    }
}

/// The items whose id has a provider prefix, and how many had none (dropped like foreign
/// items; the caller logs them with the plugin's id). Other foreign ids are dropped by
/// [`crate::caps`].
pub(crate) fn items(list: Vec<wit::Item>) -> (Vec<Item>, usize) {
    let before = list.len();
    let items: Vec<Item> = list
        .into_iter()
        .filter_map(|item| Item::try_from(item).ok())
        .collect();
    let dropped = before - items.len();
    (items, dropped)
}

impl From<wit::SearchMode> for core_view::SearchMode {
    fn from(mode: wit::SearchMode) -> core_view::SearchMode {
        match mode {
            wit::SearchMode::CoreFilter => core_view::SearchMode::CoreFilter,
            wit::SearchMode::Module => core_view::SearchMode::Module,
        }
    }
}

impl From<wit::Accessory> for core_view::Accessory {
    fn from(a: wit::Accessory) -> core_view::Accessory {
        match a {
            wit::Accessory::Text(s) => core_view::Accessory::Text(s),
            wit::Accessory::Tag(s) => core_view::Accessory::Tag(s),
        }
    }
}

impl From<wit::ListRow> for core_view::ListRow {
    fn from(row: wit::ListRow) -> core_view::ListRow {
        core_view::ListRow {
            id: row.id,
            section: row.section,
            title: row.title,
            subtitle: row.subtitle,
            icon: row.icon.map(Icon::from),
            accessories: row.accessories.into_iter().map(Into::into).collect(),
            detail_markdown: row.detail_markdown,
            actions: actions(row.actions),
        }
    }
}

impl From<wit::GridTile> for core_view::GridTile {
    fn from(tile: wit::GridTile) -> core_view::GridTile {
        core_view::GridTile {
            id: tile.id,
            image: tile.image.map(Icon::from),
            caption: tile.caption,
            actions: actions(tile.actions),
        }
    }
}

impl From<wit::FieldKind> for core_view::FieldKind {
    fn from(kind: wit::FieldKind) -> core_view::FieldKind {
        match kind {
            wit::FieldKind::Text(f) => core_view::FieldKind::Text {
                value: f.value,
                placeholder: f.placeholder,
            },
            wit::FieldKind::Multiline(value) => core_view::FieldKind::Multiline { value },
            wit::FieldKind::Checkbox(value) => core_view::FieldKind::Checkbox { value },
            wit::FieldKind::Dropdown(f) => core_view::FieldKind::Dropdown {
                options: f.options.into_iter().map(|o| (o.value, o.label)).collect(),
                value: f.value,
            },
            wit::FieldKind::Date(value) => core_view::FieldKind::Date { value },
        }
    }
}

impl From<wit::FormField> for core_view::FormField {
    fn from(field: wit::FormField) -> core_view::FormField {
        core_view::FormField {
            id: field.id,
            label: field.label,
            kind: field.kind.into(),
            error: field.error,
        }
    }
}

impl From<wit::LineAnnotation> for core_view::LineAnnotation {
    fn from(a: wit::LineAnnotation) -> core_view::LineAnnotation {
        core_view::LineAnnotation {
            line: a.line,
            text: a.text,
            is_error: a.is_error,
        }
    }
}

impl From<wit::View> for core_view::View {
    fn from(view: wit::View) -> core_view::View {
        match view {
            wit::View::ListView(v) => core_view::View::List(core_view::ListView {
                id: ViewId(v.id),
                title: v.title,
                placeholder: v.placeholder,
                search: v.search.into(),
                query: v.query,
                revision: v.revision,
                rows: v.rows.into_iter().map(Into::into).collect(),
                show_detail: v.show_detail,
            }),
            wit::View::GridView(v) => core_view::View::Grid(core_view::GridView {
                id: ViewId(v.id),
                title: v.title,
                columns: v.columns,
                tiles: v.tiles.into_iter().map(Into::into).collect(),
            }),
            wit::View::DetailView(v) => core_view::View::Detail(core_view::DetailView {
                id: ViewId(v.id),
                title: v.title,
                markdown: v.markdown,
                metadata: v
                    .metadata
                    .into_iter()
                    .map(|kv| (kv.key, kv.value))
                    .collect(),
                actions: actions(v.actions),
            }),
            wit::View::FormView(v) => core_view::View::Form(core_view::FormView {
                id: ViewId(v.id),
                title: v.title,
                fields: v.fields.into_iter().map(Into::into).collect(),
                submit_title: v.submit_title,
            }),
            wit::View::ScratchpadView(v) => {
                core_view::View::Scratchpad(core_view::ScratchpadView {
                    id: ViewId(v.id),
                    title: v.title,
                    text: v.text,
                    revision: v.revision,
                    annotations: v.annotations.into_iter().map(Into::into).collect(),
                    actions: actions(v.actions),
                })
            }
        }
    }
}

impl From<wit::ToastKind> for ToastKind {
    fn from(kind: wit::ToastKind) -> ToastKind {
        match kind {
            wit::ToastKind::Success => ToastKind::Success,
            wit::ToastKind::Error => ToastKind::Error,
            wit::ToastKind::Progress => ToastKind::Progress,
        }
    }
}

impl From<wit::Effect> for Effect {
    fn from(effect: wit::Effect) -> Effect {
        match effect {
            wit::Effect::PushView(v) => Effect::PushView(v.into()),
            wit::Effect::UpdateView(v) => Effect::UpdateView(v.into()),
            wit::Effect::PopView => Effect::PopView,
            wit::Effect::CloseWindow => Effect::CloseWindow,
            wit::Effect::Toast(t) => Effect::Toast(Toast {
                kind: t.kind.into(),
                title: t.title,
                message: t.message,
            }),
            wit::Effect::CopyText(s) => Effect::CopyText(s),
            // `WasmPlugin::accept` counts and logs the dropped ones before converting.
            wit::Effect::ReplaceItems(list) => Effect::ReplaceItems(items(list).0),
            wit::Effect::ShowWidgets(ids) => Effect::ShowWidgets(ids),
            wit::Effect::RedrawWidget(id) => Effect::RedrawWidget(id),
            wit::Effect::SetWidgetImage(image) => Effect::SetWidgetImage(image.into()),
        }
    }
}

/// One field of a converted record: `copy` (the same type), `into`, `opt` (an option of a
/// converted type) or `vec` (a list of one).
macro_rules! field {
    (copy, $v:expr) => {
        $v
    };
    (into, $v:expr) => {
        $v.into()
    };
    (opt, $v:expr) => {
        $v.map(Into::into)
    };
    (vec, $v:expr) => {
        $v.into_iter().map(Into::into).collect()
    };
}

/// `From` both ways between `wit::$wit` and `cw::$core`, whose fields have the same names.
/// Both sides are destructured and built in full, so a field added on either side fails to
/// compile.
macro_rules! record {
    ($wit:ident <=> $core:ident { $($f:ident: $mode:ident),* $(,)? }) => {
        impl From<wit::$wit> for cw::$core {
            fn from(v: wit::$wit) -> cw::$core {
                let wit::$wit { $($f),* } = v;
                cw::$core { $($f: field!($mode, $f)),* }
            }
        }

        impl From<cw::$core> for wit::$wit {
            fn from(v: cw::$core) -> wit::$wit {
                let cw::$core { $($f),* } = v;
                wit::$wit { $($f: field!($mode, $f)),* }
            }
        }
    };
}

/// `From` both ways between the enum `wit::$wit` and the fieldless enum `cw::$core`, with
/// the same case names.
macro_rules! cases {
    ($wit:ident <=> $core:ident { $($case:ident),* $(,)? }) => {
        impl From<wit::$wit> for cw::$core {
            fn from(v: wit::$wit) -> cw::$core {
                match v {
                    $(wit::$wit::$case => cw::$core::$case),*
                }
            }
        }

        impl From<cw::$core> for wit::$wit {
            fn from(v: cw::$core) -> wit::$wit {
                match v {
                    $(cw::$core::$case => wit::$wit::$case),*
                }
            }
        }
    };
}

cases!(ColorToken <=> ColorToken { Text, Muted, Panel, Border, Selected, Error, Success });
cases!(Align <=> Align { Start, Center, End, Stretch });
cases!(Justify <=> Justify { Start, Center, End, SpaceBetween });
cases!(TextStyle <=> TextStyle { Title, Value, Body, Caption });
cases!(TextAlign <=> TextAlign { Start, Center, End });
cases!(ImageFit <=> ImageFit { Contain, Cover, Fill });
cases!(ImageFormat <=> ImageFormat { Png, Jpeg, Svg });
cases!(PointerKind <=> PointerKind { Down, Up, Move, Enter, Leave });

record!(ThemeColors <=> ThemeColors {
    text: copy,
    muted: copy,
    panel: copy,
    border: copy,
    selected: copy,
    error: copy,
    success: copy,
});
record!(TextSizes <=> TextSizes { title: copy, value: copy, body: copy, caption: copy });
record!(WidgetTheme <=> WidgetTheme {
    dark: copy,
    colors: into,
    ui_font: copy,
    mono_font: copy,
    sizes: into,
});
record!(Point <=> Point { x: copy, y: copy });
record!(WidgetContext <=> WidgetContext {
    width: copy,
    height: copy,
    scale: copy,
    theme: into,
    millis_shown: copy,
    focused: copy,
    pointer: opt,
    query: copy,
    payload: copy,
});
record!(Edges <=> Edges { top: copy, right: copy, bottom: copy, left: copy });
record!(Layout <=> Layout { padding: into, gap: copy, align: into, justify: into });
record!(TextNode <=> TextNode {
    text: copy,
    style: into,
    color: into,
    align: into,
    max_lines: copy,
    mono: copy,
});
record!(IconNode <=> IconNode { icon: into, size: copy });
record!(ImageNode <=> ImageNode { image: copy, fit: into, radius: copy });
record!(CanvasNode <=> CanvasNode { focusable: copy, commands: vec });
record!(Node <=> Node {
    id: copy,
    parent: copy,
    grow: copy,
    width: copy,
    height: copy,
    kind: into,
});
record!(ColorStop <=> ColorStop { color: into, at: copy });
record!(LinearGradient <=> LinearGradient { angle: copy, start: into, end: into });
record!(ShapeStyle <=> ShapeStyle { paint: into, stroke: copy, opacity: copy });
record!(RectCmd <=> RectCmd {
    x: copy,
    y: copy,
    width: copy,
    height: copy,
    radius: copy,
    style: into,
});
record!(EllipseCmd <=> EllipseCmd { cx: copy, cy: copy, rx: copy, ry: copy, style: into });
record!(PathCmd <=> PathCmd { segments: vec, style: into });
record!(TextCmd <=> TextCmd {
    x: copy,
    y: copy,
    text: copy,
    size: copy,
    color: into,
    font: copy,
    mono: copy,
    opacity: copy,
});
record!(ImageCmd <=> ImageCmd {
    image: copy,
    x: copy,
    y: copy,
    width: copy,
    height: copy,
    opacity: copy,
});
record!(ClipRect <=> ClipRect { x: copy, y: copy, width: copy, height: copy });
record!(Transform <=> Transform { translate: into, scale: into, rotate: copy });
record!(WidgetFrame <=> Frame { nodes: vec, animate: copy, actions: vec, images: vec });

impl From<wit::WidgetAnswer> for cw::WidgetAnswer {
    fn from(a: wit::WidgetAnswer) -> cw::WidgetAnswer {
        let wit::WidgetAnswer {
            id,
            payload,
            actions,
        } = a;
        cw::WidgetAnswer {
            widget: id,
            payload,
            actions: self::actions(actions),
        }
    }
}

impl From<cw::WidgetAnswer> for wit::WidgetAnswer {
    fn from(a: cw::WidgetAnswer) -> wit::WidgetAnswer {
        let cw::WidgetAnswer {
            widget,
            payload,
            actions,
        } = a;
        wit::WidgetAnswer {
            id: widget,
            payload,
            actions: actions.into_iter().map(Into::into).collect(),
        }
    }
}

/// Items without a provider prefix are dropped silently, as in `replace-items`. A plugin's
/// real search answers do not pass here: `WasmPlugin::search` converts `answer.items` itself,
/// counting and logging the dropped ones, and checks the caps.
impl From<wit::SearchAnswer> for cw::SearchAnswer {
    fn from(a: wit::SearchAnswer) -> cw::SearchAnswer {
        let wit::SearchAnswer { items, widgets } = a;
        cw::SearchAnswer {
            items: self::items(items).0,
            widgets: widgets.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<cw::SearchAnswer> for wit::SearchAnswer {
    fn from(a: cw::SearchAnswer) -> wit::SearchAnswer {
        let cw::SearchAnswer { items, widgets } = a;
        wit::SearchAnswer {
            items: items.into_iter().map(Into::into).collect(),
            widgets: widgets.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<wit::Color> for cw::Color {
    fn from(c: wit::Color) -> cw::Color {
        match c {
            wit::Color::Token(t) => cw::Color::Token(t.into()),
            wit::Color::Rgba(rgba) => cw::Color::Rgba(rgba),
        }
    }
}

impl From<cw::Color> for wit::Color {
    fn from(c: cw::Color) -> wit::Color {
        match c {
            cw::Color::Token(t) => wit::Color::Token(t.into()),
            cw::Color::Rgba(rgba) => wit::Color::Rgba(rgba),
        }
    }
}

impl From<wit::PointerAt> for cw::PointerAt {
    fn from(p: wit::PointerAt) -> cw::PointerAt {
        let wit::PointerAt { node_id, at } = p;
        cw::PointerAt {
            node: node_id,
            at: at.into(),
        }
    }
}

impl From<cw::PointerAt> for wit::PointerAt {
    fn from(p: cw::PointerAt) -> wit::PointerAt {
        let cw::PointerAt { node, at } = p;
        wit::PointerAt {
            node_id: node,
            at: at.into(),
        }
    }
}

impl From<wit::ButtonNode> for cw::ButtonNode {
    fn from(b: wit::ButtonNode) -> cw::ButtonNode {
        let wit::ButtonNode {
            label,
            icon,
            action_id,
        } = b;
        cw::ButtonNode {
            label,
            icon: icon.map(Into::into),
            action: action_id,
        }
    }
}

impl From<cw::ButtonNode> for wit::ButtonNode {
    fn from(b: cw::ButtonNode) -> wit::ButtonNode {
        let cw::ButtonNode {
            label,
            icon,
            action,
        } = b;
        wit::ButtonNode {
            label,
            icon: icon.map(Into::into),
            action_id: action,
        }
    }
}

impl From<wit::NodeKind> for cw::NodeKind {
    fn from(kind: wit::NodeKind) -> cw::NodeKind {
        match kind {
            wit::NodeKind::Row(l) => cw::NodeKind::Row(l.into()),
            wit::NodeKind::Column(l) => cw::NodeKind::Column(l.into()),
            wit::NodeKind::Overlay(l) => cw::NodeKind::Overlay(l.into()),
            wit::NodeKind::Text(t) => cw::NodeKind::Text(t.into()),
            wit::NodeKind::Icon(i) => cw::NodeKind::Icon(i.into()),
            wit::NodeKind::Image(i) => cw::NodeKind::Image(i.into()),
            wit::NodeKind::Progress(p) => cw::NodeKind::Progress(p),
            wit::NodeKind::Spacer => cw::NodeKind::Spacer,
            wit::NodeKind::Button(b) => cw::NodeKind::Button(b.into()),
            wit::NodeKind::Canvas(c) => cw::NodeKind::Canvas(c.into()),
        }
    }
}

impl From<cw::NodeKind> for wit::NodeKind {
    fn from(kind: cw::NodeKind) -> wit::NodeKind {
        match kind {
            cw::NodeKind::Row(l) => wit::NodeKind::Row(l.into()),
            cw::NodeKind::Column(l) => wit::NodeKind::Column(l.into()),
            cw::NodeKind::Overlay(l) => wit::NodeKind::Overlay(l.into()),
            cw::NodeKind::Text(t) => wit::NodeKind::Text(t.into()),
            cw::NodeKind::Icon(i) => wit::NodeKind::Icon(i.into()),
            cw::NodeKind::Image(i) => wit::NodeKind::Image(i.into()),
            cw::NodeKind::Progress(p) => wit::NodeKind::Progress(p),
            cw::NodeKind::Spacer => wit::NodeKind::Spacer,
            cw::NodeKind::Button(b) => wit::NodeKind::Button(b.into()),
            cw::NodeKind::Canvas(c) => wit::NodeKind::Canvas(c.into()),
        }
    }
}

impl From<wit::Paint> for cw::Paint {
    fn from(p: wit::Paint) -> cw::Paint {
        match p {
            wit::Paint::Solid(c) => cw::Paint::Solid(c.into()),
            wit::Paint::Linear(g) => cw::Paint::Linear(g.into()),
        }
    }
}

impl From<cw::Paint> for wit::Paint {
    fn from(p: cw::Paint) -> wit::Paint {
        match p {
            cw::Paint::Solid(c) => wit::Paint::Solid(c.into()),
            cw::Paint::Linear(g) => wit::Paint::Linear(g.into()),
        }
    }
}

impl From<wit::PathSegment> for cw::PathSegment {
    fn from(segment: wit::PathSegment) -> cw::PathSegment {
        match segment {
            wit::PathSegment::MoveTo(p) => cw::PathSegment::MoveTo(p.into()),
            wit::PathSegment::LineTo(p) => cw::PathSegment::LineTo(p.into()),
            wit::PathSegment::QuadTo(wit::QuadTo { ctrl, to }) => cw::PathSegment::QuadTo {
                ctrl: ctrl.into(),
                to: to.into(),
            },
            wit::PathSegment::CubicTo(wit::CubicTo { ctrl_a, ctrl_b, to }) => {
                cw::PathSegment::CubicTo {
                    ctrl_a: ctrl_a.into(),
                    ctrl_b: ctrl_b.into(),
                    to: to.into(),
                }
            }
            wit::PathSegment::Close => cw::PathSegment::Close,
        }
    }
}

impl From<cw::PathSegment> for wit::PathSegment {
    fn from(segment: cw::PathSegment) -> wit::PathSegment {
        match segment {
            cw::PathSegment::MoveTo(p) => wit::PathSegment::MoveTo(p.into()),
            cw::PathSegment::LineTo(p) => wit::PathSegment::LineTo(p.into()),
            cw::PathSegment::QuadTo { ctrl, to } => wit::PathSegment::QuadTo(wit::QuadTo {
                ctrl: ctrl.into(),
                to: to.into(),
            }),
            cw::PathSegment::CubicTo { ctrl_a, ctrl_b, to } => {
                wit::PathSegment::CubicTo(wit::CubicTo {
                    ctrl_a: ctrl_a.into(),
                    ctrl_b: ctrl_b.into(),
                    to: to.into(),
                })
            }
            cw::PathSegment::Close => wit::PathSegment::Close,
        }
    }
}

impl From<wit::Draw> for cw::Draw {
    fn from(draw: wit::Draw) -> cw::Draw {
        match draw {
            wit::Draw::Rect(c) => cw::Draw::Rect(c.into()),
            wit::Draw::Ellipse(c) => cw::Draw::Ellipse(c.into()),
            wit::Draw::Path(c) => cw::Draw::Path(c.into()),
            wit::Draw::Text(c) => cw::Draw::Text(c.into()),
            wit::Draw::Image(c) => cw::Draw::Image(c.into()),
            wit::Draw::Svg(c) => cw::Draw::Svg(c.into()),
            wit::Draw::PushClip(c) => cw::Draw::PushClip(c.into()),
            wit::Draw::PopClip => cw::Draw::PopClip,
            wit::Draw::PushTransform(t) => cw::Draw::PushTransform(t.into()),
            wit::Draw::PopTransform => cw::Draw::PopTransform,
        }
    }
}

impl From<cw::Draw> for wit::Draw {
    fn from(draw: cw::Draw) -> wit::Draw {
        match draw {
            cw::Draw::Rect(c) => wit::Draw::Rect(c.into()),
            cw::Draw::Ellipse(c) => wit::Draw::Ellipse(c.into()),
            cw::Draw::Path(c) => wit::Draw::Path(c.into()),
            cw::Draw::Text(c) => wit::Draw::Text(c.into()),
            cw::Draw::Image(c) => wit::Draw::Image(c.into()),
            cw::Draw::Svg(c) => wit::Draw::Svg(c.into()),
            cw::Draw::PushClip(c) => wit::Draw::PushClip(c.into()),
            cw::Draw::PopClip => wit::Draw::PopClip,
            cw::Draw::PushTransform(t) => wit::Draw::PushTransform(t.into()),
            cw::Draw::PopTransform => wit::Draw::PopTransform,
        }
    }
}

impl From<wit::WidgetImage> for cw::WidgetImage {
    fn from(image: wit::WidgetImage) -> cw::WidgetImage {
        let wit::WidgetImage { id, format, bytes } = image;
        cw::WidgetImage {
            id,
            format: format.into(),
            bytes: bytes.into(),
        }
    }
}

impl From<cw::WidgetImage> for wit::WidgetImage {
    fn from(image: cw::WidgetImage) -> wit::WidgetImage {
        let cw::WidgetImage { id, format, bytes } = image;
        wit::WidgetImage {
            id,
            format: format.into(),
            bytes: bytes.to_vec(),
        }
    }
}

/// Flags have no exhaustive form: each WIT flag is named here and in the other direction.
impl From<wit::Modifiers> for Modifiers {
    fn from(m: wit::Modifiers) -> Modifiers {
        let mut out = Modifiers::empty();
        out.set(Modifiers::CTRL, m.contains(wit::Modifiers::CTRL));
        out.set(Modifiers::ALT, m.contains(wit::Modifiers::ALT));
        out.set(Modifiers::SHIFT, m.contains(wit::Modifiers::SHIFT));
        out.set(Modifiers::SUPER, m.contains(wit::Modifiers::SUPER));
        out
    }
}

impl From<Modifiers> for wit::Modifiers {
    fn from(m: Modifiers) -> wit::Modifiers {
        let mut out = wit::Modifiers::empty();
        for (core, wit) in [
            (Modifiers::CTRL, wit::Modifiers::CTRL),
            (Modifiers::ALT, wit::Modifiers::ALT),
            (Modifiers::SHIFT, wit::Modifiers::SHIFT),
            (Modifiers::SUPER, wit::Modifiers::SUPER),
        ] {
            if m.contains(core) {
                out |= wit;
            }
        }
        out
    }
}

impl From<wit::WidgetPointer> for cw::PointerEvent {
    fn from(p: wit::WidgetPointer) -> cw::PointerEvent {
        let wit::WidgetPointer { node_id, kind, at } = p;
        cw::PointerEvent {
            node: node_id,
            kind: kind.into(),
            at: at.into(),
        }
    }
}

impl From<cw::PointerEvent> for wit::WidgetPointer {
    fn from(p: cw::PointerEvent) -> wit::WidgetPointer {
        let cw::PointerEvent { node, kind, at } = p;
        wit::WidgetPointer {
            node_id: node,
            kind: kind.into(),
            at: at.into(),
        }
    }
}

impl From<wit::WidgetKey> for cw::KeyEvent {
    fn from(k: wit::WidgetKey) -> cw::KeyEvent {
        let wit::WidgetKey {
            node_id,
            key,
            modifiers,
        } = k;
        cw::KeyEvent {
            node: node_id,
            key,
            modifiers: modifiers.into(),
        }
    }
}

impl From<cw::KeyEvent> for wit::WidgetKey {
    fn from(k: cw::KeyEvent) -> wit::WidgetKey {
        let cw::KeyEvent {
            node,
            key,
            modifiers,
        } = k;
        wit::WidgetKey {
            node_id: node,
            key,
            modifiers: modifiers.into(),
        }
    }
}

impl From<wit::WidgetEvent> for cw::WidgetEvent {
    fn from(event: wit::WidgetEvent) -> cw::WidgetEvent {
        match event {
            wit::WidgetEvent::Shown => cw::WidgetEvent::Shown,
            wit::WidgetEvent::Hidden => cw::WidgetEvent::Hidden,
            wit::WidgetEvent::Action(wit::WidgetAction { node_id, action_id }) => {
                cw::WidgetEvent::Action {
                    node: node_id,
                    action: action_id,
                }
            }
            wit::WidgetEvent::Pointer(p) => cw::WidgetEvent::Pointer(p.into()),
            wit::WidgetEvent::Key(k) => cw::WidgetEvent::Key(k.into()),
        }
    }
}

impl From<cw::WidgetEvent> for wit::WidgetEvent {
    fn from(event: cw::WidgetEvent) -> wit::WidgetEvent {
        match event {
            cw::WidgetEvent::Shown => wit::WidgetEvent::Shown,
            cw::WidgetEvent::Hidden => wit::WidgetEvent::Hidden,
            cw::WidgetEvent::Action { node, action } => {
                wit::WidgetEvent::Action(wit::WidgetAction {
                    node_id: node,
                    action_id: action,
                })
            }
            cw::WidgetEvent::Pointer(p) => wit::WidgetEvent::Pointer(p.into()),
            cw::WidgetEvent::Key(k) => wit::WidgetEvent::Key(k.into()),
        }
    }
}

impl From<core_view::FieldValue> for wit::FieldValue {
    fn from(value: core_view::FieldValue) -> wit::FieldValue {
        match value {
            core_view::FieldValue::Text(s) => wit::FieldValue::Text(s),
            core_view::FieldValue::Bool(b) => wit::FieldValue::Boolean(b),
            core_view::FieldValue::Date(d) => wit::FieldValue::Date(d),
        }
    }
}

impl From<core_view::ViewEvent> for wit::ViewEvent {
    fn from(event: core_view::ViewEvent) -> wit::ViewEvent {
        match event {
            core_view::ViewEvent::QueryChanged {
                revision,
                text,
                readings,
            } => wit::ViewEvent::QueryChanged(wit::QueryChanged {
                revision,
                text,
                readings: query_readings(&readings),
            }),
            core_view::ViewEvent::ActionChosen { row_id, action_id } => {
                wit::ViewEvent::ActionChosen(wit::ActionChosen { row_id, action_id })
            }
            core_view::ViewEvent::FormSubmitted { values } => wit::ViewEvent::FormSubmitted(
                values
                    .into_iter()
                    .map(|(field_id, value)| wit::FormEntry {
                        field_id,
                        value: value.into(),
                    })
                    .collect(),
            ),
            core_view::ViewEvent::TextChanged { revision, text } => {
                wit::ViewEvent::TextChanged(wit::TextChanged { revision, text })
            }
            core_view::ViewEvent::Closed => wit::ViewEvent::Closed,
        }
    }
}

/// The readings of a query, in order. Their script is the core's own and is not sent.
pub(crate) fn query_readings(readings: &[QueryReading]) -> Vec<wit::QueryReading> {
    readings
        .iter()
        .map(|r| wit::QueryReading {
            kind: match r.kind {
                ReadingKind::Layout => wit::QueryReadingKind::Layout,
                ReadingKind::Translit => wit::QueryReadingKind::Translit,
            },
            text: r.text.clone(),
        })
        .collect()
}

/// `event.view` for an event of the plugin's view `view`.
pub(crate) fn view_event(view: &ViewId, event: core_view::ViewEvent) -> wit::Event {
    wit::Event::View(wit::ViewTarget {
        view_id: view.0.clone(),
        event: event.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_roundtrip_keeps_every_wit_field() {
        let item = wit::Item {
            id: "p/x".into(),
            title: "T".into(),
            subtitle: Some("S".into()),
            keywords: vec!["k".into()],
            icon: Some(wit::Icon::PluginFile("icons/a.png".into())),
            actions: vec![wit::Action {
                id: "a".into(),
                title: "A".into(),
                shortcut: Some("ctrl+1".into()),
            }],
        };
        let core = Item::try_from(item.clone()).unwrap();
        assert_eq!(core.kind, ItemKind::Command);
        assert_eq!(core.source, None);
        assert_eq!(core.icon, Some(Icon::PluginFile("icons/a.png".into())));
        let back = wit::Item::from(core);
        assert_eq!(format!("{back:?}"), format!("{item:?}"));
    }

    /// Ruling AQ: a scratchpad's actions reach the core.
    #[test]
    fn scratchpad_keeps_its_actions() {
        let pad = wit::View::ScratchpadView(wit::ScratchpadView {
            id: "pad".into(),
            title: "Pad".into(),
            text: "1+1".into(),
            revision: 2,
            annotations: vec![],
            actions: vec![wit::Action {
                id: "copy".into(),
                title: "Copy result".into(),
                shortcut: None,
            }],
        });
        let core_view::View::Scratchpad(core) = core_view::View::from(pad) else {
            panic!("not a scratchpad");
        };
        assert_eq!(core.actions.len(), 1);
        assert_eq!(core.actions[0].id, "copy");
        assert_eq!(core.actions[0].title, "Copy result");
    }

    /// Readings keep their order and kind; `script` stays on the host side.
    #[test]
    fn query_readings_convert_in_order_without_script() {
        use tomakast_core::Scripts;
        let core = vec![
            QueryReading {
                kind: ReadingKind::Layout,
                text: "firefox".into(),
                script: Scripts::LATIN,
            },
            QueryReading {
                kind: ReadingKind::Translit,
                text: "ashkuashch".into(),
                script: Scripts::LATIN,
            },
        ];
        let converted = query_readings(&core);
        assert_eq!(
            format!("{converted:?}"),
            format!(
                "{:?}",
                [
                    wit::QueryReading {
                        kind: wit::QueryReadingKind::Layout,
                        text: "firefox".into(),
                    },
                    wit::QueryReading {
                        kind: wit::QueryReadingKind::Translit,
                        text: "ashkuashch".into(),
                    },
                ]
            )
        );
        let event = wit::ViewEvent::from(core_view::ViewEvent::QueryChanged {
            revision: 4,
            text: "ашкуащч".into(),
            readings: core,
        });
        let wit::ViewEvent::QueryChanged(changed) = event else {
            panic!("not query-changed");
        };
        assert_eq!((changed.revision, &*changed.text), (4, "ашкуащч"));
        assert_eq!(format!("{:?}", changed.readings), format!("{converted:?}"));
    }

    fn style(paint: cw::Paint) -> cw::ShapeStyle {
        cw::ShapeStyle {
            paint,
            stroke: Some(1.5),
            opacity: 0.5,
        }
    }

    /// A frame with every node kind, draw command, path segment, paint and colour case.
    fn every_kind_frame() -> cw::Frame {
        let p = |x: f32, y: f32| cw::Point { x, y };
        let layout = cw::Layout {
            padding: cw::Edges {
                top: 1.0,
                right: 2.0,
                bottom: 3.0,
                left: 4.0,
            },
            gap: 5.0,
            align: cw::Align::Stretch,
            justify: cw::Justify::SpaceBetween,
        };
        let token = cw::Color::Token(cw::ColorToken::Selected);
        let rgba = cw::Color::Rgba(0x1122_33ff);
        let image = cw::ImageCmd {
            image: "img".into(),
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
            opacity: 0.25,
        };
        let commands = vec![
            cw::Draw::Rect(cw::RectCmd {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
                radius: 5.0,
                style: style(cw::Paint::Solid(token)),
            }),
            cw::Draw::Ellipse(cw::EllipseCmd {
                cx: 1.0,
                cy: 2.0,
                rx: 3.0,
                ry: 4.0,
                style: style(cw::Paint::Linear(cw::LinearGradient {
                    angle: 90.0,
                    start: cw::ColorStop {
                        color: rgba,
                        at: 0.0,
                    },
                    end: cw::ColorStop {
                        color: token,
                        at: 1.0,
                    },
                })),
            }),
            cw::Draw::Path(cw::PathCmd {
                segments: vec![
                    cw::PathSegment::MoveTo(p(0.0, 0.0)),
                    cw::PathSegment::LineTo(p(1.0, 0.0)),
                    cw::PathSegment::QuadTo {
                        ctrl: p(2.0, 1.0),
                        to: p(3.0, 0.0),
                    },
                    cw::PathSegment::CubicTo {
                        ctrl_a: p(4.0, 1.0),
                        ctrl_b: p(5.0, -1.0),
                        to: p(6.0, 0.0),
                    },
                    cw::PathSegment::Close,
                ],
                style: style(cw::Paint::Solid(rgba)),
            }),
            cw::Draw::Text(cw::TextCmd {
                x: 1.0,
                y: 2.0,
                text: "t".into(),
                size: 12.0,
                color: token,
                font: Some("Inter".into()),
                mono: true,
                opacity: 1.0,
            }),
            cw::Draw::Image(image.clone()),
            cw::Draw::Svg(cw::ImageCmd {
                image: "svg".into(),
                ..image
            }),
            cw::Draw::PushClip(cw::ClipRect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            }),
            cw::Draw::PopClip,
            cw::Draw::PushTransform(cw::Transform {
                translate: p(1.0, 2.0),
                scale: p(2.0, 2.0),
                rotate: 45.0,
            }),
            cw::Draw::PopTransform,
        ];
        let kinds = vec![
            cw::NodeKind::Column(layout.clone()),
            cw::NodeKind::Row(layout.clone()),
            cw::NodeKind::Overlay(layout),
            cw::NodeKind::Text(cw::TextNode {
                text: "a\nb".into(),
                style: cw::TextStyle::Value,
                color: rgba,
                align: cw::TextAlign::End,
                max_lines: 2,
                mono: true,
            }),
            cw::NodeKind::Icon(cw::IconNode {
                icon: Icon::Emoji("x".into()),
                size: 16.0,
            }),
            cw::NodeKind::Image(cw::ImageNode {
                image: "img".into(),
                fit: cw::ImageFit::Cover,
                radius: 4.0,
            }),
            cw::NodeKind::Progress(0.5),
            cw::NodeKind::Spacer,
            cw::NodeKind::Button(cw::ButtonNode {
                label: Some("Go".into()),
                icon: Some(Icon::Named("go".into())),
                action: "go".into(),
            }),
            cw::NodeKind::Canvas(cw::CanvasNode {
                focusable: true,
                commands,
            }),
        ];
        cw::Frame {
            nodes: kinds
                .into_iter()
                .enumerate()
                .map(|(i, kind)| cw::Node {
                    id: format!("n{i}"),
                    parent: (i > 0).then_some(0),
                    grow: 1.0,
                    width: Some(10.0),
                    height: None,
                    kind,
                })
                .collect(),
            animate: true,
            actions: vec![Action {
                id: "a".into(),
                title: "A".into(),
                shortcut: Some("ctrl+1".into()),
            }],
            images: vec![cw::WidgetImage {
                id: "img".into(),
                format: cw::ImageFormat::Svg,
                bytes: b"<svg/>".to_vec().into(),
            }],
        }
    }

    #[test]
    fn widget_frame_roundtrips() {
        let frame = every_kind_frame();
        let back = cw::Frame::from(wit::WidgetFrame::from(frame.clone()));
        assert_eq!(back, frame);
    }

    #[test]
    fn widget_context_roundtrips_and_renames() {
        let ctx = cw::WidgetContext {
            width: 355.0,
            height: 120.0,
            scale: 2.0,
            theme: cw::WidgetTheme {
                dark: true,
                colors: cw::ThemeColors {
                    text: 1,
                    muted: 2,
                    panel: 3,
                    border: 4,
                    selected: 5,
                    error: 6,
                    success: 7,
                },
                ui_font: "Inter".into(),
                mono_font: "Mono".into(),
                sizes: cw::TextSizes {
                    title: 1.0,
                    value: 2.0,
                    body: 3.0,
                    caption: 4.0,
                },
            },
            millis_shown: 99,
            focused: Some("b".into()),
            pointer: Some(cw::PointerAt {
                node: Some("c".into()),
                at: cw::Point { x: 1.0, y: 2.0 },
            }),
            query: Some("2+2".into()),
            payload: Some("4".into()),
        };
        let wit_ctx = wit::WidgetContext::from(ctx.clone());
        assert_eq!(
            wit_ctx.pointer.as_ref().unwrap().node_id.as_deref(),
            Some("c")
        );
        assert_eq!(cw::WidgetContext::from(wit_ctx), ctx);
    }

    #[test]
    fn widget_events_roundtrip() {
        let at = cw::Point { x: 3.0, y: 4.0 };
        for event in [
            cw::WidgetEvent::Shown,
            cw::WidgetEvent::Hidden,
            cw::WidgetEvent::Action {
                node: Some("btn".into()),
                action: "inc".into(),
            },
            cw::WidgetEvent::Action {
                node: None,
                action: "copy".into(),
            },
            cw::WidgetEvent::Pointer(cw::PointerEvent {
                node: "c".into(),
                kind: cw::PointerKind::Leave,
                at,
            }),
            cw::WidgetEvent::Key(cw::KeyEvent {
                node: "c".into(),
                key: "a".into(),
                modifiers: Modifiers::CTRL | Modifiers::SUPER,
            }),
            cw::WidgetEvent::Key(cw::KeyEvent {
                node: "c".into(),
                key: "b".into(),
                modifiers: Modifiers::ALT | Modifiers::SHIFT,
            }),
        ] {
            let back = cw::WidgetEvent::from(wit::WidgetEvent::from(event.clone()));
            assert_eq!(back, event);
        }
        let wit::WidgetEvent::Key(key) =
            wit::WidgetEvent::from(cw::WidgetEvent::Key(cw::KeyEvent {
                node: "c".into(),
                key: "a".into(),
                modifiers: Modifiers::SHIFT,
            }))
        else {
            panic!("not a key");
        };
        assert_eq!(key.modifiers, wit::Modifiers::SHIFT);
    }

    #[test]
    fn search_answer_keeps_widgets_and_drops_bare_items() {
        let answer = wit::SearchAnswer {
            items: vec![wit::Item {
                id: "bare".into(),
                title: "T".into(),
                subtitle: None,
                keywords: vec![],
                icon: None,
                actions: vec![],
            }],
            widgets: vec![wit::WidgetAnswer {
                id: "answer".into(),
                payload: "4\n2+2".into(),
                actions: vec![wit::Action {
                    id: "copy".into(),
                    title: "Copy".into(),
                    shortcut: None,
                }],
            }],
        };
        let core = cw::SearchAnswer::from(answer);
        assert!(core.items.is_empty());
        assert_eq!(core.widgets[0].widget, "answer");
        assert_eq!(core.widgets[0].payload, "4\n2+2");
        let back = wit::SearchAnswer::from(core.clone());
        assert_eq!(back.widgets[0].id, "answer");
        assert_eq!(cw::SearchAnswer::from(back), core);
    }

    #[test]
    fn widget_effects_reach_the_core() {
        let image = wit::WidgetImage {
            id: "i".into(),
            format: wit::ImageFormat::Png,
            bytes: vec![1, 2, 3],
        };
        assert_eq!(
            Effect::from(wit::Effect::ShowWidgets(vec!["a".into(), "b".into()])),
            Effect::ShowWidgets(vec!["a".into(), "b".into()])
        );
        assert_eq!(
            Effect::from(wit::Effect::RedrawWidget("a".into())),
            Effect::RedrawWidget("a".into())
        );
        assert_eq!(
            Effect::from(wit::Effect::SetWidgetImage(image)),
            Effect::SetWidgetImage(cw::WidgetImage {
                id: "i".into(),
                format: cw::ImageFormat::Png,
                bytes: vec![1, 2, 3].into(),
            })
        );
    }

    #[test]
    fn item_without_provider_is_refused() {
        let item = wit::Item {
            id: "bare".into(),
            title: "T".into(),
            subtitle: None,
            keywords: vec![],
            icon: None,
            actions: vec![],
        };
        assert!(Item::try_from(item.clone()).is_err());
        assert_eq!(items(vec![item]).1, 1);
    }
}
