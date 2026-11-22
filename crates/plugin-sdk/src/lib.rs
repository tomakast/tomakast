//! Write [tomakast](https://github.com/tomakast/tomakast) launcher plugins in Rust.
//!
//! A plugin is a WebAssembly component built for `wasm32-wasip2`. This crate wraps the
//! contract, WIT `tomakast:plugin@0.1.0`, and gives you:
//!
//! - the [`Plugin`] trait, one function per export, each with a default that does nothing,
//!   and [`export_plugin!`] to export your type;
//! - builders that check ids as you build ([`Item::new`], [`View::list`], [`Effect::push_view`]
//!   and the others in [`builders`]): a bad id is an [`InvalidId`] at the line that made it,
//!   and `?` turns it into the export's `Err`, which the launcher shows;
//! - plain functions for the imports: [`storage`], [`preferences`], [`log`], [`timers`],
//!   [`http`], [`open`] and [`locale`];
//! - every record of the contract, re-exported as is (its fields are public), for what the
//!   builders do not cover.
//!
//! The rules of the contract (permissions, deadlines, size caps, what needs a user action)
//! are the doc comments of `plugin.wit`; the documentation of each type repeats its part.
//!
//! ```no_run
//! use tomakast_plugin::{Effect, Item, Plugin, SearchAnswer, Toast, export_plugin};
//!
//! struct Hello;
//!
//! impl Plugin for Hello {
//!     fn search(query: String) -> Result<SearchAnswer, String> {
//!         if query != "hello" {
//!             return Ok(Vec::new().into());
//!         }
//!         Ok(vec![Item::new("hello/greet", "Say hello")?.action("greet", "Greet")?].into())
//!     }
//!
//!     fn run(_item: Item, _action_id: String) -> Result<Vec<Effect>, String> {
//!         Ok(vec![Effect::toast(Toast::success("Hello!"))])
//!     }
//! }
//!
//! export_plugin!(Hello);
//! ```
//!
//! A panic is written to the launcher's log (through [`log::error`]); the plugin then traps
//! and the launcher stops it as broken. Return `Err` for anything the user can recover from.
#![warn(missing_docs)]

pub mod builders;
mod imports;

pub use builders::{FrameBuilder, InvalidId, ItemDraft, NodeRef, param};
pub use imports::{http, locale, location, log, open, preferences, storage, timers};

/// The bindings `wit-bindgen` generates for the `plugin` world. Use the re-exports at the
/// crate root instead; this module is public only for [`export_plugin!`].
#[doc(hidden)]
#[allow(missing_docs, clippy::all)]
pub mod bindings {
    wit_bindgen::generate!({
        world: "plugin",
        path: "../plugin-api/wit/since_v0.1.0",
        pub_export_macro: true,
        default_bindings_module: "tomakast_plugin::bindings",
    });
}

/// Every type of the contract's `types` interface, with public fields.
pub use bindings::tomakast::plugin::types::{
    Accessory, Action, ActionChosen, Align, ButtonNode, CanvasNode, ClipRect, Color, ColorStop,
    ColorToken, Coordinates, CubicTo, DetailView, Draw, DropdownField, DropdownOption, Edges,
    Effect, EllipseCmd, Event, FieldKind, FieldValue, FormEntry, FormField, FormView, GridTile,
    GridView, HomeTile, HttpDone, HttpRequest, HttpResponse, Icon, IconNode, ImageCmd, ImageFit,
    ImageFormat, ImageNode, Item, Justify, KeyValue, Layout, LineAnnotation, LinearGradient,
    ListRow, ListView, MenuOption, Modifiers, Node, NodeKind, Paint, PathCmd, PathSegment, Point,
    PointerAt, PointerKind, QuadTo, QueryChanged, QueryReading, QueryReadingKind, RectCmd,
    RequestId, ScratchpadView, SearchAnswer, SearchMode, ShapeStyle, TextAlign, TextChanged,
    TextCmd, TextField, TextNode, TextSizes, TextStyle, ThemeColors, TimerId, Toast, ToastKind,
    ToolbarButton, Transform, View, ViewEvent, ViewTarget, WidgetAction, WidgetAnswer,
    WidgetContext, WidgetEvent, WidgetFrame, WidgetImage, WidgetKey, WidgetParam, WidgetPointer,
    WidgetSize, WidgetTarget, WidgetTheme,
};

/// A search answer of items only, no tiles: `Ok(items.into())`.
impl From<Vec<Item>> for SearchAnswer {
    fn from(items: Vec<Item>) -> SearchAnswer {
        SearchAnswer {
            items,
            widgets: Vec::new(),
        }
    }
}

/// The version of the plugin API (the WIT package `tomakast:plugin`) this crate is built
/// against. Write it as `api` in your `plugin.toml`.
///
/// The component records it too: every interface it imports is named
/// `tomakast:plugin/<interface>@0.1.0`, and the launcher refuses a plugin whose `plugin.toml`
/// says another version, or whose version it does not run.
pub const API_VERSION: &str = "0.1.0";

/// A plugin: the five exports of the contract. Every function has a default that answers
/// nothing (`render_widget`'s answers `Err`, as there is no widget to draw), so implement only
/// what your plugin does. The `search` export calls [`search_with`](Plugin::search_with),
/// which calls [`search`](Plugin::search) unless you implement it. Functions take no `self`:
/// the launcher
/// may unload an idle plugin and load it again, so keep what must last in [`storage`].
///
/// Each export has a deadline (`search` 100 ms, `init` 2000 ms, `run` and `on_event`
/// 1000 ms, `render_widget` 100 ms); a plugin that misses one is stopped as broken.
/// `render_widget` should also answer within 16 ms, or its frame is dropped.
pub trait Plugin {
    /// Called once after the user approved the plugin and it was loaded, and again after
    /// every reload. The commands of `plugin.toml` are indexed before it is called. Start
    /// timers here: they end with the plugin instance.
    fn init() -> Result<Vec<Effect>, String> {
        Ok(Vec::new())
    }

    /// Items and tiles for what the user typed, called on every keystroke. Called only when
    /// `plugin.toml` has `search = true` under `[permissions]`. At most 50 items, in the
    /// order to show them, and 16 widget answers. A list of items converts into an answer:
    /// `Ok(items.into())`.
    fn search(query: String) -> Result<SearchAnswer, String> {
        let _ = query;
        Ok(Vec::new().into())
    }

    /// [`search`](Self::search) with the readings of `query`: the same keys on another
    /// keyboard layout ("ашкуащч" → "firefox") or a romanization ("опен" → "open"). At most 8,
    /// never equal to `query` or to each other. Implement it to match a query typed on the
    /// wrong layout or in another script; by default it calls `search(query)`.
    fn search_with(query: String, readings: Vec<QueryReading>) -> Result<SearchAnswer, String> {
        let _ = readings;
        Self::search(query)
    }

    /// Runs `action_id` on `item`, which the user chose: an indexed item, one `search`
    /// returned, or a command of `plugin.toml` (as the item `<plugin-id>/<command-id>`
    /// with the action id `run`).
    fn run(item: Item, action_id: String) -> Result<Vec<Effect>, String> {
        let _ = (item, action_id);
        Ok(Vec::new())
    }

    /// A view event, a fired timer, an HTTP answer, a preference change, a widget event (with
    /// the tile's params), a location, or the plugin's own tiles on the user's Home
    /// (`Event::Home`: after `init` and whenever that list changes; never a user action).
    fn on_event(event: Event) -> Result<Vec<Effect>, String> {
        let _ = event;
        Ok(Vec::new())
    }

    /// Draws a tile of the widget `id`, one of the `[[widgets]]` of `plugin.toml`, for
    /// `context`. Called by the launcher only, at most 60 times per second per tile, while
    /// the tile is on screen or about to be shown. The default draws nothing: it answers
    /// `Err("no widget <id>")`.
    fn render_widget(id: String, context: WidgetContext) -> Result<WidgetFrame, String> {
        let _ = context;
        Err(format!("no widget {id}"))
    }
}

impl<T: Plugin> bindings::Guest for T {
    fn init() -> Result<Vec<Effect>, String> {
        log_panics();
        T::init()
    }

    fn search(query: String, readings: Vec<QueryReading>) -> Result<SearchAnswer, String> {
        log_panics();
        T::search_with(query, readings)
    }

    fn run(item: Item, action_id: String) -> Result<Vec<Effect>, String> {
        log_panics();
        T::run(item, action_id)
    }

    fn on_event(event: Event) -> Result<Vec<Effect>, String> {
        log_panics();
        T::on_event(event)
    }

    fn render_widget(id: String, context: WidgetContext) -> Result<WidgetFrame, String> {
        log_panics();
        T::render_widget(id, context)
    }
}

/// Sends panic messages to the launcher's log, once per plugin instance: WASI gives a plugin
/// no stderr, so they would be lost.
fn log_panics() {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        // "panicked at <file>:<line>:<col>:\n<message>"
        std::panic::set_hook(Box::new(|info| log::error(&info.to_string())));
    });
}

/// Exports `$ty`, a type that implements [`Plugin`], as the plugin's component. Use it once,
/// in your crate's `lib.rs` (built as a `cdylib`). `$ty` may be a name or a path.
///
/// ```no_run
/// struct Mine;
/// impl tomakast_plugin::Plugin for Mine {}
/// tomakast_plugin::export_plugin!(Mine);
/// ```
///
/// ```no_run
/// mod plugin {
///     pub struct Mine;
///     impl tomakast_plugin::Plugin for Mine {}
/// }
/// tomakast_plugin::export_plugin!(plugin::Mine);
/// ```
#[macro_export]
macro_rules! export_plugin {
    ($ty:path) => {
        // `export!` of wit-bindgen takes a bare name: a path goes through this alias.
        #[doc(hidden)]
        type __TomakastPluginExport = $ty;
        $crate::bindings::export!(__TomakastPluginExport with_types_in $crate::bindings);
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(text: &str) -> QueryReading {
        QueryReading {
            kind: QueryReadingKind::Layout,
            text: text.into(),
        }
    }

    /// Answers how many readings it got and the first one.
    struct EchoReadings;

    impl Plugin for EchoReadings {
        fn search_with(query: String, readings: Vec<QueryReading>) -> Result<SearchAnswer, String> {
            let first = readings.first().map_or("", |r| &*r.text);
            let title = format!("{query}: {} readings, first {first}", readings.len());
            Ok(vec![Item::new("echo/readings", title)?.action("run", "Run")?].into())
        }
    }

    /// Implements only `search`, as plugins written before readings do.
    struct QueryOnly;

    impl Plugin for QueryOnly {
        fn search(query: String) -> Result<SearchAnswer, String> {
            Ok(vec![Item::new("query/only", query)?.action("run", "Run")?].into())
        }
    }

    #[test]
    fn search_export_passes_readings_to_search_with() {
        let translit = QueryReading {
            kind: QueryReadingKind::Translit,
            text: "ashkuashch".into(),
        };
        let readings = vec![layout("firefox"), translit];
        let items = <EchoReadings as bindings::Guest>::search("ашкуащч".into(), readings)
            .unwrap()
            .items;
        assert_eq!(items.len(), 1);
        assert!(items[0].title.contains("firefox"), "{}", items[0].title);
        assert_eq!(items[0].title, "ашкуащч: 2 readings, first firefox");
    }

    #[test]
    fn search_with_defaults_to_search() {
        let items =
            <QueryOnly as bindings::Guest>::search("ашкуащч".into(), vec![layout("firefox")])
                .unwrap()
                .items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "ашкуащч");
    }

    #[test]
    fn a_list_of_items_is_an_answer_without_tiles() {
        let item = Item::new("p/x", "X")
            .unwrap()
            .action("open", "Open")
            .unwrap();
        let answer = SearchAnswer::from(vec![item]);
        assert_eq!(answer.items.len(), 1);
        assert_eq!(answer.items[0].id, "p/x");
        assert!(answer.widgets.is_empty());
    }

    #[test]
    fn api_version_is_the_wit_package_version() {
        let wit = include_str!("../../plugin-api/wit/since_v0.1.0/plugin.wit");
        let package = format!("package tomakast:plugin@{API_VERSION};");
        assert!(
            wit.lines().any(|line| line == package),
            "plugin.wit does not declare {package}"
        );
    }
}
