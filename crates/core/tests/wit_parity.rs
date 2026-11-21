//! The WIT 0.1 draft must parse and mirror the core view model: every record's
//! field names and every variant/enum/flags' case names are checked against a
//! table that follows `view.rs`, `effect.rs`, `index.rs` and `widget.rs`. The
//! records and variants changed before the freeze, those of the widgets edit, and
//! the world's exports, are also checked down to their field and payload types. The
//! location edit (2026-11-20) adds a plugin-only event case and record, with no core
//! mirror. The Home edit (2026-11-21) replaces the payload of a tile with its parameters,
//! adds the tile sizes and the `home` event, and removes `show-widgets`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use wit_parser::{Function, Resolve, Type, TypeDefKind, WorldItem, WorldKey};

/// Expected shape of a WIT type, in declaration order.
enum Shape {
    Record(&'static [&'static str]),
    Variant(&'static [&'static str]),
    Enum(&'static [&'static str]),
    Flags(&'static [&'static str]),
}
use Shape::{Enum, Flags, Record, Variant};

/// `(wit type, shape)`. Core tuples become records (`key-value`, `dropdown-option`),
/// payload-carrying struct variants get a payload record, and core
/// `Icon::PluginFile` maps to `icon.plugin-file` (core `Icon::File` is for built-in
/// modules only). Core `Item::kind` and `Item::source` are set
/// by the host, so WIT `item` has neither.
///
/// Widgets (`widget.rs`) rename on purpose: core `WidgetAnswer.widget` is
/// `widget-answer.id`, `ButtonNode.action` is `button-node.action-id`,
/// `PointerAt.node` / `PointerEvent.node` / `KeyEvent.node` are `node-id`, core
/// `Frame` is `widget-frame`, `PointerEvent` is `widget-pointer`, `KeyEvent` is
/// `widget-key`, and the struct variants `PathSegment::{QuadTo, CubicTo}` and
/// `WidgetEvent::Action` get the payload records `quad-to`, `cubic-to` and
/// `widget-action`. `widget-target` carries a widget event to `on-event`.
const TYPES: &[(&str, Shape)] = &[
    ("action", Record(&["id", "title", "shortcut"])),
    ("icon", Variant(&["named", "plugin-file", "emoji", "color"])),
    (
        "item",
        Record(&["id", "title", "subtitle", "keywords", "icon", "actions"]),
    ),
    ("key-value", Record(&["key", "value"])),
    ("dropdown-option", Record(&["value", "label"])),
    ("search-mode", Enum(&["core-filter", "module"])),
    ("accessory", Variant(&["text", "tag"])),
    (
        "list-row",
        Record(&[
            "id",
            "section",
            "title",
            "subtitle",
            "icon",
            "accessories",
            "detail-markdown",
            "actions",
        ]),
    ),
    (
        "list-view",
        Record(&[
            "id",
            "title",
            "placeholder",
            "search",
            "query",
            "revision",
            "rows",
            "show-detail",
        ]),
    ),
    ("grid-tile", Record(&["id", "image", "caption", "actions"])),
    ("grid-view", Record(&["id", "title", "columns", "tiles"])),
    (
        "detail-view",
        Record(&["id", "title", "markdown", "metadata", "actions"]),
    ),
    ("text-field", Record(&["value", "placeholder"])),
    ("dropdown-field", Record(&["options", "value"])),
    (
        "field-kind",
        Variant(&["text", "multiline", "checkbox", "dropdown", "date"]),
    ),
    ("form-field", Record(&["id", "label", "kind", "error"])),
    (
        "form-view",
        Record(&["id", "title", "fields", "submit-title"]),
    ),
    ("line-annotation", Record(&["line", "text", "is-error"])),
    (
        "scratchpad-view",
        Record(&["id", "title", "text", "revision", "annotations", "actions"]),
    ),
    (
        "view",
        Variant(&[
            "list-view",
            "grid-view",
            "detail-view",
            "form-view",
            "scratchpad-view",
        ]),
    ),
    ("field-value", Variant(&["text", "boolean", "date"])),
    ("form-entry", Record(&["field-id", "value"])),
    ("query-reading-kind", Enum(&["layout", "translit"])),
    ("query-reading", Record(&["kind", "text"])),
    ("query-changed", Record(&["revision", "text", "readings"])),
    ("action-chosen", Record(&["row-id", "action-id"])),
    ("text-changed", Record(&["revision", "text"])),
    (
        "view-event",
        Variant(&[
            "query-changed",
            "action-chosen",
            "form-submitted",
            "text-changed",
            "closed",
        ]),
    ),
    ("toast-kind", Enum(&["success", "error", "progress"])),
    ("toast", Record(&["kind", "title", "message"])),
    // core Effect::SetRootQuery and Effect::EditHome are internal (built-in modules) and
    // have no WIT case.
    (
        "effect",
        Variant(&[
            "push-view",
            "update-view",
            "pop-view",
            "close-window",
            "toast",
            "copy-text",
            "replace-items",
            "redraw-widget",
            "set-widget-image",
        ]),
    ),
    (
        "event",
        Variant(&[
            "view",
            "timer-fired",
            "http-done",
            "preferences-changed",
            "widget",
            "location",
            "home",
        ]),
    ),
    // The plugin-only answer to `location.request` (2026-11-20); the core has no mirror.
    (
        "coordinates",
        Record(&["latitude", "longitude", "accuracy-km"]),
    ),
    ("view-target", Record(&["view-id", "event"])),
    ("http-done", Record(&["request", "response"])),
    // Widgets.
    ("widget-size", Enum(&["quarter", "single", "double"])),
    // Core `Params` is a sorted list of pairs; WIT has a record per pair.
    ("widget-param", Record(&["key", "value"])),
    (
        "widget-answer",
        Record(&["id", "params", "size", "actions"]),
    ),
    ("search-answer", Record(&["items", "widgets"])),
    (
        "color-token",
        Enum(&[
            "text", "muted", "panel", "border", "selected", "error", "success",
        ]),
    ),
    ("color", Variant(&["token", "rgba"])),
    (
        "theme-colors",
        Record(&[
            "text", "muted", "panel", "border", "selected", "error", "success",
        ]),
    ),
    ("text-sizes", Record(&["title", "value", "body", "caption"])),
    (
        "widget-theme",
        Record(&["dark", "colors", "ui-font", "mono-font", "sizes"]),
    ),
    ("point", Record(&["x", "y"])),
    ("pointer-at", Record(&["node-id", "at"])),
    (
        "widget-context",
        Record(&[
            "width",
            "height",
            "scale",
            "theme",
            "millis-shown",
            "focused",
            "pointer",
            "query",
            "params",
        ]),
    ),
    ("edges", Record(&["top", "right", "bottom", "left"])),
    ("align", Enum(&["start", "center", "end", "stretch"])),
    (
        "justify",
        Enum(&["start", "center", "end", "space-between"]),
    ),
    ("layout", Record(&["padding", "gap", "align", "justify"])),
    ("text-style", Enum(&["title", "value", "body", "caption"])),
    ("text-align", Enum(&["start", "center", "end"])),
    (
        "text-node",
        Record(&["text", "style", "color", "align", "max-lines", "mono"]),
    ),
    ("icon-node", Record(&["icon", "size"])),
    ("image-fit", Enum(&["contain", "cover", "fill"])),
    ("image-node", Record(&["image", "fit", "radius"])),
    ("button-node", Record(&["label", "icon", "action-id"])),
    ("canvas-node", Record(&["focusable", "commands"])),
    (
        "node-kind",
        Variant(&[
            "row", "column", "overlay", "text", "icon", "image", "progress", "spacer", "button",
            "canvas",
        ]),
    ),
    (
        "node",
        Record(&["id", "parent", "grow", "width", "height", "kind"]),
    ),
    ("color-stop", Record(&["color", "at"])),
    ("linear-gradient", Record(&["angle", "start", "end"])),
    ("paint", Variant(&["solid", "linear"])),
    ("shape-style", Record(&["paint", "stroke", "opacity"])),
    (
        "rect-cmd",
        Record(&["x", "y", "width", "height", "radius", "style"]),
    ),
    ("ellipse-cmd", Record(&["cx", "cy", "rx", "ry", "style"])),
    ("quad-to", Record(&["ctrl", "to"])),
    ("cubic-to", Record(&["ctrl-a", "ctrl-b", "to"])),
    (
        "path-segment",
        Variant(&["move-to", "line-to", "quad-to", "cubic-to", "close"]),
    ),
    ("path-cmd", Record(&["segments", "style"])),
    (
        "text-cmd",
        Record(&["x", "y", "text", "size", "color", "font", "mono", "opacity"]),
    ),
    (
        "image-cmd",
        Record(&["image", "x", "y", "width", "height", "opacity"]),
    ),
    ("clip-rect", Record(&["x", "y", "width", "height"])),
    ("transform", Record(&["translate", "scale", "rotate"])),
    (
        "draw",
        Variant(&[
            "rect",
            "ellipse",
            "path",
            "text",
            "image",
            "svg",
            "push-clip",
            "pop-clip",
            "push-transform",
            "pop-transform",
        ]),
    ),
    ("image-format", Enum(&["png", "jpeg", "svg"])),
    ("widget-image", Record(&["id", "format", "bytes"])),
    (
        "widget-frame",
        Record(&["nodes", "animate", "actions", "images"]),
    ),
    (
        "pointer-kind",
        Enum(&["down", "up", "move", "enter", "leave"]),
    ),
    ("widget-pointer", Record(&["node-id", "kind", "at"])),
    ("modifiers", Flags(&["ctrl", "alt", "shift", "super"])),
    ("widget-key", Record(&["node-id", "key", "modifiers"])),
    ("widget-action", Record(&["node-id", "action-id"])),
    (
        "widget-event",
        Variant(&["shown", "hidden", "action", "pointer", "key"]),
    ),
    ("widget-target", Record(&["widget-id", "params", "event"])),
    // Home (2026-11-21): core `HomeTile.widget` is `home-tile.widget-id`.
    ("home-tile", Record(&["widget-id", "params"])),
];

/// `(wit type, member, type)`: field types of records and payload types of
/// variant cases (`-` for a case without payload), for every type the
/// pre-freeze edits changed.
const MEMBER_TYPES: &[(&str, &str, &str)] = &[
    ("action", "id", "string"),
    ("action", "title", "string"),
    ("action", "shortcut", "option<string>"),
    ("form-field", "id", "string"),
    ("form-field", "label", "string"),
    ("form-field", "kind", "field-kind"),
    ("form-field", "error", "option<string>"),
    ("icon", "named", "string"),
    ("icon", "plugin-file", "string"),
    ("icon", "emoji", "string"),
    ("icon", "color", "string"),
    ("item", "id", "string"),
    ("item", "title", "string"),
    ("item", "subtitle", "option<string>"),
    ("item", "keywords", "list<string>"),
    ("item", "icon", "option<icon>"),
    ("item", "actions", "list<action>"),
    ("query-reading", "kind", "query-reading-kind"),
    ("query-reading", "text", "string"),
    ("query-changed", "revision", "u64"),
    ("query-changed", "text", "string"),
    ("query-changed", "readings", "list<query-reading>"),
    ("view-event", "query-changed", "query-changed"),
    ("view-event", "action-chosen", "action-chosen"),
    ("view-event", "form-submitted", "list<form-entry>"),
    ("view-event", "text-changed", "text-changed"),
    ("view-event", "closed", "-"),
    ("view-target", "view-id", "string"),
    ("view-target", "event", "view-event"),
    ("event", "view", "view-target"),
    ("event", "timer-fired", "timer-id"),
    ("event", "http-done", "http-done"),
    ("event", "preferences-changed", "-"),
    ("event", "location", "result<coordinates, string>"),
    ("coordinates", "latitude", "f64"),
    ("coordinates", "longitude", "f64"),
    ("coordinates", "accuracy-km", "u32"),
    ("scratchpad-view", "id", "string"),
    ("scratchpad-view", "title", "string"),
    ("scratchpad-view", "text", "string"),
    ("scratchpad-view", "revision", "u64"),
    ("scratchpad-view", "annotations", "list<line-annotation>"),
    ("scratchpad-view", "actions", "list<action>"),
    ("effect", "push-view", "view"),
    ("effect", "update-view", "view"),
    ("effect", "pop-view", "-"),
    ("effect", "close-window", "-"),
    ("effect", "toast", "toast"),
    ("effect", "copy-text", "string"),
    ("effect", "replace-items", "list<item>"),
    ("effect", "redraw-widget", "string"),
    ("effect", "set-widget-image", "widget-image"),
    ("event", "widget", "widget-target"),
    ("event", "home", "list<home-tile>"),
    // Widgets.
    ("widget-param", "key", "string"),
    ("widget-param", "value", "string"),
    ("widget-answer", "id", "string"),
    ("widget-answer", "params", "list<widget-param>"),
    ("widget-answer", "size", "option<widget-size>"),
    ("widget-answer", "actions", "list<action>"),
    ("search-answer", "items", "list<item>"),
    ("search-answer", "widgets", "list<widget-answer>"),
    ("color", "token", "color-token"),
    ("color", "rgba", "u32"),
    ("theme-colors", "text", "u32"),
    ("theme-colors", "muted", "u32"),
    ("theme-colors", "panel", "u32"),
    ("theme-colors", "border", "u32"),
    ("theme-colors", "selected", "u32"),
    ("theme-colors", "error", "u32"),
    ("theme-colors", "success", "u32"),
    ("text-sizes", "title", "f32"),
    ("text-sizes", "value", "f32"),
    ("text-sizes", "body", "f32"),
    ("text-sizes", "caption", "f32"),
    ("widget-theme", "dark", "bool"),
    ("widget-theme", "colors", "theme-colors"),
    ("widget-theme", "ui-font", "string"),
    ("widget-theme", "mono-font", "string"),
    ("widget-theme", "sizes", "text-sizes"),
    ("point", "x", "f32"),
    ("point", "y", "f32"),
    ("pointer-at", "node-id", "option<string>"),
    ("pointer-at", "at", "point"),
    ("widget-context", "width", "f32"),
    ("widget-context", "height", "f32"),
    ("widget-context", "scale", "f32"),
    ("widget-context", "theme", "widget-theme"),
    ("widget-context", "millis-shown", "u64"),
    ("widget-context", "focused", "option<string>"),
    ("widget-context", "pointer", "option<pointer-at>"),
    ("widget-context", "query", "option<string>"),
    ("widget-context", "params", "list<widget-param>"),
    ("edges", "top", "f32"),
    ("edges", "right", "f32"),
    ("edges", "bottom", "f32"),
    ("edges", "left", "f32"),
    ("layout", "padding", "edges"),
    ("layout", "gap", "f32"),
    ("layout", "align", "align"),
    ("layout", "justify", "justify"),
    ("text-node", "text", "string"),
    ("text-node", "style", "text-style"),
    ("text-node", "color", "color"),
    ("text-node", "align", "text-align"),
    ("text-node", "max-lines", "u8"),
    ("text-node", "mono", "bool"),
    ("icon-node", "icon", "icon"),
    ("icon-node", "size", "f32"),
    ("image-node", "image", "string"),
    ("image-node", "fit", "image-fit"),
    ("image-node", "radius", "f32"),
    ("button-node", "label", "option<string>"),
    ("button-node", "icon", "option<icon>"),
    ("button-node", "action-id", "string"),
    ("canvas-node", "focusable", "bool"),
    ("canvas-node", "commands", "list<draw>"),
    ("node-kind", "row", "layout"),
    ("node-kind", "column", "layout"),
    ("node-kind", "overlay", "layout"),
    ("node-kind", "text", "text-node"),
    ("node-kind", "icon", "icon-node"),
    ("node-kind", "image", "image-node"),
    ("node-kind", "progress", "f32"),
    ("node-kind", "spacer", "-"),
    ("node-kind", "button", "button-node"),
    ("node-kind", "canvas", "canvas-node"),
    ("node", "id", "string"),
    ("node", "parent", "option<u32>"),
    ("node", "grow", "f32"),
    ("node", "width", "option<f32>"),
    ("node", "height", "option<f32>"),
    ("node", "kind", "node-kind"),
    ("color-stop", "color", "color"),
    ("color-stop", "at", "f32"),
    ("linear-gradient", "angle", "f32"),
    ("linear-gradient", "start", "color-stop"),
    ("linear-gradient", "end", "color-stop"),
    ("paint", "solid", "color"),
    ("paint", "linear", "linear-gradient"),
    ("shape-style", "paint", "paint"),
    ("shape-style", "stroke", "option<f32>"),
    ("shape-style", "opacity", "f32"),
    ("rect-cmd", "x", "f32"),
    ("rect-cmd", "y", "f32"),
    ("rect-cmd", "width", "f32"),
    ("rect-cmd", "height", "f32"),
    ("rect-cmd", "radius", "f32"),
    ("rect-cmd", "style", "shape-style"),
    ("ellipse-cmd", "cx", "f32"),
    ("ellipse-cmd", "cy", "f32"),
    ("ellipse-cmd", "rx", "f32"),
    ("ellipse-cmd", "ry", "f32"),
    ("ellipse-cmd", "style", "shape-style"),
    ("quad-to", "ctrl", "point"),
    ("quad-to", "to", "point"),
    ("cubic-to", "ctrl-a", "point"),
    ("cubic-to", "ctrl-b", "point"),
    ("cubic-to", "to", "point"),
    ("path-segment", "move-to", "point"),
    ("path-segment", "line-to", "point"),
    ("path-segment", "quad-to", "quad-to"),
    ("path-segment", "cubic-to", "cubic-to"),
    ("path-segment", "close", "-"),
    ("path-cmd", "segments", "list<path-segment>"),
    ("path-cmd", "style", "shape-style"),
    ("text-cmd", "x", "f32"),
    ("text-cmd", "y", "f32"),
    ("text-cmd", "text", "string"),
    ("text-cmd", "size", "f32"),
    ("text-cmd", "color", "color"),
    ("text-cmd", "font", "option<string>"),
    ("text-cmd", "mono", "bool"),
    ("text-cmd", "opacity", "f32"),
    ("image-cmd", "image", "string"),
    ("image-cmd", "x", "f32"),
    ("image-cmd", "y", "f32"),
    ("image-cmd", "width", "f32"),
    ("image-cmd", "height", "f32"),
    ("image-cmd", "opacity", "f32"),
    ("clip-rect", "x", "f32"),
    ("clip-rect", "y", "f32"),
    ("clip-rect", "width", "f32"),
    ("clip-rect", "height", "f32"),
    ("transform", "translate", "point"),
    ("transform", "scale", "point"),
    ("transform", "rotate", "f32"),
    ("draw", "rect", "rect-cmd"),
    ("draw", "ellipse", "ellipse-cmd"),
    ("draw", "path", "path-cmd"),
    ("draw", "text", "text-cmd"),
    ("draw", "image", "image-cmd"),
    ("draw", "svg", "image-cmd"),
    ("draw", "push-clip", "clip-rect"),
    ("draw", "pop-clip", "-"),
    ("draw", "push-transform", "transform"),
    ("draw", "pop-transform", "-"),
    ("widget-image", "id", "string"),
    ("widget-image", "format", "image-format"),
    ("widget-image", "bytes", "list<u8>"),
    ("widget-frame", "nodes", "list<node>"),
    ("widget-frame", "animate", "bool"),
    ("widget-frame", "actions", "list<action>"),
    ("widget-frame", "images", "list<widget-image>"),
    ("widget-pointer", "node-id", "string"),
    ("widget-pointer", "kind", "pointer-kind"),
    ("widget-pointer", "at", "point"),
    ("widget-key", "node-id", "string"),
    ("widget-key", "key", "string"),
    ("widget-key", "modifiers", "modifiers"),
    ("widget-action", "node-id", "option<string>"),
    ("widget-action", "action-id", "string"),
    ("widget-event", "shown", "-"),
    ("widget-event", "hidden", "-"),
    ("widget-event", "action", "widget-action"),
    ("widget-event", "pointer", "widget-pointer"),
    ("widget-event", "key", "widget-key"),
    ("widget-target", "widget-id", "string"),
    ("widget-target", "params", "list<widget-param>"),
    ("widget-target", "event", "widget-event"),
    ("home-tile", "widget-id", "string"),
    ("home-tile", "params", "list<widget-param>"),
];

/// The world's exports, as WIT signatures.
const EXPORTS: &[(&str, &str)] = &[
    ("init", "func() -> result<list<effect>, string>"),
    (
        "search",
        "func(query: string, readings: list<query-reading>) -> result<search-answer, string>",
    ),
    (
        "run",
        "func(item: item, action-id: string) -> result<list<effect>, string>",
    ),
    (
        "on-event",
        "func(event: event) -> result<list<effect>, string>",
    ),
    (
        "render-widget",
        "func(id: string, context: widget-context) -> result<widget-frame, string>",
    ),
];

/// `(interface, function, signature)` for imports whose signatures changed
/// before the freeze.
const IMPORTS: &[(&str, &str, &str)] = &[
    (
        "storage",
        "get",
        "func(key: string) -> result<option<string>, string>",
    ),
    (
        "storage",
        "set",
        "func(key: string, value: string) -> result<_, string>",
    ),
    (
        "storage",
        "remove",
        "func(key: string) -> result<_, string>",
    ),
    ("locale", "local-time-zone", "func() -> string"),
    ("location", "request", "func() -> result<_, string>"),
];

/// Renders `ty` as WIT source: named types by name, anonymous ones by structure.
fn render(resolve: &Resolve, ty: &Type) -> String {
    let id = match ty {
        Type::Bool => return "bool".into(),
        Type::U8 => return "u8".into(),
        Type::U16 => return "u16".into(),
        Type::U32 => return "u32".into(),
        Type::U64 => return "u64".into(),
        Type::F32 => return "f32".into(),
        Type::F64 => return "f64".into(),
        Type::String => return "string".into(),
        Type::Id(id) => *id,
        other => panic!("type {other:?} is not used by the contract"),
    };
    let def = &resolve.types[id];
    if let Some(name) = &def.name {
        return name.clone();
    }
    match &def.kind {
        TypeDefKind::Option(t) => format!("option<{}>", render(resolve, t)),
        TypeDefKind::List(t) => format!("list<{}>", render(resolve, t)),
        TypeDefKind::Result(r) => {
            let side = |t: &Option<Type>| t.as_ref().map_or("_".into(), |t| render(resolve, t));
            format!("result<{}, {}>", side(&r.ok), side(&r.err))
        }
        other => panic!("anonymous type {other:?} is not used by the contract"),
    }
}

fn signature(resolve: &Resolve, f: &Function) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, render(resolve, &p.ty)))
        .collect();
    let result = f
        .result
        .as_ref()
        .map_or(String::new(), |t| format!(" -> {}", render(resolve, t)));
    format!("func({}){result}", params.join(", "))
}

#[test]
fn wit_parses_and_mirrors_core_views() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugin-api/wit/since_v0.1.0");
    let mut resolve = Resolve::default();
    resolve.push_dir(&dir).expect("plugin.wit parses");

    // The package doc comment sits above `package` and so documents the package.
    let (_, package) = resolve.packages.iter().next().expect("one package");
    assert!(
        package
            .docs
            .contents
            .as_deref()
            .is_some_and(|d| d.starts_with("The tomakast plugin contract")),
        "package doc comment: {:?}",
        package.docs.contents
    );

    let (_, interface) = resolve
        .interfaces
        .iter()
        .find(|(_, i)| i.name.as_deref() == Some("types"))
        .expect("interface `types`");

    for (name, shape) in TYPES {
        let id = interface
            .types
            .get(*name)
            .unwrap_or_else(|| panic!("missing type `{name}`"));
        let (kind, expected): (&str, &[&str]) = match shape {
            Record(f) => ("record", f),
            Variant(c) => ("variant", c),
            Enum(c) => ("enum", c),
            Flags(f) => ("flags", f),
        };
        let actual: Vec<&str> = match (&resolve.types[*id].kind, shape) {
            (TypeDefKind::Record(r), Record(_)) => r.fields.iter().map(|f| &*f.name).collect(),
            (TypeDefKind::Variant(v), Variant(_)) => v.cases.iter().map(|c| &*c.name).collect(),
            (TypeDefKind::Enum(e), Enum(_)) => e.cases.iter().map(|c| &*c.name).collect(),
            (TypeDefKind::Flags(f), Flags(_)) => f.flags.iter().map(|f| &*f.name).collect(),
            (other, _) => panic!("`{name}` should be a {kind}, found {other:?}"),
        };
        assert_eq!(actual, expected, "{kind} `{name}` differs from core");
    }

    for (name, member, expected) in MEMBER_TYPES {
        let id = interface.types[*name];
        let actual = match &resolve.types[id].kind {
            TypeDefKind::Record(r) => r
                .fields
                .iter()
                .find(|f| f.name == *member)
                .map(|f| render(&resolve, &f.ty)),
            TypeDefKind::Variant(v) => v
                .cases
                .iter()
                .find(|c| c.name == *member)
                .map(|c| c.ty.as_ref().map_or("-".into(), |t| render(&resolve, t))),
            other => panic!("`{name}` has no members: {other:?}"),
        }
        .unwrap_or_else(|| panic!("`{name}` has no member `{member}`"));
        assert_eq!(&actual, expected, "type of `{name}.{member}`");
    }

    // Every type defined in `types` is covered by the table (or is a plain alias).
    let covered: BTreeSet<&str> = TYPES.iter().map(|(n, _)| *n).collect();
    let uncovered: Vec<&str> = interface
        .types
        .iter()
        .filter(|(n, id)| {
            !covered.contains(n.as_str())
                && !matches!(resolve.types[**id].kind, TypeDefKind::Type(_))
        })
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(
        uncovered,
        ["http-request", "http-response"],
        "types missing from the parity table"
    );

    for (iface, name, expected) in IMPORTS {
        let (_, i) = resolve
            .interfaces
            .iter()
            .find(|(_, i)| i.name.as_deref() == Some(*iface))
            .unwrap_or_else(|| panic!("interface `{iface}`"));
        let f = i
            .functions
            .get(*name)
            .unwrap_or_else(|| panic!("missing `{iface}.{name}`"));
        assert_eq!(
            &signature(&resolve, f),
            expected,
            "signature of `{iface}.{name}`"
        );
    }

    let (_, world) = resolve
        .worlds
        .iter()
        .find(|(_, w)| w.name == "plugin")
        .expect("world `plugin`");
    let interface_names = |items: Vec<(&WorldKey, &WorldItem)>| {
        items
            .into_iter()
            .filter(|(_, v)| !matches!(v, WorldItem::Type { .. }))
            .map(|(k, _)| k)
            .map(|k| match k {
                WorldKey::Name(n) => n.clone(),
                WorldKey::Interface(i) => resolve.interfaces[*i].name.clone().unwrap(),
            })
            .collect::<BTreeSet<String>>()
    };
    let to_set = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<BTreeSet<_>>();
    assert_eq!(
        interface_names(world.exports.iter().collect()),
        to_set(&EXPORTS.iter().map(|(n, _)| *n).collect::<Vec<_>>())
    );
    for (name, expected) in EXPORTS {
        let Some(WorldItem::Function(f)) = world.exports.get(&WorldKey::Name(name.to_string()))
        else {
            panic!("export `{name}` is not a function");
        };
        assert_eq!(&signature(&resolve, f), expected, "signature of `{name}`");
    }
    // `types` is imported implicitly by the world's `use`.
    assert_eq!(
        interface_names(world.imports.iter().collect()),
        to_set(&[
            "types",
            "storage",
            "preferences",
            "open",
            "http",
            "timers",
            "log",
            "locale",
            "location"
        ])
    );
}
