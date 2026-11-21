# tomakast-plugin: write tomakast plugins in Rust

A tomakast plugin is a WebAssembly component in a folder, next to a `plugin.toml` that says
what it is and what it may do. This crate is the Rust SDK for writing one. It gives you the
plugin API (WIT `tomakast:plugin@0.1.0`) as a trait, builders for what you send to the
launcher, and plain functions for what the launcher lends you (storage, preferences, timers,
web requests, opening links, the log).

The full contract is [`crates/plugin-api/wit/since_v0.1.0/plugin.wit`](../plugin-api/wit/since_v0.1.0/plugin.wit).
Its doc comments are the rules: permissions, deadlines, size caps, and what needs a user
action. The crate docs (`cargo doc --open`) repeat each rule where it applies.

Two guides go with this README:

- [Writing a plugin](../../docs/plugins/writing-a-plugin.md): the execution model, permissions
  and user actions, deadlines and budgets, views and effects, and the calculator plugin as a
  worked example;
- [The manifest](../../docs/plugins/manifest.md): every key of `plugin.toml`, with its type,
  limits and an example.

## Hello, world

The plugin below adds a command, "Say hello", to the launcher. It also answers what you type:
`hello Ada` shows a row "Hello, Ada!".

You need Rust with the `wasm32-wasip2` target (`rustup target add wasm32-wasip2`).

`Cargo.toml`:

```toml
[package]
name = "hello"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
# The SDK from the tomakast repository. Cargo checks out the whole repository, which the
# SDK needs: it reads the contract from `crates/plugin-api`. Pin a commit with `rev`.
tomakast-plugin = { git = "https://github.com/tomakast/tomakast", rev = "<commit>" }
# Or, from a local checkout of the repository:
# tomakast-plugin = { path = "../tomakast/crates/plugin-sdk" }

# Optional: a smaller plugin.wasm.
[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
strip = true
```

`plugin.toml`. The `id` is also the name of the plugin's folder: 1–32 characters from `a-z`,
`0-9` and `-`.

```toml
id = "hello"
name = "Hello"
version = "0.1.0"
api = "0.1.0"
author = "You"

[[commands]]
id = "greet"
title = "Say hello"
icon = "face-slightly-smiling"

[permissions]
search = true
```

`src/lib.rs`:

```rust
use tomakast_plugin::{Effect, Item, Plugin, SearchAnswer, Toast, export_plugin};

struct Hello;

impl Plugin for Hello {
    // Needs `search = true`: the plugin sees what you type.
    fn search(query: String) -> Result<SearchAnswer, String> {
        let Some(name) = query.strip_prefix("hello ").map(str::trim) else {
            return Ok(Vec::new().into());
        };
        if name.is_empty() {
            return Ok(Vec::new().into());
        }
        // Item ids are "<plugin id>/<your id>". `?` turns a bad id into an error the
        // launcher shows.
        let item = Item::new("hello/greeting", format!("Hello, {name}!"))?
            .subtitle("Press Enter to say it")
            .action("say", "Say it")?;
        // An answer of items only; an answer can also carry tiles (widget answers).
        Ok(vec![item].into())
    }

    fn run(item: Item, action_id: String) -> Result<Vec<Effect>, String> {
        let text = match action_id.as_str() {
            // The command of plugin.toml arrives as the item "hello/greet", action "run".
            "run" => "Hello, world!".to_string(),
            // The item `search` returned comes back whole.
            "say" => item.title,
            _ => return Ok(Vec::new()),
        };
        Ok(vec![Effect::toast(Toast::success(text))])
    }
}

export_plugin!(Hello);
```

Build it:

```sh
cargo build --release --target wasm32-wasip2
```

This writes `target/wasm32-wasip2/release/hello.wasm`.

If your plugin's folder is inside another Cargo workspace, add an empty `[workspace]` table to
its `Cargo.toml`. Cargo then builds it on its own.

## Install it

To install a plugin, copy its folder into the launcher's plugins folder. Name the folder
after the plugin's `id`, and put two files in it: `plugin.toml`, and the built component
renamed to `plugin.wasm`.

| System | Plugins folder |
|---|---|
| Linux | `~/.local/share/tomakast/plugins/` (or `$XDG_DATA_HOME/tomakast/plugins/`) |
| macOS | `~/Library/Application Support/tomakast/plugins/` |
| Windows | `%APPDATA%\tomakast\data\plugins\` |

If `TOMAKAST_HOME` is set, the plugins folder is `$TOMAKAST_HOME/plugins/`.

On Linux:

```sh
mkdir -p ~/.local/share/tomakast/plugins/hello
cp plugin.toml ~/.local/share/tomakast/plugins/hello/
cp target/wasm32-wasip2/release/hello.wasm ~/.local/share/tomakast/plugins/hello/plugin.wasm
```

You do not need to restart the launcher: it notices the new folder. Before the plugin first
runs, the launcher lists the permissions the plugin asks for, and you allow or deny them. The
decision is tied to the exact content of both files. When you rebuild and copy again, the
plugin reloads. It asks again if it now wants more permissions, or has another name or author.

## What the SDK gives you

- **`Plugin`**: `init`, `search`, `run`, `on_event` and `render_widget`, the five exports.
  Each has a default that answers nothing, except `render_widget`, whose default answers
  `Err` (there is no widget to draw). The functions take no `self`, because the launcher may
  unload a plugin and load it again. Keep in `storage` what must last.
- **`Plugin::search_with(query, readings)`**: `search` with the readings of the query, the
  same keys on another keyboard layout (`ашкуащч` → `firefox`) or a romanization (`опен` →
  `open`); at most 8, never equal to the query or to each other. By default it calls
  `search(query)`, so a plugin that implements only `search` keeps working. Implement it to
  find what the user typed on the wrong layout:

  ```rust
  fn search_with(query: String, readings: Vec<QueryReading>) -> Result<SearchAnswer, String> {
      // Try the query as typed, then each reading.
      let texts = std::iter::once(query).chain(readings.into_iter().map(|r| r.text));
      for text in texts {
          if text.eq_ignore_ascii_case("firefox") {
              return Ok(vec![Item::new("hello/firefox", "Firefox")?.action("open", "Open")?].into());
          }
      }
      Ok(Vec::new().into())
  }
  ```
- **`export_plugin!(Type)`**: once, in `lib.rs`.
- **Builders**, which check every id when you build:
  - `Item::new(id, title)`, which gives an `ItemDraft`. Its first `.action(id, title)` makes it
    an `Item`, because an item without actions is rejected;
  - `Action::new(id, title)`;
  - `View::list`, `View::grid`, `View::detail`, `View::form`, `View::scratchpad`, with
    `ListRow`, `GridTile`, `FormField`, `DropdownOption` and `LineAnnotation`;
  - `Effect::push_view`, `update_view`, `pop_view`, `close_window`, `toast`, `copy_text`
    and `replace_items`;
  - `Toast::success`, `Toast::error` and `Toast::progress`;
  - `HttpRequest::get` and `HttpRequest::new`;
  - widgets: `WidgetFrame::new(root)` with `.child(parent, node)`, `Node`, `Layout`, `Draw`,
    `Color`, `Paint`, `Transform`, `WidgetAnswer::new` with `.param`, `.size` and
    `.action`, `WidgetImage`, the effects `Effect::redraw_widget` and `set_widget_image`,
    and `param(&params, key)` to read a tile's param (see "Widgets").

  A bad id is an `InvalidId`, and `?` turns it into the `String` error of the export. The
  records' fields are public, for anything the builders do not cover.
- **Imports**:
  - `storage::{get, set, remove}`;
  - `preferences::get`;
  - `log::{debug, info, warn, error}`;
  - `timers::{start, cancel}`;
  - `http::fetch`;
  - `open::{url, path}`;
  - `locale::local_time_zone`;
  - `location::request` (the answer arrives as `Event::Location`).
- **`API_VERSION`** (`"0.1.0"`): the `api` of `plugin.toml` must be `API_VERSION`. The
  component records the version it was built for. A plugin built for a version the launcher
  does not run is refused, with an error toast that says "built for API x.y.z".

## Widgets

A widget is a tile in the strip above the launcher's list. Declare it in `plugin.toml`
(`sizes` is one or more of `"quarter"`, `"single"` and `"double"`, the first the default; its
params under `[[widgets.params]]`), and draw it in `render_widget`. A tile shows in two ways,
and neither needs a permission:

- from `search`, in a `WidgetAnswer`: the widget, its params (`.param(key, value)`, only the
  keys it declares), its size (`.size(..)`, else the default) and its actions. The user can
  put it on Home from its action menu ("Add to Home"), with the same params;
- on the user's Home, which the user keeps in `config.toml` as `[[home.widgets]]` entries.
  The plugin is told its own Home tiles in `Event::Home`, after `init` and whenever they
  change, so it can keep their data fresh while the launcher is hidden.

Every render context and widget event carries the tile's params: read one with
`param(&context.params, "zone")`. Two tiles of the same widget with the same params are the
same to the plugin.

A frame is a flat list of nodes: the root first, every later node under an earlier one:
`FrameBuilder::child` guarantees that order. Pass it a row, a column or an overlay as the
parent. In debug builds `build` also panics on a button without an id, a duplicate node id
and unbalanced clips or transforms.

```toml
[[widgets]]
id = "clock"
title = "Clock"
sizes = ["single", "quarter"]

[[widgets.params]]
key = "zone"
title = "Time zone"

[permissions]
search = true
timers = true
```

```rust,ignore
use tomakast_plugin::{
    Effect, Event, Layout, Node, Plugin, SearchAnswer, TextStyle, WidgetAnswer, WidgetContext,
    WidgetFrame, WidgetSize, export_plugin, param, timers,
};

struct Clock;

impl Plugin for Clock {
    fn init() -> Result<Vec<Effect>, String> {
        timers::start(1000, true)?; // a timer every second ...
        Ok(Vec::new())
    }

    fn search(query: String) -> Result<SearchAnswer, String> {
        // "time Europe/Oslo" shows a quarter tile for that zone.
        let Some(zone) = query.strip_prefix("time ") else {
            return Ok(Vec::new().into());
        };
        let tile = WidgetAnswer::new("clock")?
            .param("zone", zone)
            .size(WidgetSize::Quarter);
        Ok(SearchAnswer { items: Vec::new(), widgets: vec![tile] })
    }

    fn on_event(event: Event) -> Result<Vec<Effect>, String> {
        Ok(match event {
            // ... redraws every clock tile
            Event::TimerFired(_) => vec![Effect::redraw_widget("clock")],
            // The plugin's own Home tiles, e.g. to fetch their data in the background.
            Event::Home(_tiles) => Vec::new(),
            _ => Vec::new(),
        })
    }

    fn render_widget(_id: String, context: WidgetContext) -> Result<WidgetFrame, String> {
        let zone = param(&context.params, "zone").unwrap_or("local");
        let root = WidgetFrame::new(Node::column(Layout::default().padding(12.)));
        let at = root.root();
        let seconds = context.millis_shown / 1000; // use `locale::local_time_zone` for real time
        let (frame, _) = root.child(at, Node::text(format!("{zone}: {seconds} s"), TextStyle::Value));
        Ok(frame.build())
    }
}

export_plugin!(Clock);
```

`Draw` builds canvas commands (`Draw::rect(..).fill(Paint::solid(Color::rgb(0x336699)))`),
and `WidgetImage::png`, `jpeg` and `svg` carry images with `FrameBuilder::image` or
`Effect::set_widget_image`. The caps (nodes, commands, bytes, the 16 ms soft render
deadline) are in the doc comments of `plugin.wit`.

## Permissions

Under `[permissions]` in `plugin.toml`:

| Key | What it allows |
|---|---|
| `search = true` | `search` is called with what the user types |
| `storage = true` | `storage`: up to 1 MiB of key-value data |
| `clipboard = true` | the `copy_text` effect, in answer to a user action |
| `open = true` | `open::url` and `open::path`, during a user action |
| `timers = true` | `timers` |
| `index = true` | the `replace_items` effect: items in the launcher's index |
| `http = ["api.example.org", "*.example.net"]` | `http::fetch` to those domains |
| `location = true` | `location::request`: the computer's location, rounded to 0.1°, with its accuracy in km |

`preferences`, `log` and `locale` need no permission. A call the plugin is not allowed to
make fails. An effect it is not allowed to send is dropped, with a notice to the user.

## Rules worth knowing first

The full list is in `plugin.wit`.

- **Deadlines:** `search` must answer in 100 ms, `init` in 2 s, `run` and `on_event` in
  1 s. A plugin that misses a deadline, traps (panics), or goes over 64 MiB of memory is
  stopped, with an error toast. It stays stopped until its folder changes.
- **No ambient access:** WASI gives no files, environment, arguments, stdio or sockets. It
  gives the wall clock (`SystemTime`), the monotonic clock (`Instant`) and random numbers,
  and nothing else. Use the imports for the rest.
- **No blocking:** a blocking call such as `std::thread::sleep` stops the plugin. For
  "later", use a timer.
- **User actions:** copying text and opening links need one. That is `run`, or `on_event`
  for a chosen action, a submitted form, or a widget's `action`, `key` or completed click
  (an `up` on the node that got the last `down`). Typing (`query-changed`, `text-changed`),
  `closed` and the other widget events are not user actions; in answer to those widget
  events, pushing or popping a view and closing the launcher are dropped too.
- **Panics:** a panic message goes to the launcher's log, then the plugin is stopped.
  Return `Err` for errors the user can recover from.
- **Debugging:** `log::info` and the other log functions write to the launcher's log.

## Testing your plugin

The imports (`storage`, `preferences`, `log`, `timers`, `http`, `open`, `locale`, `location`) exist only
inside the launcher. A native `#[test]` that calls one fails to link or panics. So keep your
logic pure: parse, compute and build items and views in plain functions of their inputs,
and test those natively with `cargo test`. Call the imports in a thin layer around that
logic, and test the layer by running the plugin in the launcher. The calculator plugin in
the tomakast repository (`plugins/calc`) is built this way: its `engine` and `views`
modules have native tests, and its storage code is tested through the plugin host.

## Developing the SDK

The SDK is not a member of the tomakast workspace, because plugins built with it target
`wasm32-wasip2`. Its tests run natively:

```sh
cargo test --manifest-path crates/plugin-sdk/Cargo.toml
```
