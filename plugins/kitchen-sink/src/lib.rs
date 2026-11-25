//! Test plugin for the tomakast plugin host. It implements the whole `plugin` world; each
//! behaviour a host test needs is selected by a `search` query or a `run` action id.
//!
//! `init` indexes one item, `kitchen-sink/indexed`, whose action ids select behaviours. It puts
//! no tile on Home: a test does, through the core.
//!
//! Widgets (`[[widgets]]` of `plugin.toml`, drawn with the SDK's frame builder):
//! - `static`: a column with a title (`Static`) and a body;
//! - `buttons`: two buttons, `inc` and `reset`, whose `action` events answer a toast of that
//!   name;
//! - `anim`: a focusable canvas, `animate: true`, whose first rect moves with `millis-shown`;
//!   it also holds every `draw` case (a rect, an ellipse, a path of every segment, text, a PNG
//!   and an SVG image (sent once by `init`), a clip and a transform), both paints and both colours;
//! - `echo` (double, param `text`): the condition tile of `tile:<text>`, which draws its
//!   param `text`;
//! - `quarter-a` and `quarter-b` (quarters, param `text`): the condition tiles of `quarters`,
//!   which draw `Quarter`, their param `text` and the size the render context gives;
//! - `probe`: drawn in the mode `render:<mode>` sets (`ok` at first):
//!   - `ok`: a column with the text `ok`;
//!   - `spin`: never returns;
//!   - `slow:<ms>`: busy-waits `<ms>`, then draws the text `slow`;
//!   - `err`: answers `Err`;
//!   - `big`: 257 nodes, one over the cap;
//!   - `bad-tree`: node 1 names node 5 as its parent;
//!   - `bad-svg:<href|script|huge>`: sends the SVG image `bad-svg` that links to
//!     `/etc/hostname`, holds a `<script>`, or has 4097 elements;
//!   - `image:<png|jpeg|svg>`: draws the text `image:<format>` and sends the image
//!     `probe-<format>` (a 1x1 PNG, an embedded 1x1 JPEG, a small SVG);
//!   - `all`: every node kind (row, column, overlay, text, icon `icons/tile.png`, image,
//!     progress, spacer, button, canvas) and an inline image of each format (`all-png`,
//!     `all-jpeg`, `all-svg`);
//!   - `icon:<path>`: draws an icon node of the plugin file `<path>`;
//!   - `open-url:<url>`: calls `open.open-url` while drawing, and draws its result as text.
//!
//! Widget events (`on-event` with `event.widget`), so the tests see which of them are user
//! actions:
//! - the `action` `copy` answers `copy-text(<text>)` (the tile's param `text`) and
//!   `copy-twice` answers it twice;
//!   `inc` and `reset` answer a toast of that name; any other action id is answered as the
//!   reply of that name (see "Replies" below, such as `open-url:<url>`);
//! - a `key` answers `copy-text(<text>)`;
//! - a pointer `down`, `up` or `move` on the node `close` answers `close-window`, on `push` a
//!   detail view `pushed`, on `pop` `pop-view`, and on any other node `copy-text(<text>)`.
//!
//! The `home` event is kept (the latest list only) and answers no effects; the `run` action
//! `home` shows it.
//!
//! Other events answer no effects.
//!
//! Queries understood by `search`:
//! - `spin`: never returns (an infinite loop), to test deadlines;
//! - `slow:<text>`: busy-waits 90 ms, then answers one item titled `<text>`;
//! - `echo:<text>`: one item `kitchen-sink/echo` titled `<text>`, with every field set (its
//!   icon is the plugin file `icons/echo.png`);
//! - `bad-shortcut`: one item whose action has a 100 KiB multi-line shortcut;
//! - `icon:<path>`: one item whose icon is the plugin file `<path>`;
//! - `foreign-id`: one own item and two whose ids lack the `kitchen-sink/` prefix;
//! - `many:<n>`: `<n>` items;
//! - `sleep`: blocks in `std::thread::sleep` for 10 s (a blocking WASI call);
//! - `tile:<text>`: no items and one widget answer, the `echo` tile with param `text` =
//!   `<text>` and the actions `copy` and `copy-twice`;
//! - `tile-undeclared`: one item and a widget answer for `nope`, a widget it does not declare;
//! - `tile-undeclared-param`: one item and an `echo` answer with the param `nope`, which
//!   `echo` does not declare;
//! - `tile-repeated-param`: one item and an `echo` answer with the param `text` twice;
//! - `tile-unoffered-size`: one item and an `echo` answer in the size `quarter`, which `echo`
//!   (a double) does not offer;
//! - `quarters`: no items and two widget answers, the quarter tiles `quarter-a` and
//!   `quarter-b`, params `text` = `A` and `B`, action `copy`.
//!
//! Action ids understood by `run`:
//! - `run` on `kitchen-sink/hello` (the manifest command): a success toast "hello";
//! - `echo-item`: a detail view whose metadata lists every field of the item it received;
//! - `big-view`: a list view with 2001 rows (one over the cap);
//! - `replace`: `replace-items` with an own item, a foreign one and one that reuses the id of
//!   the manifest command;
//! - `hog:<mib>`: allocates and touches `<mib>` MiB, then answers with no effects;
//! - `big-return:<mib>`: answers `copy-text` with `<mib>` MiB of text;
//! - `err-flood`: answers `Err` with 1 MiB of lines;
//! - `replace-flood`: `replace-items` with one own item and 1000 foreign ones;
//! - `bad-shortcut`: a list view whose row action has a 100 KiB multi-line shortcut;
//! - `panic`: panics;
//! - `storage`: runs a fixed sequence of storage calls, answered as detail-view metadata;
//! - `locate`: `location.request`; nothing, or a detail view `locate` with its `error`. The
//!   `location` event answers a detail view `location` with `latitude`, `longitude` and
//!   `accuracy-km`, or `error`;
//! - `storage-flood:<n>`: `<n>` storage writes in one call, answered as detail view `storage`
//!   (`written`, `error`: the first error);
//! - `prefs`: reads preferences, answered as detail-view metadata;
//! - `timer:<millis>:<once|repeat>`: starts a timer, answered as detail view `timer` with the
//!   result in `result`;
//! - `timer-reply:<millis>:<reply>`: a one-shot timer whose firing is answered with `<reply>`
//!   (see below) instead of the `timer-fired` view;
//! - `busy-timer:<millis>:<work>`: a repeating timer whose every firing busy-waits `<work>` ms,
//!   then answers detail view `busy-fired` (`id`);
//! - `cancel-race`: starts timer 1 (10 ms), whose firing cancels timer 2 (20 ms), then
//!   busy-waits 300 ms so that both are due before the call returns;
//! - `fetch:<METHOD> <url>[|<header>=<value>]…[|body=<text>]`: starts a request with those
//!   headers and body, answered as detail view `fetch` (`result`);
//! - `log:<text>`: logs `<text>` at info level and answers no effects;
//! - `close-window`: answers `close-window`;
//! - `view:<kind>`: pushes a small valid view of that kind (`list`, `grid`, `detail`, `form`
//!   or `scratchpad`), with id `<kind>`;
//! - `toasts:<n>`, `updates:<n>`, `copy:<text>`, `replace-many:<n>`, `cancel:<id>`,
//!   `open-url:<url>`, `open-path:<path>`: the reply of that name;
//! - `tz`: detail view `tz` whose metadata `zone` is what `locale.local-time-zone` answered;
//! - `render:<mode>`: the mode `probe` draws in (see above), answered with no effects;
//! - `redraw:<id>`: `redraw-widget(<id>)`;
//! - `home`: detail view `home` whose metadata `tiles` lists the Home tiles of the latest
//!   `home` event, in its order, as `<widget>` or `<widget> <key>=<value>,…`, joined by `; `
//!   (empty before any);
//! - `set-image:<id>:<png|jpeg|svg>` and `image-flood:<n>`: the replies of that name.
//!
//! Threat-model probes (`run` actions; each answers detail view `probe` whose metadata records
//! what the guest saw, so a denial is visible as data and the plugin keeps running):
//! - `wasi-fs`: reads `/etc/passwd`, lists `.` and `/`, writes `./probe.txt`;
//! - `wasi-env`: the environment (variable count and names), the arguments and the current
//!   directory;
//! - `wasi-tcp:<port>`: a TCP connect to `127.0.0.1:<port>` and a name lookup of `localhost`;
//! - `wasi-stdout:<text>`: writes `<text>` to stdout and stderr and records the write results;
//! - `markdown`: a detail view `markdown` whose markdown holds HTML and remote images;
//! - `bad-ids:<target>`: an effect whose id at `<target>` (`view`, `row`, `action`, `field`,
//!   `tile` or `item`) has a control or format character.
//!
//! An event of the view `bad-ids` is answered with a list view whose row id has a newline.
//!
//! Replies (`run` actions and `timer-reply`): `toasts:<n>` is `<n>` success toasts,
//! `updates:<n>` is `<n>` updates of a detail view `live`, `copy:<text>` copies `<text>`,
//! `replace-many:<n>` replaces the plugin's items with `<n>` items, `cancel:<id>` cancels
//! timer `<id>` and answers no effects, `open-url:<url>` / `open-path:<path>` call `open` and
//! answer detail view `open` (`result`); `open-twice:<url>` opens `<url>`, then the path `/`,
//! then `<url>` again in one call (`result`, `second`, `third`); `set-image:<id>:<format>` is
//! one `set-widget-image` of the sample image of that format named `<id>`;
//! `image-flood:<n>` is `<n>` `set-widget-image`s of the sample PNG, named `flood0` and on
//! (`timer-reply:<millis>:image-flood:<n>` sends them unsolicited).
//!
//! `on-event` answers a view event with a toast: the view id as title, the event as message.
//! An event of the view `oversized` is answered with a list view of 2001 rows instead; any
//! event of a view `reply:<reply>` is answered with that reply (see below).
//! A fired timer is answered with detail view `timer-fired` (`id`), unless it was started by
//! `timer-reply`; an HTTP answer with detail view `http-done` (`request`, `status`, `error`,
//! `body-len`, `body` (its first KiB), `x-test` (that response header)).
//! Anything else answers with no items or no effects.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write as _;
use std::time::{Duration, Instant};

// Built on the SDK, but mostly from the raw records: several behaviours send what the SDK's
// builders refuse (ids with control characters, foreign item ids).
use tomakast_plugin::{
    Accessory, Action, Color, ColorToken, CubicTo, DetailView, Draw, Effect, Event, FieldKind,
    FormField, FormView, GridTile, GridView, HttpDone, HttpRequest, Icon, ImageFit, Item, KeyValue,
    Layout, ListRow, ListView, MenuOption, Node, Paint, PathSegment, Plugin, Point, PointerKind,
    QuadTo, ScratchpadView, SearchAnswer, SearchMode, TextStyle, Toast, ToastKind, ToolbarButton,
    Transform, View, ViewEvent, WidgetAnswer, WidgetContext, WidgetEvent, WidgetFrame, WidgetImage,
    WidgetParam, WidgetSize, WidgetTarget, export_plugin, http, locale, location, log, open, param,
    preferences, storage, timers,
};

thread_local! {
    /// The replies of timers started by `timer-reply`, by timer id.
    static TIMER_REPLIES: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::new());
    /// How long each firing of a timer started by `busy-timer` busy-waits, by timer id.
    static BUSY_TIMERS: RefCell<HashMap<u64, u64>> = RefCell::new(HashMap::new());
    /// The mode the `probe` widget draws in (`render:<mode>`).
    static PROBE_MODE: RefCell<String> = RefCell::new("ok".into());
    /// The Home tiles of the latest `home` event, as the `home` action lists them.
    static HOME: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The sample PNG (1x1).
const PNG: &[u8] = include_bytes!("../icons/echo.png");
/// The 16x16 PNG tile of the `anim` and `all` drawings.
const TILE: &[u8] = include_bytes!("../icons/tile.png");
/// The sample JPEG (1x1, grey).
const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x10, 0x0b, 0x0c, 0x0e, 0x0c, 0x0a, 0x10,
    0x0e, 0x0d, 0x0e, 0x12, 0x11, 0x10, 0x13, 0x18, 0x28, 0x1a, 0x18, 0x16, 0x16, 0x18, 0x31, 0x23,
    0x25, 0x1d, 0x28, 0x3a, 0x33, 0x3d, 0x3c, 0x39, 0x33, 0x38, 0x37, 0x40, 0x48, 0x5c, 0x4e, 0x40,
    0x44, 0x57, 0x45, 0x37, 0x38, 0x50, 0x6d, 0x51, 0x57, 0x5f, 0x62, 0x67, 0x68, 0x67, 0x3e, 0x4d,
    0x71, 0x79, 0x70, 0x64, 0x78, 0x5c, 0x65, 0x67, 0x63, 0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x01,
    0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xff, 0xc4, 0x00, 0x14, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xc4, 0x00, 0x14,
    0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3f, 0x00, 0x3f, 0xff, 0xd9,
];
/// The sample SVG.
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#3080ff"/></svg>"##;

struct KitchenSink;

fn action(id: &str, title: &str) -> Action {
    Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
    }
}

fn item(local: &str, title: &str) -> Item {
    Item {
        id: format!("kitchen-sink/{local}"),
        title: title.into(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        actions: vec![action("open", "Open")],
    }
}

fn detail(id: &str, metadata: Vec<(String, String)>) -> Vec<Effect> {
    vec![Effect::PushView(View::DetailView(DetailView {
        id: id.into(),
        title: id.into(),
        markdown: String::new(),
        metadata: metadata
            .into_iter()
            .map(|(key, value)| KeyValue { key, value })
            .collect(),
        actions: Vec::new(),
    }))]
}

impl Plugin for KitchenSink {
    fn init() -> Result<Vec<Effect>, String> {
        let mut indexed = item("indexed", "Indexed by init");
        indexed.actions = vec![action("echo-item", "Echo"), action("big-view", "Big view")];
        Ok(vec![
            Effect::ReplaceItems(vec![indexed]),
            // `anim`'s images are sent once, here; its frames only name them.
            Effect::set_widget_image(WidgetImage::png("anim-png", TILE)),
            Effect::set_widget_image(WidgetImage::svg("anim-svg", SVG)),
        ])
    }

    fn search(query: String) -> Result<SearchAnswer, String> {
        if let Some(text) = query.strip_prefix("tile:") {
            let tile = WidgetAnswer::new("echo")?
                .param("text", text)
                .action("copy", "Copy")?
                .action("copy-twice", "Copy twice")?;
            return Ok(SearchAnswer {
                items: Vec::new(),
                widgets: vec![tile],
            });
        }
        if query == "quarters" {
            let quarter = |id: &str, text: &str| -> Result<WidgetAnswer, String> {
                Ok(WidgetAnswer::new(id)?
                    .param("text", text)
                    .action("copy", "Copy")?)
            };
            return Ok(SearchAnswer {
                items: Vec::new(),
                widgets: vec![quarter("quarter-a", "A")?, quarter("quarter-b", "B")?],
            });
        }
        if query == "tile-undeclared" {
            return Ok(SearchAnswer {
                items: vec![item("undeclared", "Undeclared")],
                widgets: vec![WidgetAnswer::new("nope")?],
            });
        }
        if let Some(bad) = bad_tile(&query)? {
            return Ok(SearchAnswer {
                items: vec![item("bad-tile", "Bad tile")],
                widgets: vec![bad],
            });
        }
        Ok(search_items(query)?.into())
    }

    fn render_widget(id: String, context: WidgetContext) -> Result<WidgetFrame, String> {
        match id.as_str() {
            "static" => Ok(text_frame(&["Static", "A column with a title and a body"])),
            "buttons" => {
                let frame = WidgetFrame::new(Node::row(Layout::default().gap(8.)));
                let root = frame.root();
                let (frame, _) = frame.child(root, Node::button("inc", "inc").label("+1"));
                let (frame, _) = frame.child(root, Node::button("reset", "reset").label("Reset"));
                Ok(frame.build())
            }
            "anim" => {
                let x = (context.millis_shown % 2000) as f32 / 2000. * context.width;
                let rect = Draw::rect(x, 40., 20., 20.)
                    .fill(Paint::solid(Color::token(ColorToken::Selected)));
                let mut commands = vec![rect];
                commands.extend(every_draw());
                let frame = WidgetFrame::new(Node::column(Layout::default()));
                let root = frame.root();
                let canvas = Node::canvas("anim", commands).focusable().grow(1.);
                let (frame, _) = frame.child(root, canvas);
                Ok(frame.animate(true).build())
            }
            "echo" => Ok(text_frame(&[
                param(&context.params, "text").unwrap_or_default()
            ])),
            "quarter-a" | "quarter-b" => Ok(text_frame(&[
                "Quarter",
                param(&context.params, "text").unwrap_or_default(),
                &format!("{} x {}", context.width, context.height),
            ])),
            "probe" => probe_frame(&PROBE_MODE.with(|m| m.borrow().clone())),
            _ => Err(format!("no widget {id}")),
        }
    }

    fn run(item: Item, action_id: String) -> Result<Vec<Effect>, String> {
        if let Some(mib) = action_id.strip_prefix("hog:") {
            let mib: usize = mib.parse().map_err(|_| "bad size".to_string())?;
            let mut chunks: Vec<Vec<u8>> = Vec::new();
            for _ in 0..mib {
                chunks.push(std::hint::black_box(vec![1u8; 1024 * 1024]));
            }
            drop(std::hint::black_box(chunks));
            return Ok(Vec::new());
        }
        if let Some(effects) = reply_to(&action_id) {
            return effects;
        }
        if let Some(effects) = widget_action(&action_id) {
            return effects;
        }
        if let Some(rest) = action_id.strip_prefix("timer:") {
            let (millis, kind) = rest.split_once(':').ok_or("bad timer")?;
            let millis: u64 = millis.parse().map_err(|_| "bad millis".to_string())?;
            let result = timers::start(millis, kind == "repeat");
            return Ok(detail(
                "timer",
                vec![("result".into(), format!("{result:?}"))],
            ));
        }
        if let Some(rest) = action_id.strip_prefix("busy-timer:") {
            let (millis, work) = rest.split_once(':').ok_or("bad timer")?;
            let millis: u64 = millis.parse().map_err(|_| "bad millis".to_string())?;
            let work: u64 = work.parse().map_err(|_| "bad millis".to_string())?;
            let result = timers::start(millis, true);
            if let Ok(id) = result {
                BUSY_TIMERS.with(|b| b.borrow_mut().insert(id, work));
            }
            return Ok(detail(
                "timer",
                vec![("result".into(), format!("{result:?}"))],
            ));
        }
        if let Some(rest) = action_id.strip_prefix("timer-reply:") {
            let (millis, reply) = rest.split_once(':').ok_or("bad timer")?;
            let millis: u64 = millis.parse().map_err(|_| "bad millis".to_string())?;
            let result = timers::start(millis, false);
            if let Ok(id) = result {
                TIMER_REPLIES.with(|r| r.borrow_mut().insert(id, reply.to_string()));
            }
            return Ok(detail(
                "timer",
                vec![("result".into(), format!("{result:?}"))],
            ));
        }
        if action_id == "cancel-race" {
            // Timer 1 cancels timer 2 when it fires; both come due while this call still runs,
            // so one wake-up takes both events.
            let first = timers::start(10, false)?;
            let second = timers::start(20, false)?;
            TIMER_REPLIES.with(|r| r.borrow_mut().insert(first, format!("cancel:{second}")));
            busy(Duration::from_millis(300));
            return Ok(detail(
                "timer",
                vec![("result".into(), format!("{first} {second}"))],
            ));
        }
        if let Some(rest) = action_id.strip_prefix("fetch:") {
            let (method, rest) = rest.split_once(' ').ok_or("bad fetch")?;
            let mut parts = rest.split('|');
            let url = parts.next().unwrap_or_default();
            let mut request = HttpRequest {
                method: method.into(),
                url: url.into(),
                headers: Vec::new(),
                body: None,
            };
            for part in parts {
                let (key, value) = part.split_once('=').ok_or("bad fetch part")?;
                if key == "body" {
                    request.body = Some(value.as_bytes().to_vec());
                } else {
                    request.headers.push(KeyValue {
                        key: key.into(),
                        value: value.into(),
                    });
                }
            }
            let result = http::fetch(&request);
            return Ok(detail(
                "fetch",
                vec![("result".into(), format!("{result:?}"))],
            ));
        }
        if let Some(effects) = probe(&action_id) {
            return effects;
        }
        if let Some(n) = action_id.strip_prefix("storage-flood:") {
            return Ok(detail("storage", storage_flood(n)));
        }
        if let Some(kind) = action_id.strip_prefix("view:") {
            return small_view(kind).map(|view| vec![Effect::PushView(view)]);
        }
        if let Some(text) = action_id.strip_prefix("log:") {
            log::info(text);
            return Ok(Vec::new());
        }
        if let Some(mib) = action_id.strip_prefix("big-return:") {
            let mib: usize = mib.parse().map_err(|_| "bad size".to_string())?;
            return Ok(vec![Effect::CopyText("x".repeat(mib * 1024 * 1024))]);
        }
        match action_id.as_str() {
            "run" if item.id == "kitchen-sink/hello" => Ok(vec![Effect::Toast(Toast {
                kind: ToastKind::Success,
                title: "hello".into(),
                message: None,
            })]),
            "echo-item" => Ok(detail("echo", describe_item(&item))),
            "big-view" => Ok(vec![Effect::PushView(View::ListView(big_list(2001)))]),
            "replace" => {
                let mut theirs = self::item("x", "Theirs");
                theirs.id = "other/theirs".into();
                Ok(vec![Effect::ReplaceItems(vec![
                    self::item("replaced", "Replaced"),
                    theirs,
                    self::item("hello", "Not the command"),
                ])])
            }
            "close-window" => Ok(vec![Effect::CloseWindow]),
            "panic" => panic!("kitchen-sink was asked to panic"),
            "err-flood" => Err("an error line\n".repeat(80 * 1024)),
            "replace-flood" => {
                let mut items = vec![self::item("flood", "Flood")];
                items.extend((0..1000).map(|i| {
                    let mut theirs = self::item("x", "Theirs");
                    theirs.id = format!("other/t{i}");
                    theirs
                }));
                Ok(vec![Effect::ReplaceItems(items)])
            }
            "bad-shortcut" => {
                let mut list = big_list(1);
                list.rows[0].actions = vec![bad_shortcut()];
                Ok(vec![Effect::PushView(View::ListView(list))])
            }
            "storage" => Ok(detail("storage", storage_script())),
            "locate" => Ok(match location::request() {
                Ok(()) => Vec::new(),
                Err(e) => detail("locate", vec![("error".into(), e)]),
            }),
            "home" => Ok(detail(
                "home",
                vec![("tiles".into(), HOME.with(|h| h.borrow().clone()))],
            )),
            "tz" => Ok(detail(
                "tz",
                vec![
                    ("zone".into(), locale::local_time_zone()),
                    ("languages".into(), locale::languages().join(",")),
                ],
            )),
            // The default of a declared preference, and one of a key it does not declare,
            // which the launcher drops.
            "preference-default" => Ok(vec![
                Effect::preference_default("name", "Ann"),
                Effect::preference_default("undeclared", "x"),
            ]),
            "prefs" => Ok(detail(
                "prefs",
                ["greeting", "name", "loud", "undeclared"]
                    .into_iter()
                    .map(|key| (key.to_string(), format!("{:?}", preferences::get(key))))
                    .collect(),
            )),
            _ => Ok(Vec::new()),
        }
    }

    fn on_event(event: Event) -> Result<Vec<Effect>, String> {
        let target = match event {
            Event::View(target) => target,
            Event::TimerFired(id) => {
                if let Some(millis) = BUSY_TIMERS.with(|b| b.borrow().get(&id).copied()) {
                    busy(Duration::from_millis(millis));
                    return Ok(detail("busy-fired", vec![("id".into(), id.to_string())]));
                }
                let reply = TIMER_REPLIES.with(|r| r.borrow_mut().remove(&id));
                return match reply {
                    Some(reply) => reply_to(&reply).unwrap_or(Ok(Vec::new())),
                    None => Ok(detail("timer-fired", vec![("id".into(), id.to_string())])),
                };
            }
            Event::HttpDone(done) => return Ok(detail("http-done", describe_http(done))),
            Event::Location(answer) => {
                let metadata = match answer {
                    Ok(at) => vec![
                        ("latitude".into(), at.latitude.to_string()),
                        ("longitude".into(), at.longitude.to_string()),
                        ("accuracy-km".into(), at.accuracy_km.to_string()),
                    ],
                    Err(e) => vec![("error".into(), e)],
                };
                return Ok(detail("location", metadata));
            }
            Event::Widget(target) => return Ok(widget_event(target)),
            Event::Home(tiles) => {
                let listed: Vec<String> = tiles
                    .iter()
                    .map(|tile| match &tile.params[..] {
                        [] => tile.widget_id.clone(),
                        params => format!("{} {}", tile.widget_id, pairs(params)),
                    })
                    .collect();
                HOME.with(|h| *h.borrow_mut() = listed.join("; "));
                return Ok(Vec::new());
            }
            Event::PreferencesChanged => {
                return Ok(vec![Effect::Toast(Toast {
                    kind: ToastKind::Success,
                    title: "preferences-changed".into(),
                    message: Some(format!("greeting={:?}", preferences::get("greeting"))),
                })]);
            }
        };
        if target.view_id == "oversized" {
            return Ok(vec![Effect::UpdateView(View::ListView(big_list(2001)))]);
        }
        if target.view_id == "bad-ids" {
            return bad_ids("row");
        }
        if let Some(reply) = target.view_id.strip_prefix("reply:")
            && let Some(effects) = reply_to(reply)
        {
            return effects;
        }
        let message = match target.event {
            ViewEvent::QueryChanged(q) => format!("query-changed {} {}", q.revision, q.text),
            ViewEvent::ActionChosen(a) => format!("action-chosen {} {}", a.row_id, a.action_id),
            ViewEvent::FormSubmitted(entries) => {
                let entries: Vec<String> = entries
                    .iter()
                    .map(|e| format!("{}={:?}", e.field_id, e.value))
                    .collect();
                format!("form-submitted {}", entries.join(","))
            }
            ViewEvent::TextChanged(t) => format!("text-changed {} {}", t.revision, t.text),
            ViewEvent::Closed => "closed".to_string(),
        };
        Ok(vec![Effect::Toast(Toast {
            kind: ToastKind::Success,
            title: target.view_id,
            message: Some(message),
        })])
    }
}

/// The items `search` answers for `query` (see the module docs).
fn search_items(query: String) -> Result<Vec<Item>, String> {
    if query == "spin" {
        spin();
    }
    if query == "sleep" {
        std::thread::sleep(Duration::from_secs(10));
        return Ok(Vec::new());
    }
    if let Some(text) = query.strip_prefix("slow:") {
        busy(Duration::from_millis(90));
        return Ok(vec![item("slow", text)]);
    }
    if let Some(text) = query.strip_prefix("echo:") {
        return Ok(vec![Item {
            id: "kitchen-sink/echo".into(),
            title: text.into(),
            subtitle: Some("a subtitle".into()),
            keywords: vec!["one".into(), "two".into()],
            icon: Some(Icon::PluginFile("icons/echo.png".into())),
            actions: vec![
                action("echo-item", "Echo"),
                Action {
                    id: "copy".into(),
                    title: "Copy".into(),
                    shortcut: Some("ctrl+shift+c".into()),
                },
            ],
        }]);
    }
    if let Some(path) = query.strip_prefix("icon:") {
        let mut with_icon = item("icon", "With icon");
        with_icon.icon = Some(Icon::PluginFile(path.into()));
        return Ok(vec![with_icon]);
    }
    if query == "bad-shortcut" {
        let mut bad = item("bad", "Bad shortcut");
        bad.actions = vec![bad_shortcut()];
        return Ok(vec![bad]);
    }
    if query == "foreign-id" {
        let mut theirs = item("x", "Theirs");
        theirs.id = "other/theirs".into();
        let mut bare = item("x", "No prefix");
        bare.id = "no-prefix".into();
        return Ok(vec![item("mine", "Mine"), theirs, bare]);
    }
    if let Some(n) = query.strip_prefix("many:") {
        let n: usize = n.parse().map_err(|_| "bad count".to_string())?;
        return Ok((0..n).map(|i| item(&format!("m{i}"), "Many")).collect());
    }
    Ok(Vec::new())
}

/// `toasts:<n>`, `updates:<n>`, `copy:<text>` or `replace-many:<n>` as effects.
fn reply_to(reply: &str) -> Option<Result<Vec<Effect>, String>> {
    let (kind, arg) = reply.split_once(':')?;
    let count = || arg.parse::<usize>().map_err(|_| "bad count".to_string());
    Some(match kind {
        "toasts" => count().map(|n| {
            (0..n)
                .map(|i| {
                    Effect::Toast(Toast {
                        kind: ToastKind::Success,
                        title: format!("toast {i}"),
                        message: None,
                    })
                })
                .collect()
        }),
        "updates" => count().map(|n| {
            (0..n)
                .map(|i| {
                    let mut view = detail("live", vec![("n".into(), i.to_string())]);
                    let Some(Effect::PushView(view)) = view.pop() else {
                        unreachable!()
                    };
                    Effect::UpdateView(view)
                })
                .collect()
        }),
        "copy" => Ok(vec![Effect::CopyText(arg.to_string())]),
        "open-url" => Ok(detail(
            "open",
            vec![("result".into(), format!("{:?}", open::url(arg)))],
        )),
        "open-path" => Ok(detail(
            "open",
            vec![("result".into(), format!("{:?}", open::path(arg)))],
        )),
        "open-twice" => {
            let first = format!("{:?}", open::url(arg));
            let second = format!("{:?}", open::path("/"));
            let third = format!("{:?}", open::url(arg));
            Ok(detail(
                "open",
                vec![
                    ("result".into(), first),
                    ("second".into(), second),
                    ("third".into(), third),
                ],
            ))
        }
        "cancel" => arg.parse().map_err(|_| "bad id".to_string()).map(|id| {
            timers::cancel(id);
            Vec::new()
        }),
        "replace-many" => count().map(|n| {
            let items = (0..n).map(|i| item(&format!("r{i}"), "Replaced")).collect();
            vec![Effect::ReplaceItems(items)]
        }),
        "set-image" => match arg.split_once(':') {
            Some((id, format)) => {
                sample_image(id, format).map(|image| vec![Effect::set_widget_image(image)])
            }
            None => Err("bad set-image".into()),
        },
        "image-flood" => count().map(|n| {
            (0..n)
                .map(|i| Effect::set_widget_image(WidgetImage::png(format!("flood{i}"), PNG)))
                .collect()
        }),
        _ => return None,
    })
}

/// The widget `run` actions `render:<mode>` and `redraw:<id>`, or `None` for another action.
fn widget_action(action_id: &str) -> Option<Result<Vec<Effect>, String>> {
    let (kind, arg) = action_id.split_once(':')?;
    Some(match kind {
        "render" => {
            PROBE_MODE.with(|m| *m.borrow_mut() = arg.to_string());
            Ok(Vec::new())
        }
        "redraw" => Ok(vec![Effect::redraw_widget(arg)]),
        _ => return None,
    })
}

/// The answer to an event of a widget's tile (see the crate doc).
fn widget_event(target: WidgetTarget) -> Vec<Effect> {
    let text = param(&target.params, "text").unwrap_or_default().to_owned();
    let copy = || Effect::copy_text(text.clone());
    match target.event {
        WidgetEvent::Action(a) => match a.action_id.as_str() {
            "copy" => vec![copy()],
            "copy-twice" => vec![copy(), copy()],
            "inc" | "reset" => vec![Effect::toast(Toast::success(a.action_id))],
            other => reply_to(other).and_then(Result::ok).unwrap_or_default(),
        },
        WidgetEvent::Key(_) => vec![copy()],
        WidgetEvent::Pointer(p) => match (p.kind, p.node_id.as_str()) {
            (PointerKind::Enter | PointerKind::Leave, _) => Vec::new(),
            (_, "close") => vec![Effect::close_window()],
            (_, "push") => detail("pushed", Vec::new()),
            (_, "pop") => vec![Effect::pop_view()],
            _ => vec![copy()],
        },
        WidgetEvent::Shown | WidgetEvent::Hidden => Vec::new(),
    }
}

/// `params` as `<key>=<value>,…`.
fn pairs(params: &[WidgetParam]) -> String {
    let pairs: Vec<String> = params
        .iter()
        .map(|p| format!("{}={}", p.key, p.value))
        .collect();
    pairs.join(",")
}

/// The `echo` answers the host rejects whole, by query (see the crate doc), or `None` for
/// another query. Built from the raw record where the SDK's builder would not send it.
fn bad_tile(query: &str) -> Result<Option<WidgetAnswer>, String> {
    let echo = || WidgetAnswer::new("echo").map(|a| a.param("text", "x"));
    Ok(Some(match query {
        "tile-undeclared-param" => echo()?.param("nope", "x"),
        "tile-repeated-param" => {
            let mut answer = echo()?;
            answer.params.push(WidgetParam {
                key: "text".into(),
                value: "y".into(),
            });
            answer
        }
        "tile-unoffered-size" => echo()?.size(WidgetSize::Quarter),
        _ => return Ok(None),
    }))
}

/// A column of text nodes, the first a title.
fn text_frame(texts: &[&str]) -> WidgetFrame {
    let mut frame = WidgetFrame::new(Node::column(Layout::default().padding(8.).gap(4.)));
    let root = frame.root();
    for (i, text) in texts.iter().enumerate() {
        let style = if i == 0 {
            TextStyle::Title
        } else {
            TextStyle::Body
        };
        frame = frame.child(root, Node::text(*text, style)).0;
    }
    frame.build()
}

/// One of each `draw` case, with every `path-segment`, both `paint`s and both `color`s. It
/// names the images `anim-png` and `anim-svg`.
fn every_draw() -> Vec<Draw> {
    let at = |x, y| Point { x, y };
    let gradient = Paint::linear(45., Color::rgb(0xff8000), Color::token(ColorToken::Success));
    let path = Draw::path(vec![
        PathSegment::MoveTo(at(4., 4.)),
        PathSegment::LineTo(at(40., 4.)),
        PathSegment::QuadTo(QuadTo {
            ctrl: at(50., 20.),
            to: at(40., 36.),
        }),
        PathSegment::CubicTo(CubicTo {
            ctrl_a: at(30., 50.),
            ctrl_b: at(10., 50.),
            to: at(4., 36.),
        }),
        PathSegment::Close,
    ])
    .fill(gradient);
    vec![
        Draw::ellipse(70., 20., 14., 8.).fill(Paint::solid(Color::rgba(0x30c0ff80))),
        path,
        Draw::text(90., 4., "draw", 12.),
        Draw::image("anim-png", 90., 24., 16., 16.),
        Draw::svg("anim-svg", 110., 24., 16., 16.),
        Draw::push_clip(0., 60., 60., 20.),
        Draw::rect(0., 60., 100., 100.).fill(Paint::solid(Color::token(ColorToken::Text))),
        Draw::pop_clip(),
        Draw::push_transform(Transform::translate(70., 60.).scale(1., 1.).rotate(15.)),
        Draw::rect(0., 0., 16., 16.).stroke(2., Paint::solid(Color::rgb(0x3080ff))),
        Draw::pop_transform(),
    ]
}

/// A row, a column, an overlay, a text, an icon, an image, a progress bar, a spacer, a button
/// and a canvas, and an inline image of each format (the `probe` mode `all`).
fn all_nodes_frame() -> WidgetFrame {
    let frame = WidgetFrame::new(Node::column(Layout::default().padding(4.).gap(2.)));
    let root = frame.root();
    let (frame, row) = frame.child(root, Node::row(Layout::default().gap(4.)));
    let (frame, _) = frame.child(row, Node::text("all", TextStyle::Caption));
    let (frame, _) = frame.child(
        row,
        Node::icon(Icon::PluginFile("icons/tile.png".into()), 16.),
    );
    let (frame, _) = frame.child(
        row,
        Node::image("all-png", ImageFit::Contain).size(16., 16.),
    );
    let (frame, _) = frame.child(row, Node::spacer());
    let (frame, _) = frame.child(row, Node::button("go", "go").label("Go"));
    let (frame, overlay) = frame.child(root, Node::overlay(Layout::default()));
    let (frame, _) = frame.child(overlay, Node::progress(0.5).height(8.));
    let (frame, _) = frame.child(overlay, Node::text("50%", TextStyle::Caption));
    let canvas = Node::canvas("all-canvas", vec![Draw::rect(0., 0., 8., 8.)]).height(12.);
    let (frame, _) = frame.child(root, canvas);
    frame
        .image(WidgetImage::png("all-png", TILE))
        .image(WidgetImage::jpeg("all-jpeg", JPEG))
        .image(WidgetImage::svg("all-svg", SVG))
        .build()
}

/// The sample image of `format` (`png`, `jpeg` or `svg`), named `id`.
fn sample_image(id: &str, format: &str) -> Result<WidgetImage, String> {
    Ok(match format {
        "png" => WidgetImage::png(id, PNG),
        "jpeg" => WidgetImage::jpeg(id, JPEG),
        "svg" => WidgetImage::svg(id, SVG),
        _ => return Err("bad image format".into()),
    })
}

/// The `probe` widget's frame in `mode` (see the crate doc).
fn probe_frame(mode: &str) -> Result<WidgetFrame, String> {
    if mode == "spin" {
        spin();
    }
    if let Some(ms) = mode.strip_prefix("slow:") {
        let ms: u64 = ms.parse().map_err(|_| "bad millis".to_string())?;
        busy(Duration::from_millis(ms));
        return Ok(text_frame(&["slow"]));
    }
    if let Some(kind) = mode.strip_prefix("bad-svg:") {
        let svg = match kind {
            "href" => r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><image href="/etc/hostname" width="8" height="8"/></svg>"#.to_string(),
            "script" => r#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#.to_string(),
            "huge" => format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{}</svg>"#, "<g/>".repeat(4096)),
            _ => return Err("bad svg kind".into()),
        };
        return Ok(image_frame("bad-svg", WidgetImage::svg("bad-svg", svg)));
    }
    if let Some(url) = mode.strip_prefix("open-url:") {
        return Ok(text_frame(&[&format!("{:?}", open::url(url))]));
    }
    if let Some(path) = mode.strip_prefix("icon:") {
        let frame = WidgetFrame::new(Node::column(Layout::default()));
        let root = frame.root();
        let icon = Node::icon(Icon::PluginFile(path.into()), 16.);
        return Ok(frame.child(root, icon).0.build());
    }
    if let Some(format) = mode.strip_prefix("image:") {
        let image = sample_image(&format!("probe-{format}"), format)?;
        return Ok(image_frame(mode, image));
    }
    match mode {
        "ok" => Ok(text_frame(&["ok"])),
        "all" => Ok(all_nodes_frame()),
        "err" => Err("the probe was asked to fail".into()),
        "big" => {
            let mut frame = WidgetFrame::new(Node::column(Layout::default()));
            let root = frame.root();
            for _ in 0..256 {
                frame = frame.child(root, Node::spacer()).0;
            }
            Ok(frame.build())
        }
        "bad-tree" => {
            let mut frame = text_frame(&["a", "b"]);
            // Built valid, then broken: the builder never makes this.
            frame.nodes[1].parent = Some(5);
            Ok(frame)
        }
        _ => Err(format!("bad probe mode {mode}")),
    }
}

/// A column with the text `text` and an image node naming `image`, which the frame sends.
fn image_frame(text: &str, image: WidgetImage) -> WidgetFrame {
    let frame = WidgetFrame::new(Node::column(Layout::default()));
    let root = frame.root();
    let (frame, _) = frame.child(root, Node::text(text, TextStyle::Body));
    let (frame, _) = frame.child(root, Node::image(image.id.clone(), ImageFit::Contain));
    frame.image(image).build()
}

fn describe_http(done: HttpDone) -> Vec<(String, String)> {
    let mut out = vec![("request".to_string(), done.request.to_string())];
    match done.response {
        Ok(response) => {
            let head = &response.body[..response.body.len().min(1024)];
            let x_test = response
                .headers
                .iter()
                .find(|h| h.key.eq_ignore_ascii_case("x-test"))
                .map(|h| h.value.clone())
                .unwrap_or_default();
            out.push(("status".into(), response.status.to_string()));
            out.push(("body-len".into(), response.body.len().to_string()));
            out.push(("body".into(), String::from_utf8_lossy(head).into_owned()));
            out.push(("x-test".into(), x_test));
        }
        Err(error) => out.push(("error".into(), error)),
    }
    out
}

/// A small valid view of `kind`, with id `kind`.
fn small_view(kind: &str) -> Result<View, String> {
    Ok(match kind {
        "list" => View::ListView(big_list(1)),
        "grid" => View::GridView(GridView {
            id: "grid".into(),
            title: "Grid".into(),
            columns: 2,
            tiles: vec![GridTile {
                id: "t0".into(),
                image: Some(Icon::Emoji("🙂".into())),
                caption: "Tile".into(),
                actions: vec![action("pick", "Pick")],
            }],
        }),
        "detail" => {
            let Some(Effect::PushView(view)) = detail("detail", Vec::new()).pop() else {
                unreachable!()
            };
            view
        }
        "form" => View::FormView(FormView {
            id: "form".into(),
            title: "Form".into(),
            fields: vec![FormField {
                id: "name".into(),
                label: "Name".into(),
                kind: FieldKind::Checkbox(false),
                error: None,
            }],
            submit_title: "Submit".into(),
        }),
        "scratchpad" => View::ScratchpadView(ScratchpadView {
            id: "scratchpad".into(),
            title: "Pad".into(),
            text: "1+1".into(),
            revision: 0,
            annotations: Vec::new(),
            actions: Vec::new(),
            output: None,
            toolbar: Vec::new(),
            busy: false,
            placeholder: String::new(),
            detail: None,
        }),
        // A compose pad with a toolbar: a plain button and a menu.
        "compose" => View::ScratchpadView(ScratchpadView {
            id: "compose".into(),
            title: "Compose".into(),
            text: "hello".into(),
            revision: 0,
            annotations: Vec::new(),
            actions: Vec::new(),
            output: Some("**out**".into()),
            toolbar: vec![
                ToolbarButton {
                    id: "plain".into(),
                    label: "Plain".into(),
                    icon: Some(Icon::Named("arrow-left-right".into())),
                    shortcut: Some("ctrl+s".into()),
                    options: Vec::new(),
                    selected: None,
                },
                ToolbarButton {
                    id: "menu".into(),
                    label: "Menu".into(),
                    icon: None,
                    shortcut: None,
                    options: vec![MenuOption {
                        id: "one".into(),
                        title: "One".into(),
                        subtitle: Some("1".into()),
                        icon: Some(Icon::Emoji("1️⃣".into())),
                        keywords: vec!["first".into()],
                    }],
                    selected: Some("one".into()),
                },
            ],
            busy: true,
            placeholder: "Type here".into(),
            detail: Some("*notes*".into()),
        }),
        _ => return Err("bad view kind".into()),
    })
}

fn bad_shortcut() -> Action {
    Action {
        id: "bad".into(),
        title: "Bad".into(),
        shortcut: Some(format!("ctrl+\n{}", "x\n".repeat(50 * 1024))),
    }
}

fn describe_item(item: &Item) -> Vec<(String, String)> {
    let actions: Vec<String> = item
        .actions
        .iter()
        .map(|a| format!("{}:{}:{:?}", a.id, a.title, a.shortcut))
        .collect();
    vec![
        ("id".into(), item.id.clone()),
        ("title".into(), item.title.clone()),
        ("subtitle".into(), format!("{:?}", item.subtitle)),
        ("keywords".into(), item.keywords.join(",")),
        ("icon".into(), format!("{:?}", item.icon)),
        ("actions".into(), actions.join(",")),
    ]
}

fn big_list(rows: usize) -> ListView {
    ListView {
        id: "big".into(),
        title: "Big".into(),
        placeholder: String::new(),
        search: SearchMode::CoreFilter,
        query: String::new(),
        revision: 0,
        rows: (0..rows)
            .map(|i| ListRow {
                id: format!("r{i}"),
                section: None,
                title: format!("Row {i}"),
                subtitle: None,
                icon: None,
                accessories: vec![Accessory::Text("a".into())],
                detail_markdown: None,
                actions: Vec::new(),
            })
            .collect(),
        show_detail: false,
    }
}

/// Every storage call the tests check, in order, as `(step, result)`.
fn storage_script() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut record = |step: &str, result: String| out.push((step.to_string(), result));
    record("set", format!("{:?}", storage::set("k", "v")));
    record("get", format!("{:?}", storage::get("k")));
    record("remove", format!("{:?}", storage::remove("k")));
    record("get-removed", format!("{:?}", storage::get("k")));
    record(
        "long-key",
        format!("{:?}", storage::set(&"k".repeat(257), "v")),
    );
    let big = "x".repeat(256 * 1024 + 1);
    record("big-value", format!("{:?}", storage::set("big", &big)));
    // 200 KiB values: five fit in 1 MiB, the sixth does not.
    let value = "v".repeat(200 * 1024);
    let mut stored = 0;
    let mut error = String::new();
    for i in 0..10 {
        match storage::set(&format!("fill{i}"), &value) {
            Ok(()) => stored += 1,
            Err(e) => {
                error = e;
                break;
            }
        }
    }
    record("fill-stored", stored.to_string());
    record("fill-error", error);
    out
}

/// `n` storage writes in a row (sets and removes, alternating): how many succeeded and the
/// first error.
fn storage_flood(n: &str) -> Vec<(String, String)> {
    let n: usize = n.parse().unwrap_or(0);
    let mut written = 0;
    let mut error = String::new();
    for i in 0..n {
        let result = if i % 2 == 0 {
            storage::set("flood", "v")
        } else {
            storage::remove("flood")
        };
        match result {
            Ok(()) => written += 1,
            Err(e) if error.is_empty() => error = e,
            Err(_) => {}
        }
    }
    vec![
        ("written".into(), written.to_string()),
        ("error".into(), error),
    ]
}

/// The threat-model probes (see the crate doc), or `None` for another action.
fn probe(action_id: &str) -> Option<Result<Vec<Effect>, String>> {
    let (name, arg) = action_id.split_once(':').unwrap_or((action_id, ""));
    let seen = match name {
        "wasi-fs" => vec![
            (
                "passwd",
                outcome(std::fs::read("/etc/passwd").map(|b| b.len())),
            ),
            ("dot", outcome(std::fs::read_dir(".").map(|d| d.count()))),
            ("root", outcome(std::fs::read_dir("/").map(|d| d.count()))),
            ("write", outcome(std::fs::write("probe.txt", b"x"))),
        ],
        "wasi-env" => vec![
            // Variable names only, and the count: values could exceed the metadata cap.
            ("env-count", std::env::vars_os().count().to_string()),
            (
                "env-names",
                format!(
                    "{:?}",
                    std::env::vars_os().map(|(k, _)| k).collect::<Vec<_>>()
                ),
            ),
            (
                "args",
                format!("{:?}", std::env::args_os().collect::<Vec<_>>()),
            ),
            ("cwd", outcome(std::env::current_dir())),
        ],
        "wasi-tcp" => {
            let port: u16 = match arg.parse() {
                Ok(port) => port,
                Err(_) => return Some(Err("bad port".into())),
            };
            vec![
                (
                    "tcp",
                    outcome(std::net::TcpStream::connect(("127.0.0.1", port))),
                ),
                (
                    "lookup",
                    outcome(
                        std::net::ToSocketAddrs::to_socket_addrs(&("localhost", port))
                            .map(|a| a.count()),
                    ),
                ),
            ]
        }
        "wasi-stdout" => {
            let out = std::io::stdout()
                .write_all(arg.as_bytes())
                .and_then(|()| std::io::stdout().flush());
            let err = std::io::stderr()
                .write_all(arg.as_bytes())
                .and_then(|()| std::io::stderr().flush());
            vec![("stdout", outcome(out)), ("stderr", outcome(err))]
        }
        "markdown" => {
            return Some(Ok(vec![Effect::PushView(View::DetailView(DetailView {
                id: "markdown".into(),
                title: "Markdown".into(),
                markdown: HOSTILE_MARKDOWN.into(),
                metadata: Vec::new(),
                actions: Vec::new(),
            }))]));
        }
        "bad-ids" => return Some(bad_ids(arg)),
        _ => return None,
    };
    Some(Ok(detail(
        "probe",
        seen.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
    )))
}

/// `Ok(<value>)` or `Err(<error>)`, as probe metadata.
fn outcome<T: std::fmt::Debug>(result: std::io::Result<T>) -> String {
    match result {
        Ok(value) => format!("Ok({value:?})"),
        Err(e) => format!("Err({e})"),
    }
}

/// Markdown a hostile plugin might send: raw HTML (a script, an iframe, an image tag) and a
/// remote image that would be a tracking pixel if it were fetched.
const HOSTILE_MARKDOWN: &str = "# Hostile\n\n\
<script>alert(1)</script>\n\n\
<img src=\"https://tracker.example/tag.png\">\n\n\
![pixel](https://tracker.example/pixel.png)\n\n\
Inline <b>bold</b> and <iframe src=\"https://evil.example/\"></iframe>\n";

/// One effect whose id at `target` has a control or format character.
fn bad_ids(target: &str) -> Result<Vec<Effect>, String> {
    let mut list = big_list(1);
    let view = |list: ListView| Ok(vec![Effect::PushView(View::ListView(list))]);
    match target {
        "view" => {
            list.id = "big\u{1b}[31m".into();
            view(list)
        }
        "row" => {
            list.rows[0].id = "r0\nr1".into();
            view(list)
        }
        "action" => {
            list.rows[0].actions = vec![action("pick\u{7}", "Pick")];
            view(list)
        }
        "field" => Ok(vec![Effect::PushView(View::FormView(FormView {
            id: "form".into(),
            title: "Form".into(),
            fields: vec![FormField {
                id: "name\u{202e}".into(),
                label: "Name".into(),
                kind: FieldKind::Checkbox(false),
                error: None,
            }],
            submit_title: "Submit".into(),
        }))]),
        "tile" => Ok(vec![Effect::PushView(View::GridView(GridView {
            id: "grid".into(),
            title: "Grid".into(),
            columns: 1,
            tiles: vec![GridTile {
                id: "t\u{0}".into(),
                image: None,
                caption: "Tile".into(),
                actions: Vec::new(),
            }],
        }))]),
        "item" => Ok(vec![Effect::ReplaceItems(vec![item("i\r", "Bad id")])]),
        _ => Err("bad target".into()),
    }
}

/// Busy-waits `time` (a plugin may not sleep).
fn busy(time: Duration) {
    let start = Instant::now();
    while start.elapsed() < time {
        std::hint::spin_loop();
    }
}

fn spin() -> ! {
    let mut x: u64 = 0;
    loop {
        x = std::hint::black_box(x.wrapping_add(1));
    }
}

export_plugin!(KitchenSink);
