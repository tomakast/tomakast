//! `demo` module: one item per view kind, the native freeze-proof for every
//! view before plan 2b-2 binds WASM modules to the same types. The daemon
//! registers it only when `TOMAKAST_DEMO=1`.

use std::time::Duration;

use tomakast_core::{
    Accessory, Action, DetailView, Effect, FieldKind, FieldValue, FormField, FormView, GridTile,
    GridView, Icon, Item, ItemId, ItemKind, LineAnnotation, ListLayout, ListRow, ListView, Module,
    ModuleContext, ScratchpadView, SearchAnswer, SearchMode, Toast, ToastKind, View, ViewEvent,
    ViewId,
};

/// How long "Demo: Slow" takes to answer a query.
const SLOW_DELAY: Duration = Duration::from_millis(300);

const LIST: &str = "list";
const GRID: &str = "grid";
const DETAIL: &str = "detail";
const FORM: &str = "form";
const SCRATCHPAD: &str = "scratchpad";
const SLOW: &str = "slow";
const PANIC: &str = "panic";
const SLOW_RESULT: &str = "slow-result";

/// Titles of the demo list's rows `row-1` … `row-6`.
const ROW_NAMES: [&str; 6] = ["Apple", "Banana", "Cherry", "Carrot", "Leek", "Pea"];
/// The shortcut of the list rows' "Copy Name" action.
const COPY_SHORTCUT: &str = "ctrl+shift+c";

#[derive(Default)]
pub struct DemoModule;

impl DemoModule {
    pub fn new() -> DemoModule {
        DemoModule
    }
}

fn item(local: &str, title: &str, subtitle: &str) -> Item {
    Item {
        id: ItemId::new("demo", local),
        title: title.to_string(),
        subtitle: Some(subtitle.to_string()),
        keywords: vec!["demo".to_string()],
        icon: None,
        kind: ItemKind::Command,
        source: None,
        actions: vec![Action {
            id: "open".to_string(),
            title: "Open".to_string(),
            shortcut: None,
        }],
    }
}

fn items() -> Vec<Item> {
    vec![
        item(LIST, "Demo: List", "Sections and a detail pane"),
        item(
            GRID,
            "Demo: Grid",
            "12 items: theme icons, emoji and colours",
        ),
        item(
            DETAIL,
            "Demo: Detail",
            "Markdown with inert HTML, image and links",
        ),
        item(FORM, "Demo: Form", "Every field kind"),
        item(
            SCRATCHPAD,
            "Demo: Scratchpad",
            "Line lengths; \"err\" lines are errors",
        ),
        item(
            SLOW,
            "Demo: Slow",
            "A dynamic source that answers after 300 ms",
        ),
        item(PANIC, "Demo: Panic", "Panics in run_action"),
    ]
}

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.to_string(),
        title: title.to_string(),
        shortcut: None,
    }
}

fn list_view(id: ViewId) -> View {
    let rows = (1..=6)
        .map(|n| {
            let section = if n <= 3 { "Fruit" } else { "Vegetables" };
            let name = ROW_NAMES[n - 1];
            ListRow {
                id: format!("row-{n}"),
                section: Some(section.to_string()),
                title: name.to_string(),
                subtitle: Some(format!("{section} #{n}")),
                icon: None,
                accessories: vec![
                    Accessory::Text(format!("{n}")),
                    Accessory::Tag(section.to_string()),
                ],
                detail_markdown: Some(format!("# {name}\n\nRow {n} of the **{section}** section.")),
                actions: vec![
                    action("pick", "Pick"),
                    action("again", "Pick again"),
                    Action {
                        shortcut: Some(COPY_SHORTCUT.to_string()),
                        ..action("copy", "Copy Name")
                    },
                ],
            }
        })
        .collect();
    View::List(ListView {
        id,
        title: "Demo: List".to_string(),
        placeholder: "Filter the demo rows".to_string(),
        search: SearchMode::CoreFilter,
        query: String::new(),
        revision: 0,
        rows,
        show_detail: true,
        layout: ListLayout::Rows,
        footer_action: None,
        busy: false,
    })
}

/// Grid tiles: four theme icons, four emoji, four colour swatches.
const EMOJI: [(&str, &str); 4] = [
    ("🍎", "Apple"),
    ("🍌", "Banana"),
    ("🍒", "Cherry"),
    ("🥕", "Carrot"),
];
const COLORS: [&str; 4] = ["#e5484d", "#f5d90a", "#30a46c", "#0090ff"];

fn grid_view(id: ViewId) -> View {
    let tiles = (1..=12)
        .map(|n| {
            let (image, caption) = match n {
                1..=4 => (Icon::Named("image".to_string()), format!("Item {n}")),
                5..=8 => (
                    Icon::Emoji(EMOJI[n - 5].0.to_string()),
                    EMOJI[n - 5].1.to_string(),
                ),
                _ => (
                    Icon::Color(COLORS[n - 9].to_string()),
                    COLORS[n - 9].to_string(),
                ),
            };
            GridTile {
                id: format!("tile-{n}"),
                image: Some(image),
                caption,
                actions: vec![action("pick", "Pick")],
            }
        })
        .collect();
    View::Grid(GridView {
        id,
        title: "Demo: Grid".to_string(),
        columns: 4,
        tiles,
    })
}

const DETAIL_MARKDOWN: &str = "# Inert content\n\n\
Raw HTML must not render: <script>alert(1)</script> <b>not bold</b>\n\n\
<div style=\"color:red\">not a div</div>\n\n\
A remote image must not load: ![remote](https://example.com/tracker.png)\n\n\
An https link opens only when clicked, in the browser: [example](https://example.com)\n\n\
A file link never opens, a click does nothing: [passwd](file:///etc/passwd)\n";

fn detail_view(id: ViewId) -> View {
    View::Detail(DetailView {
        id,
        title: "Demo: Detail".to_string(),
        markdown: DETAIL_MARKDOWN.to_string(),
        metadata: vec![
            ("Kind".to_string(), "Detail".to_string()),
            ("Source".to_string(), "demo module".to_string()),
        ],
        actions: vec![action("close", "Close")],
    })
}

fn field(id: &str, label: &str, kind: FieldKind) -> FormField {
    FormField {
        id: id.to_string(),
        label: label.to_string(),
        kind,
        error: None,
    }
}

fn form_view(id: ViewId) -> View {
    View::Form(FormView {
        id,
        title: "Demo: Form".to_string(),
        fields: vec![
            field(
                "name",
                "Name",
                FieldKind::Text {
                    value: String::new(),
                    placeholder: "Your name".to_string(),
                },
            ),
            FormField {
                error: Some("This handle is taken".to_string()),
                ..field(
                    "handle",
                    "Handle",
                    FieldKind::Text {
                        value: "admin".to_string(),
                        placeholder: "A unique handle".to_string(),
                    },
                )
            },
            field(
                "notes",
                "Notes",
                FieldKind::Multiline {
                    value: String::new(),
                },
            ),
            field(
                "subscribe",
                "Subscribe",
                FieldKind::Checkbox { value: true },
            ),
            field(
                "colour",
                "Colour",
                FieldKind::Dropdown {
                    options: vec![
                        ("red".to_string(), "Red".to_string()),
                        ("green".to_string(), "Green".to_string()),
                        ("blue".to_string(), "Blue".to_string()),
                    ],
                    value: "green".to_string(),
                },
            ),
            field("due", "Due date", FieldKind::Date { value: None }),
        ],
        submit_title: "Submit".to_string(),
    })
}

fn scratchpad_view(id: ViewId) -> View {
    View::Scratchpad(ScratchpadView {
        id,
        title: "Demo: Scratchpad".to_string(),
        text: "hello\nworld\nthis line has an err in it".to_string(),
        revision: 0,
        annotations: annotate("hello\nworld\nthis line has an err in it"),
        actions: pad_actions(),
        output: None,
        toolbar: Vec::new(),
        busy: false,
        placeholder: String::new(),
        detail: None,
    })
}

/// The pad's actions (Ruling AQ): they act on the whole pad, from the action panel.
fn pad_actions() -> Vec<Action> {
    vec![
        Action {
            id: "copy-total".to_string(),
            title: "Copy total length".to_string(),
            shortcut: None,
        },
        Action {
            id: "clear".to_string(),
            title: "Clear".to_string(),
            shortcut: Some("ctrl+shift+l".to_string()),
        },
    ]
}

/// One annotation per non-empty line: its length; a line containing "err" is an error.
fn annotate(text: &str) -> Vec<LineAnnotation> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
        .map(|(i, line)| {
            let is_error = line.contains("err");
            LineAnnotation {
                line: i as u32,
                text: if is_error {
                    format!("error: {} chars", line.chars().count())
                } else {
                    format!("{} chars", line.chars().count())
                },
                is_error,
                swatch: None,
            }
        })
        .collect()
}

fn toast(kind: ToastKind, title: &str, message: Option<String>) -> Effect {
    Effect::Toast(Toast {
        kind,
        title: title.to_string(),
        message,
    })
}

fn describe(value: &FieldValue) -> String {
    match value {
        FieldValue::Text(s) => format!("{s:?}"),
        FieldValue::Bool(b) => b.to_string(),
        FieldValue::Date(Some(d)) => d.clone(),
        FieldValue::Date(None) => "none".to_string(),
    }
}

impl Module for DemoModule {
    fn id(&self) -> &str {
        "demo"
    }

    fn start(&mut self, ctx: ModuleContext) {
        ctx.replace_items(items());
    }

    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
        if action_id != "open" {
            anyhow::bail!("demo: unknown action {action_id:?}");
        }
        let local = item
            .id
            .as_str()
            .strip_prefix("demo/")
            .unwrap_or_default()
            .to_string();
        let id = ViewId(format!("demo/{local}"));
        let view = match local.as_str() {
            LIST => list_view(id),
            GRID => grid_view(id),
            DETAIL => detail_view(id),
            FORM => form_view(id),
            SCRATCHPAD => scratchpad_view(id),
            SLOW => {
                return Ok(vec![toast(
                    ToastKind::Success,
                    "Demo: Slow",
                    Some("Type a query: results arrive after 300 ms".to_string()),
                )]);
            }
            SLOW_RESULT => {
                return Ok(vec![toast(
                    ToastKind::Success,
                    "Slow answer chosen",
                    Some(item.title.clone()),
                )]);
            }
            PANIC => panic!("demo: Demo: Panic was asked to panic"),
            _ => anyhow::bail!("demo: unknown item {}", item.id.as_str()),
        };
        Ok(vec![Effect::PushView(view)])
    }

    fn is_dynamic_source(&self) -> bool {
        true
    }

    /// Answers every non-empty query that mentions "slow", after 300 ms.
    fn search(&mut self, query: &str) -> anyhow::Result<SearchAnswer> {
        let query = query.trim();
        if !query.to_lowercase().contains("slow") {
            return Ok(SearchAnswer::from(vec![]));
        }
        std::thread::sleep(SLOW_DELAY);
        Ok(SearchAnswer::from(vec![item(
            SLOW_RESULT,
            &format!("Slow answer for \"{query}\""),
            "Computed by a dynamic source after 300 ms",
        )]))
    }

    fn on_view_event(&mut self, view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
        Ok(match event {
            ViewEvent::ActionChosen { row_id, action_id } if action_id == "copy" => {
                let name = row_id
                    .strip_prefix("row-")
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|n| n.checked_sub(1))
                    .and_then(|ix| ROW_NAMES.get(ix));
                match name {
                    Some(name) => vec![
                        Effect::CopyText(name.to_string()),
                        toast(ToastKind::Success, "Copied", Some(name.to_string())),
                    ],
                    None => vec![toast(
                        ToastKind::Error,
                        "Nothing to copy",
                        Some(format!("unknown row {row_id}")),
                    )],
                }
            }
            ViewEvent::ActionChosen { row_id, action_id } => vec![toast(
                ToastKind::Success,
                "Action chosen",
                Some(format!("{action_id} on {row_id}")),
            )],
            ViewEvent::FormSubmitted { values } => {
                let message = values
                    .iter()
                    .map(|(id, value)| format!("{id}={}", describe(value)))
                    .collect::<Vec<_>>()
                    .join(", ");
                vec![toast(ToastKind::Success, "Form submitted", Some(message))]
            }
            ViewEvent::TextChanged { revision, text } => {
                vec![Effect::UpdateView(View::Scratchpad(ScratchpadView {
                    id: view.clone(),
                    title: "Demo: Scratchpad".to_string(),
                    annotations: annotate(&text),
                    text,
                    revision,
                    actions: pad_actions(),
                    output: None,
                    toolbar: Vec::new(),
                    busy: false,
                    placeholder: String::new(),
                    detail: None,
                }))]
            }
            ViewEvent::QueryChanged { .. } | ViewEvent::Closed => vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(local: &str) -> anyhow::Result<Vec<Effect>> {
        let it = items()
            .into_iter()
            .find(|i| i.id.as_str() == format!("demo/{local}"))
            .expect("item exists");
        DemoModule::new().run_action(&it, "open")
    }

    fn pushed(local: &str) -> View {
        match run(local).unwrap().as_slice() {
            [Effect::PushView(v)] => v.clone(),
            other => panic!("expected one PushView, got {other:?}"),
        }
    }

    #[test]
    fn items_are_valid_and_owned() {
        let all = items();
        assert_eq!(all.len(), 7);
        for it in &all {
            assert!(it.id.is_valid());
            assert_eq!(it.id.provider(), "demo");
            assert_eq!(it.actions[0].id, "open");
        }
        assert!(all.iter().any(|i| i.title == "Demo: Panic"));
    }

    #[test]
    fn list_has_sections_and_a_detail_pane() {
        let View::List(v) = pushed(LIST) else {
            panic!("not a list")
        };
        assert!(v.show_detail);
        assert_eq!(v.id, ViewId("demo/list".into()));
        let sections: std::collections::BTreeSet<_> =
            v.rows.iter().filter_map(|r| r.section.clone()).collect();
        assert_eq!(sections.len(), 2);
        assert!(
            v.rows
                .iter()
                .all(|r| r.detail_markdown.is_some() && !r.actions.is_empty())
        );
    }

    #[test]
    fn list_rows_have_a_copy_action_with_a_shortcut() {
        let View::List(v) = pushed(LIST) else {
            panic!("not a list")
        };
        for row in &v.rows {
            let with_shortcut: Vec<&Action> = row
                .actions
                .iter()
                .filter(|a| a.shortcut.is_some())
                .collect();
            let [copy] = with_shortcut.as_slice() else {
                panic!("expected one action with a shortcut, got {with_shortcut:?}")
            };
            assert_eq!(copy.shortcut.as_deref(), Some("ctrl+shift+c"));
            assert!(copy.hotkey().unwrap().is_ok());
        }
        let effects = DemoModule::new()
            .on_view_event(
                &v.id,
                ViewEvent::ActionChosen {
                    row_id: "row-2".into(),
                    action_id: "copy".into(),
                },
            )
            .unwrap();
        assert!(
            matches!(effects.as_slice(), [Effect::CopyText(t), Effect::Toast(_)] if t == "Banana"),
            "{effects:?}"
        );
    }

    #[test]
    fn grid_has_twelve_tiles_of_every_image_kind() {
        let View::Grid(v) = pushed(GRID) else {
            panic!("not a grid")
        };
        assert_eq!(v.tiles.len(), 12);
        let images: Vec<&Icon> = v.tiles.iter().filter_map(|t| t.image.as_ref()).collect();
        assert!(images.iter().all(|i| i.is_valid()), "{images:?}");
        assert!(images.iter().any(|i| matches!(i, Icon::Named(_))));
        assert!(images.iter().any(|i| matches!(i, Icon::Emoji(_))));
        assert!(images.iter().any(|i| matches!(i, Icon::Color(_))));
    }

    #[test]
    fn detail_carries_html_image_and_links() {
        let View::Detail(v) = pushed(DETAIL) else {
            panic!("not a detail")
        };
        assert!(v.markdown.contains("<script>"));
        assert!(v.markdown.contains("!["));
        assert!(v.markdown.contains("](https://"));
        assert!(v.markdown.contains("](file:///etc/passwd)"));
    }

    #[test]
    fn form_has_every_field_kind_and_submit_toasts_the_values() {
        let View::Form(v) = pushed(FORM) else {
            panic!("not a form")
        };
        let kinds: Vec<_> = v
            .fields
            .iter()
            .map(|f| std::mem::discriminant(&f.kind))
            .collect();
        for k in [
            FieldKind::Text {
                value: String::new(),
                placeholder: String::new(),
            },
            FieldKind::Multiline {
                value: String::new(),
            },
            FieldKind::Checkbox { value: false },
            FieldKind::Dropdown {
                options: vec![],
                value: String::new(),
            },
            FieldKind::Date { value: None },
        ] {
            assert!(kinds.contains(&std::mem::discriminant(&k)), "missing {k:?}");
        }
        assert!(
            v.fields.iter().any(|f| f.error.is_some()),
            "no field shows an error"
        );
        let effects = DemoModule::new()
            .on_view_event(
                &v.id,
                ViewEvent::FormSubmitted {
                    values: vec![
                        ("name".into(), FieldValue::Text("Ann".into())),
                        ("subscribe".into(), FieldValue::Bool(true)),
                        ("due".into(), FieldValue::Date(Some("2026-11-18".into()))),
                    ],
                },
            )
            .unwrap();
        let [Effect::Toast(t)] = effects.as_slice() else {
            panic!("expected one toast")
        };
        assert_eq!(t.kind, ToastKind::Success);
        let message = t.message.as_deref().unwrap();
        assert!(message.contains("name=\"Ann\"") && message.contains("subscribe=true"));
        assert!(message.contains("due=2026-11-18"));
    }

    #[test]
    fn scratchpad_annotates_line_lengths_and_flags_err() {
        let effects = DemoModule::new()
            .on_view_event(
                &ViewId("demo/scratchpad".into()),
                ViewEvent::TextChanged {
                    revision: 7,
                    text: "abc\n\nan err here".into(),
                },
            )
            .unwrap();
        let [Effect::UpdateView(View::Scratchpad(s))] = effects.as_slice() else {
            panic!("expected one UpdateView")
        };
        // The pad keeps its actions across updates (Ruling AQ).
        assert_eq!(s.actions, pad_actions());
        assert_eq!(s.revision, 7);
        assert_eq!(
            s.annotations,
            vec![
                LineAnnotation {
                    line: 0,
                    text: "3 chars".into(),
                    is_error: false,
                    swatch: None,
                },
                LineAnnotation {
                    line: 2,
                    text: "error: 11 chars".into(),
                    is_error: true,
                    swatch: None,
                },
            ]
        );
    }

    #[test]
    fn slow_source_answers_after_300_ms_and_only_for_slow_queries() {
        let mut m = DemoModule::new();
        assert!(m.is_dynamic_source());
        assert!(m.search("calc").unwrap().items.is_empty());
        let started = std::time::Instant::now();
        let found = m.search("slow").unwrap().items;
        assert!(started.elapsed() >= SLOW_DELAY);
        assert_eq!(found.len(), 1);
        assert!(found[0].id.is_valid() && found[0].id.provider() == "demo");
    }

    #[test]
    fn slow_result_row_can_be_chosen() {
        let found = DemoModule::new().search("slow").unwrap().items;
        let effects = DemoModule::new().run_action(&found[0], "open").unwrap();
        let [Effect::Toast(t)] = effects.as_slice() else {
            panic!("expected one toast")
        };
        assert_eq!(t.kind, ToastKind::Success);
    }

    #[test]
    fn panic_item_panics_in_run_action() {
        let caught = std::panic::catch_unwind(|| run(PANIC));
        assert!(caught.is_err());
    }

    #[test]
    fn closed_view_needs_no_effects() {
        let effects = DemoModule::new()
            .on_view_event(&ViewId("demo/scratchpad".into()), ViewEvent::Closed)
            .unwrap();
        assert!(effects.is_empty());
    }

    #[test]
    fn row_action_toasts() {
        let effects = DemoModule::new()
            .on_view_event(
                &ViewId("demo/list".into()),
                ViewEvent::ActionChosen {
                    row_id: "row-2".into(),
                    action_id: "pick".into(),
                },
            )
            .unwrap();
        assert!(matches!(effects.as_slice(), [Effect::Toast(t)] if t.kind == ToastKind::Success));
    }
}
