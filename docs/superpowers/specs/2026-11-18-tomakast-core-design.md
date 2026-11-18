# tomakast: core, plugin API and two proof plugins

Date: 2026-11-18. Status: draft for approval.

## Why

A Raycast-style launcher of my own, "built for me", that works the same on Linux, macOS and Windows. Raycast is macOS-only (Windows in beta), closed-source, written in JS; the existing open alternatives are either abandoned (Loungy: GPUI, mac only, no plugins; Gauntlet: iced + Deno/React, discontinued) or built on Qt and tied to the Raycast ecosystem (Vicinae).

This is the first of several specs. It covers the foundation: the core, the plugin API and two plugins that prove both loading paths work. The remaining features (dictation, translator, converter, clipboard, windows, caffeinate, notes, sync, plugin manager) are separate specs on top of the frozen API; they can be run as parallel sub-sessions only after the API in this document has stabilized.

## Goals

- Cross-platform without Electron/Tauri/webview. **Linux (the main machine is KDE Plasma on Wayland) and macOS are first-class platforms**: everything in this spec works and is verified on both. **Windows is the second priority, "but also important"** (the user's words); the author's interpretation: everything in this spec must also work and be verified, and it is acceptable to finish it after Linux and macOS. The user has a Mac and a Windows machine for manual testing.
- A small platform core: anything that is not "launching, search, rendering, hotkeys, storage, plugins" lives in modules.
- Third-party plugins in any language that compiles to WebAssembly (component model) load without rebuilding the app.
- The core provides plugins with a set of UI views; plugins describe the UI as data, the core renders it.
- Search is part of the core: a single index that modules/plugins feed items into; ranking happens in the core.
- Any command and any application can be assigned a global hotkey.
- Fast and lean: GPU rendering, ~0% CPU in the background, instant opening.
- A clean, modern, minimalist look, close to native on macOS and Windows and decent on Linux.

## Non-goals (in this spec)

- Dictation, translator, converter, clipboard history, window management, caffeinate, notes are separate specs.
- Sync. Here there is only the requirement that storage must not get in its way later (see "Storage").
- A plugin manager/store. Here, installation is by copying a folder.
- Compatibility with Raycast extensions.
- File search (it was in the original request) is a separate spec. The requirement on the core is already here: the index and matching must handle orders of magnitude more than 10k items (hundreds of thousands of paths), or files come in as a dynamic source with their own index. The index architecture must not rule this out.
- Packaging and distribution: installers, signing and notarization on macOS, auto-update (author's call: a separate spec; here it is enough to run the built binary and register autostart).
- GNOME on Wayland as a supported target: no layer-shell and limited portals. It must start as a regular window, but no polish is required.

## Anti-goals (failure, even if it "works")

- The launcher opens with a noticeable delay. The user's words: "it should feel like I pressed a key and it appeared instantly, as if I'd typed a character". Target: ~50 ms from hotkey to frame in the "warm" state.
- A third-party plugin can harm the user: read or send data without permission, execute code, bypass permission confirmation.
- The background process consumes CPU or wakes the GPU.
- A third-party plugin can hang or crash the UI.
- Plugins look inconsistent: a plugin author has no way to draw their "own" sloppy UI.
- GPUI types leak into the public plugin API (then GPUI churn breaks plugins).

## Constraints and facts the design relies on

- **GPUI's API is unstable.** The official `gpui` crate on crates.io has not been updated since 2025-10; the ecosystem lives on weekly `gpui-pre` snapshots and `gpui-kit`/`gpui-component` (0.6 → 0.10 in two months, every minor breaking). Decision: pin exact versions (`gpui-kit = "=0.10.0"`, which pins `gpui-pre =0.3.12`), upgrade deliberately. The churn is isolated in the rendering crate and invisible to plugins.
- **Wayland.** Global hotkeys, overlays, focus of the active app and the like are done differently in different compositors on Wayland. For KDE: `zwlr_layer_shell_v1` (supported by KWin, verified by the spike), the `org.freedesktop.portal.GlobalShortcuts` portal (present on the user's machine, verified via `busctl`), and as a fallback a CLI toggle that the user binds to a hotkey in the DE settings.
- **Spike results** (a throwaway prototype on KDE Wayland, GPUI via gpui-kit 0.10, layer-shell overlay, 10,321 items):
  - process start and item loading: 18 ms, RSS 26 MB; in the background without a window: 0 CPU ticks over 10 s;
  - **cold** first window open in a fresh process: ~175–210 ms to the first `render`, RSS grows to ~185 MB (GPU context initialization) and does not come back down after closing;
  - **repeat** opens in the same process (the window is closed and recreated): ~15 ms to the first `render` (3 measurements: 15.5 / 15.4 / 15.4 ms). The measurement is up to the `render` call, not up to the presented frame; the exact "hotkey → frame on screen" still needs to be measured;
  - by default GPUI on Linux exits the process when the last window closes → `cx.set_quit_mode(QuitMode::Explicit)` is needed;
  - filtering 10k rows and scrolling: the user rated responsiveness as fine;
  - rounded corners work, but an opaque white backing is visible behind the window. The cause is not established (candidates: root fill in gpui-kit/gpui-component, transparency handling in the Wayland backend, missing blur region); investigate at the start of the work.
  - Conclusion: recreating the window by itself fits the budget; only the first GPU initialization is expensive. So a "warm-up" is needed at daemon start (initialize the renderer up front), and keeping the window alive and hiding it is just one of the options if the "hotkey → frame" measurement does not fit. The requirement is the budget, not a specific mechanism.
- **Prior art for the plugin API:** Zed keeps its extension WIT interface versioned (`wit/since_v0.x.y/`), the host generates bindings for each supported version via `wasmtime::component::bindgen!` and reads the API version from a custom section in the `.wasm`. We adopt the same scheme.
- **The cost of "any language".** Rust/C/Zig/Go plugins are compact (hundreds of KB) and start fast; JS/Python via componentize-js/componentize-py drag along an interpreter (megabytes, slower start). This is acceptable; the first-class SDK is Rust.

## Approaches considered

**Plugin UI model**
- **A. A fixed set of views (as in Raycast): chosen.** The plugin returns data (list, grid, detail, form, action panel, plus "rich" core components), the core renders. Consistent look, small protocol, easy to version. Downside: non-standard UI requires a new component in the core.
- B. A tree of primitives (stack/text/input/image) with diffs. More flexible, but a large protocol that is hard to freeze, and an inconsistent look. Rejected for the sake of polish and API stability.

**Plugin runtime**
- **WASM component model on wasmtime, in-process: chosen** (the user is drawn to "any language"). Sandbox, limits (epoch interruption, memory), a plugin trap does not crash the host.
- Embedded JS (Deno/QuickJS): only one language, heavier. Rejected.
- Separate processes over IPC (like pop-launcher): stronger isolation, but more memory-hungry and more complex. Rejected for v1: under the "potentially malicious plugins" threat model the WASM sandbox is enough: the plugin has no WASI capabilities at all, only the core's host functions with permission checks, memory and time limits. Fallback if the sandbox proves insufficient (e.g. wasmtime vulnerabilities turn up): run wasmtime in a separate host process with the same WIT.
- Native Rust modules (`cdylib`): unstable Rust ABI, no sandbox. Not used for third-party plugins.

**Toolkit**
- **GPUI + gpui-component (via gpui-kit), versions pinned: chosen.** Best look, Zed runs on all three OSes, layer-shell is available, the KDE spike confirmed it.
- Slint: stable semver 1.x, Fluent/Cupertino styles, but licensed GPLv3 or royalty-free with conditions, and layer-shell support is unconfirmed. Fallback.
- iced: less polished look; Gauntlet stalled on it.

## Architecture

### Two loading paths, one internal API

- **Built-in modules** are Rust crates compiled into the binary that implement the internal module trait. They have access to platform APIs (needed for apps, later for dictation, clipboard, windows).
- **Third-party plugins** are WASM components. The host wraps each plugin in an adapter that implements the same internal trait. System access is only through imported host functions and only with permissions from the manifest.
- To the core both kinds are indistinguishable: they supply index items, commands and views.

### Crates (workspace)

Proposed layout (details are up to the builder):

- `tomakast-app`: the binary: startup, single-instance, CLI (`tomakast toggle`, `tomakast run <command-id>`).
- `tomakast-core`: module registry, index and ranking, commands, hotkeys (abstract), settings, storage. No dependencies on GPUI.
- `tomakast-ui`: rendering views in GPUI, the launcher window, theme. The only place where GPUI lives.
- `tomakast-platform`: platform shims: global hotkeys, overlay window/show-hide, autostart, opening files/URLs, single-instance IPC.
- `tomakast-plugin-host`: wasmtime, component loading, versioned bindings, limits, permissions.
- `tomakast-plugin-api`: WIT files (`wit/since_v0.1.0/…`) and the Rust guest SDK for plugin authors.
- `modules/apps`: the built-in application search module.
- `plugins/calc`: the calculator WASM plugin (built separately for `wasm32-wasip2`, kept in the repo as an example and a test).

### Process model and threads

- One daemon process. It starts at login (autostart is a platform shim) and keeps the window hidden.
- Launching the binary again makes it a client: it passes a command to the daemon over local IPC (Unix socket / named pipe) and exits. This is also the hotkey fallback on Wayland.
- The UI thread never waits on a plugin. Calls into plugins are asynchronous (wasmtime async + GPUI background executor), results are applied to the UI when ready. Stale responses (to an old query) are discarded.
- Each plugin has its own `Store`; limits: an epoch deadline per call (on the order of 100 ms for `search`, more for `run`), a memory limit. Trap or limit exceeded → the plugin is marked broken, the error is shown in the UI, the core keeps running.

### Launcher window

- The window can be recreated on every show or live permanently and be hidden; the builder chooses based on measurements; the criterion is the open budget (see "Budgets"). The GPU renderer is initialized ("warmed up") at daemon start, so that even the first open after login is fast.
- Linux/Wayland: layer-shell, Overlay layer, `KeyboardInteractivity::Exclusive` while visible.
- Linux/X11, macOS, Windows: a borderless always-on-top window; on macOS a non-activating panel (does not steal focus from the app it was invoked from), on Windows a tool window with no taskbar icon.
- Transparent background with rounded corners; the root is not filled with an opaque background. Blur/vibrancy where the platform provides it (macOS vibrancy, Windows Mica/Acrylic, KDE blur), otherwise a semi-transparent background. No blur is not a blocker.
- Hides on Esc, on focus loss and after a command runs (if the command did not open a view).
- Theme: light/dark following the system, accent color from the system where available.

### Search and ranking (core)

- **The index** is in memory; modules/plugins supply items in two ways:
  - *static sources*: a module publishes a set of items (applications, plugin commands) and updates it on events (`upsert`/`remove`);
  - *dynamic sources*: a plugin answers a `search(query)` request (calculator, later converter, files). Called in parallel, with a deadline; late ones do not block the results.
- **Index item**: stable id (`<provider-id>/<item-id>`), title, subtitle, keywords/aliases, icon, type, list of actions (the first one runs on Enter).
- **Matching**: fuzzy over title/keywords/aliases (with bonuses for word starts and consecutive characters). Target: ≤ 5 ms on 10k items per keystroke.
- **Ranking** = match relevance × usage weight. The usage weight combines three signals, as the user asked:
  - "always": frequency with decay (frecency);
  - "today": frequency within the current day;
  - "time of day": frequency within the current hour window (e.g. ±1–2 hours) across history.
  Plus a "query → selected item" binding (if "fi" usually opens Firefox, it comes first). The exact formulas and coefficients are up to the builder; they must live in one place and be covered by tests.
- **Empty query**: shows suggestions using the same weights (what is usually launched at this time).
- **Usage history** is written to storage every time an action runs.

### Commands and hotkeys

- **A command** is a named action of a module/plugin with id `<plugin-id>/<command-id>`, visible in search; it can open a view or run "without UI".
- A global hotkey can be assigned to any command and any index item (application); it is stored in settings as `hotkey → id`.
- Per-platform implementation: Windows: `RegisterHotKey`; macOS: Carbon `RegisterEventHotKey` (or equivalent); Linux/X11: XGrabKey; Linux/Wayland: the GlobalShortcuts portal if available, otherwise the user binds `tomakast run <id>` to a DE hotkey (show a hint in the UI).
- Local hotkeys inside the window: Enter is the primary action, Cmd/Ctrl+K the action panel, Esc back/close, Cmd/Ctrl+1…9 quick actions (at the builder's discretion).

### Views (UI protocol)

The core renders only these views; a plugin sends them as data:

- `list`: items with icon, title, subtitle, accessories (tags/text on the right), sections; optionally a detail panel on the right (markdown). A search field on top: filtering by the core, or forwarding the query to the plugin (`on-query`).
- `grid`: tiles (image + caption), for emoji/colors/images.
- `detail`: markdown content + metadata.
- `form`: fields: text, multiline text, checkbox, dropdown, date; submit → event to the plugin.
- `action-panel`: the list of actions for the current item (Cmd/Ctrl+K).
- `scratchpad`: a "rich" component: a multiline editor with an annotation to the right of each line, computed by the plugin (needed by the calculator, useful for the converter).
- Plus toasts/notifications (`toast`: success/error/progress).

Navigation is a view stack (push/pop), Esc pops. A plugin updates a view by sending its new version in full (enough for v1; diffs are a later optimization if needed).

The set of views is extended only through new API versions.

**What is frozen before parallel work** (author's call): only the public WIT `0.1.0` and the manifest format. The internal trait for built-in modules is not frozen: modules live in the same repository and are changed along with it. But the trait allows from the start: long-lived background services of a module, a module's own windows/overlays (dictation indicator, notes editor), a built-in module's own GPUI components beyond the fixed set of views.

### Plugin API (WIT)

Package `tomakast:plugin@0.1.0`. Sketch (exact signatures are up to the builder, but the structure is this):

- **Execution model** (author's call): a plugin is a set of short handlers that only the host calls; each call fits within the epoch deadline. Long-running or background work is done like this: from a handler, the plugin requests an asynchronous operation from the host (timer, HTTP request), the host performs it itself and, when it is ready, calls the plugin's `on-event` handler with the result. The plugin needs no async support in its language, so the model works for any language with the component model. Async component model exports (WASI 0.3) exist in wasmtime 50.x but are not used in 0.1: guest language support is still uneven. This can be added in a future API version.
- **Plugin exports**:
  - `init(ctx)`: once on load;
  - `search(query) -> list<item>`: only if the plugin declared itself a dynamic source;
  - `run(command-id, args) -> outcome`: `outcome` = close the window / show a toast / open a view;
  - `on-event(event) -> list<effect>`: a single entry point for events: view events (search input, action selection, form submit, scratchpad text change), a fired timer, a completed HTTP request, a preferences change. `effect` = update a view, show a toast, upsert/remove in the index, etc.
- **Imports (host capabilities)**, each requires a permission in the manifest:
  - `index`: upsert/remove the plugin's own static items;
  - `storage`: key-value in the plugin's own namespace;
  - `preferences`: reading the plugin's settings (declared in the manifest);
  - `clipboard`: write (read is a separate permission);
  - `open`: open a URL/path;
  - `http`: an asynchronous request through the host (`fetch(request) -> request-id`, the result arrives in `on-event`), with a domain allowlist from the manifest. Plain `wasi:http` is not given to the plugin;
  - `timers`: `schedule(delay | interval) -> timer-id`, `cancel`; firing arrives in `on-event`;
  - `log`, `toast`;
- Updates "without a user request" (asynchronous data loading, progress, periodic reindexing) are done through effects the plugin returns from `on-event` in response to a timer or HTTP. A separate push channel is not needed. The core may unload an inactive plugin and load it again; the plugin's timers survive the unload. Author's call: this covers the needs of the following specs without breaking API 0.1.
- **Versioning**: as in Zed: the WIT lives in `wit/since_v0.1.0/`; a breaking change → a new version folder; the host keeps bindings for all supported versions and picks by the version from the custom section in the `.wasm`.

### Plugin manifest and installation

- Plugin folder: `<data-dir>/plugins/<plugin-id>/` with `plugin.toml` and `plugin.wasm` (`data-dir` is platform-specific, via `directories`).
- `plugin.toml`: id, name, version, API version, author, list of commands (id, title, icon, mode: view/no-view), declared preferences, permissions (with a domain allowlist for http).
- Installation in v1 is by copying the folder; the core notices new/changed folders (file watcher) and reloads the plugin without restarting the app. Before the first run it shows the requested permissions and asks for confirmation; the decision is saved.
- Compiled modules are cached (wasmtime precompiled), so as not to spend time compiling on every start.

### Threat model and plugin security

Third-party plugins are considered **potentially malicious**, not just buggy.

- A plugin gets nothing except the capabilities declared in the manifest and approved by the user. WASI access to the filesystem, environment variables, sockets is not granted at all (only through the core's host functions).
- The permission decision is tied to the **hash of `plugin.wasm` + `plugin.toml`**, not just the id. If the plugin changed and its set of permissions **grew**, confirmation is requested again; if the permissions did not change, there is no prompt, but the hash change is recorded.
- `open`: URLs only with the `http`/`https`/`mailto` schemes (extendable via the core's allowlist); paths only to files/folders, not to executables (on Windows not `.exe/.bat/.cmd/.ps1/.lnk/.msi` etc., on macOS not `.app`/executables, on Linux not executable files and not `.desktop`). Launching applications is a privilege of built-in modules.
- `http`: only domains from the manifest allowlist, checked after redirects; limits on response size and number of concurrent requests.
- Quotas: `storage` size per plugin, number of items in `index`, rate of UI push updates, memory and execution time (epoch) per call.
- `clipboard` read is a separate permission, with explicit wording in the prompt.
- UI strings from a plugin are data only (markdown is rendered without HTML, without auto-loading external images unless there is an `http` permission for the domain), so that the core's system dialogs cannot be spoofed; the source of every item is visible (plugin icon/name).
- Host functions validate all input (sizes, counts, encodings); a plugin error does not crash the core.
- Built-in modules are trusted (they are application code); the trust boundary is WASM only.

### Storage and settings

- SQLite (one file in `data-dir`): usage history, query→item bindings, plugin KV, permission decisions.
- User settings (hotkeys, theme, enabled modules, plugin preferences) are a TOML file in `config-dir`, editable by hand; a settings UI comes later, as a separate spec.
- **Groundwork for sync** (sync itself is not here): all records have stable ids and modification timestamps, data is split into namespaces (core / each plugin), so that it can later be synced piece by piece. No specific sync mechanism is chosen.

## Two proof plugins

### `apps`: built-in application search module

Proves the "native module + static source + ranking" path.

- Linux: `.desktop` files from XDG data dirs (`/usr/share/applications`, `~/.local/share/applications`, flatpak/snap exports), honoring `NoDisplay`, `Hidden`, `OnlyShowIn`, localized `Name[ru]`, themed icons. Launch via `Exec` (through `gio launch`-like logic or a custom parser for the `%f/%u` field codes).
- macOS: `/Applications`, `~/Applications`, `/System/Applications` (`.app` bundles), name and icon from the bundle. Launch via `NSWorkspace`/`open`.
- Windows: Start Menu shortcuts (common and per-user), UWP apps desirable but not required in v1. Launch via `ShellExecute`.
- Index updates when directories change (watcher).
- Actions: open (Enter), show in file manager, copy path.

### `calc`: calculator-scratchpad WASM plugin

Proves the "third-party WASM plugin + dynamic source + rich view + storage" path. Written in Rust with the guest SDK, built for `wasm32-wasip2`.

- In general search: a query that looks like an expression (`2+2*3`, `15% of 80`) yields an item with the result; Enter copies the result.
- The "Calculator" command opens a `scratchpad`: a Numi-style multiline editor: each line is an expression with the result on the right; variables (`x = 5`, then `x * 2`), reference to the previous result, percentages, basic units (length, mass, time); the set of units is up to the builder.
- Never loses calculations: the content is saved to the plugin's `storage` on change and restored on open; history of past sheets is a list from which an old one can be opened.
- Currencies, later, via `http` (not required in this spec).

## Performance budgets (acceptance criteria)

- Warm window open (hotkey → first frame): target ~50 ms on every platform; the main criterion is subjectively "instant, like typing a character". Measure automatically (hotkey receipt timestamp → presented frame) and keep it in logs/a debug line.
  - Course of action (author's call; the user said "aim for roughly 50 ms" and did not decide on a fallback plan): (1) the plan starts with a time-boxed experiment on Linux and macOS (and briefly on Windows), before the main UI work. Verify: the "hotkey → presented frame" measurement in the warm and first-after-start states; renderer warm-up; transparency and rounded corners without a backing; on macOS, a non-activating panel that does not steal focus from the current app; on Windows, a tool window with no taskbar icon; a global hotkey on each OS. None of the macOS/Windows items have been run yet; (2) if it cannot be done out of the box, maintain our own patch/fork of the GPUI platform backend; (3) if that does not work either, show the user the measurements and discuss; consider switching toolkits only after that. Hence the `tomakast-ui` boundary must be narrow: the core does not know about GPUI.
- Keystroke → updated results on 10k static items: ≤ 16 ms (dynamic sources may render in later).
- In the background with the window hidden: ~0% CPU, no redraws.
- RSS with the window hidden: target ≤ 200 MB (the spike showed ~185 MB after the first show due to the GPU context; reducing it is not a blocker, but measure it and do not make it worse).
- A hung plugin does not affect UI responsiveness.

## Decided by the user

- Cross-platform Linux/macOS/Windows, no web wrappers, GPU rendering.
- Search is part of the core; sources are modules/plugins.
- Anyone can write plugins, they load without rebuilding; a plugin manager comes later, as a separate module.
- Plugins in WASM (any language).
- UI model A (fixed views).
- GPUI toolkit.
- First spec: core + plugin API + `apps` + `calc` (proposed, not challenged).
- Linux and macOS are first-class platforms, Windows is the second priority, "but also important"; a Mac and a Windows machine for testing are available.
- Opening the window must feel instant, target ~50 ms.
- Plugins are considered potentially malicious.

## Spec author's calls (open to challenge)

- Windows in this spec: everything must work and be verified, but it may be finished after Linux and macOS (this is how "second priority, but also important" was understood).
- Plugin execution model: short handlers + asynchronous host operations + `on-event`; no async exports in 0.1.
- Only WIT 0.1 and the manifest are frozen; the internal module trait evolves.
- Course of action if the open budget is not met (experiment → backend fork → discussion → toolkit switch).
- Packaging and signing are outside this spec.

- Built-in modules are Rust in the binary; third-party ones are WASM; a shared internal trait.
- wasmtime in-process, no separate processes for plugins.
- Views are updated in full, without diffs, in v1.
- SQLite for data, TOML for settings.
- On Wayland: the GlobalShortcuts portal + CLI fallback.
- GNOME/Wayland is not a target platform in v1.
- Plugin installation by copying a folder, hot reload via file watcher.

## Left to the builder

- The exact crate layout and type names.
- The exact WIT signatures within the described structure.
- Ranking formulas and coefficients.
- How to hide the layer-shell window without recreating it (including a fork/patch of the GPUI backend); the criterion is the open budget.
- The fuzzy algorithm (custom or a crate like `nucleo`).
- The calculator's unit set and syntax within the described scope.
- Ordering within priorities (Linux and macOS together first, Windows next), with CI builds and tests on all three from the very start.
