//! The WIT 0.1 draft must parse and mirror the core view model: every record's
//! field names and every variant/enum's case names are checked against a table
//! that follows `view.rs` and `effect.rs`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use wit_parser::{Resolve, TypeDefKind, WorldItem, WorldKey};

/// Expected shape of a WIT type, in declaration order.
enum Shape {
    Record(&'static [&'static str]),
    Variant(&'static [&'static str]),
    Enum(&'static [&'static str]),
}
use Shape::{Enum, Record, Variant};

/// `(wit type, shape)`. Core tuples become records (`key-value`, `dropdown-option`),
/// payload-carrying struct variants get a payload record, and core `Icon::File`
/// maps to `icon.plugin-file`, `Icon::Name` to `icon.named`.
const TYPES: &[(&str, Shape)] = &[
    ("action", Record(&["id", "title"])),
    ("icon", Variant(&["named", "plugin-file"])),
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
    ("form-field", Record(&["id", "label", "kind"])),
    (
        "form-view",
        Record(&["id", "title", "fields", "submit-title"]),
    ),
    ("line-annotation", Record(&["line", "text", "is-error"])),
    (
        "scratchpad-view",
        Record(&["id", "title", "text", "revision", "annotations"]),
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
    ("query-changed", Record(&["revision", "text"])),
    ("action-chosen", Record(&["row-id", "action-id"])),
    ("text-changed", Record(&["revision", "text"])),
    (
        "view-event",
        Variant(&[
            "query-changed",
            "action-chosen",
            "form-submitted",
            "text-changed",
        ]),
    ),
    ("toast-kind", Enum(&["success", "error", "progress"])),
    ("toast", Record(&["kind", "title", "message"])),
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
        ]),
    ),
    (
        "event",
        Variant(&["view", "timer-fired", "http-done", "preferences-changed"]),
    ),
    ("http-done", Record(&["request", "response"])),
];

#[test]
fn wit_parses_and_mirrors_core_views() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugin-api/wit/since_v0.1.0");
    let mut resolve = Resolve::default();
    resolve.push_dir(&dir).expect("plugin.wit parses");

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
        };
        let actual: Vec<&str> = match (&resolve.types[*id].kind, shape) {
            (TypeDefKind::Record(r), Record(_)) => r.fields.iter().map(|f| &*f.name).collect(),
            (TypeDefKind::Variant(v), Variant(_)) => v.cases.iter().map(|c| &*c.name).collect(),
            (TypeDefKind::Enum(e), Enum(_)) => e.cases.iter().map(|c| &*c.name).collect(),
            (other, _) => panic!("`{name}` should be a {kind}, found {other:?}"),
        };
        assert_eq!(actual, expected, "{kind} `{name}` differs from core");
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
        to_set(&["init", "search", "run", "on-event"])
    );
    // `types` is imported implicitly by the world's `use`.
    assert_eq!(
        interface_names(world.imports.iter().collect()),
        to_set(&[
            "types",
            "index",
            "storage",
            "preferences",
            "clipboard",
            "open",
            "http",
            "timers",
            "log",
            "toast"
        ])
    );
}
