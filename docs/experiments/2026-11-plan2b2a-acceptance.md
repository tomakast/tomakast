# Plan 2b-2a acceptance (Task 10)

Date: 2026-11-19. Branch `plan-2b2a-plugin-host`. Master baseline: `ba66171` (the plan's base).
Machine: the plan 1 setup (KDE Plasma 6.8.3, Wayland, 3840×2160 at 240 Hz, scale 1.5, NVIDIA RTX 5070 Ti on Vulkan, 16 threads, kernel 7.3.5-1-cachyos).

Four parts: [agent measurements](#agent-measurements), [threat-model suite](#threat-model-suite), [rulings](#rulings) and the [user checklist](#user-checklist) (result cells blank), then [known limitations](#known-limitations).

## Agent measurements

Measured at `9fbb7d5`, before the final review fixes (`ce8615a..`); not repeated after them.

Protocol: plan 2b-1's (`docs/experiments/2026-11-plan2b1-acceptance.md`). Release build (`cargo build --release -p tomakast-app`). Each run used a fresh `TOMAKAST_HOME` under `$XDG_RUNTIME_DIR`, with `TOMAKAST_NO_KEYBOARD=1`, `NO_COLOR=1` and `RUST_LOG=info,notify=warn,tomakast_ui=debug`; real app directories were scanned read-only. The steps:

1. Start `tomakast daemon &`, wait for the socket plus 13 s, and sample RSS with the window hidden.
2. Count CPU ticks and per-thread wake-ups over 10 s hidden.
3. Run 500 `tomakast toggle` calls (250 shows) with a 50 ms sleep, then sample RSS hidden again.
4. Count idle CPU and wake-ups over another 10 s hidden.

Every client call carried the same `TOMAKAST_HOME`, so it reached only that daemon. Master `ba66171` and the branch were run back to back: master 4 times, the branch with kitchen-sink 5 times (2 cold, 3 warm cache), and the branch without a plugin twice. The no-plugin runs separate the cost of the branch binary from the cost of a loaded plugin. 0 toggle failures in every run. In every run the only log line matching `error|panick` was the known plan 1 noise `ERROR freedesktop_desktop_entry::exec: Unknown field code: '%%F'`.

**Installing and approving kitchen-sink** (every branch run with the plugin), before the daemon starts:

```bash
mkdir -p "$TOMAKAST_HOME/plugins/kitchen-sink"
cp -r plugins/kitchen-sink/plugin.toml plugins/kitchen-sink/icons "$TOMAKAST_HOME/plugins/kitchen-sink/"
cp target/plugins/wasm32-wasip2/release/kitchen_sink.wasm "$TOMAKAST_HOME/plugins/kitchen-sink/plugin.wasm"
cargo build --release -p tomakast-plugin-host --example approve_plugin
target/release/examples/approve_plugin "$TOMAKAST_HOME/tomakast.db" "$TOMAKAST_HOME/plugins/kitchen-sink"
# allowed kitchen-sink (0.1.0), content 9c7d25fcccbd5538…
```

`approve_plugin` reads the folder with `Package::read` and requires the gate to answer `Review`. It then writes the grant the review view's "Allow" writes: the reviewed content hash and permissions, decision `Allow`, through `PluginDb::set_grant`. `tomakast run plugins/review-kitchen-sink` cannot do this, because it only opens the review view and Allow is a view action.

**The plugin was loaded in every branch run with the plugin.** Three checks showed it:

- the log has `plugin loaded plugin=kitchen-sink version=0.1.0`;
- `<home>/cache/plugins/` holds one `<wasm sha256>-<engine hash>.cwasm`;
- `tomakast run kitchen-sink/hello` produced the plugin's `hello` toast (`toast with no window title=hello` while hidden).

### Results

| Run | Plugin load | RSS hidden, after start | RSS hidden, after 250 shows | Warm open median (p95, max), n = 249 | First open | Idle CPU, 10 s ×2 |
|---|---|---|---|---|---|---|
| master 1 | — | 188.5 MiB | 221.0 MiB | 22.2 ms (22.6, 27.6) | 38.6 ms | 0 / 0 ticks |
| master 2 | — | 189.0 MiB | 221.4 MiB | 21.8 ms (22.5, 24.9) | 38.8 ms | 0 / 0 ticks |
| master 3 | — | 189.1 MiB | 221.1 MiB | 21.9 ms (22.5, 24.5) | 38.0 ms | 0 / 0 ticks |
| master 4 | — | 188.5 MiB | 223.9 MiB | 21.9 ms (22.1, 25.8) | 36.5 ms | 0 / 0 ticks |
| branch 1 (cold: cache miss) | 41.9 ms | 199.4 MiB | 230.9 MiB | 22.3 ms (23.1, 24.6) | 41.0 ms | 0 / 1 ticks |
| branch 2 (warm: cache hit) | 1.6 ms | 199.3 MiB | 235.4 MiB | 22.3 ms (23.2, 25.6) | 38.1 ms | 0 / 0 ticks |
| branch 3 (cold) | 44.0 ms | 200.0 MiB | 231.3 MiB | 22.1 ms (22.6, 24.2) | 38.2 ms | 0 / 0 ticks |
| branch 4 (warm) | 1.6 ms | 199.4 MiB | 230.6 MiB | 22.2 ms (23.1, 26.2) | 38.4 ms | 0 / 0 ticks |
| branch 5 (warm) | 1.5 ms | 199.2 MiB | 230.6 MiB | 22.0 ms (22.7, 25.1) | 37.7 ms | 0 / 0 ticks |
| branch, no plugin, 1 | — | 195.3 MiB | 227.2 MiB | 22.0 ms (22.5, 26.7) | 38.5 ms | 0 / 0 ticks |
| branch, no plugin, 2 | — | 195.3 MiB | 227.1 MiB | 22.5 ms (22.7, 23.9) | 37.2 ms | 0 / 0 ticks |

The warm runs reuse a home whose cache already holds the plugin. Branch 2 reused branch 1's home; branches 4 and 5 used a home prepared by one more cold start (52.8 ms load, no toggles). Branch runs 1–3 used the kitchen-sink build before its env probe changed to report a count; runs 4–5 and the dry run used the final build. Only probe code differs between the two.

Warm open is `launcher show request -> on_next_frame`, without the first open. "Plugin load" is the manager's `plugin loaded … elapsed`: read and hash the folder, run the gate, then compile in the child process (cold) or deserialize from the cache (warm), and instantiate.

### Against the brief's bounds

- **Warm open ≤ 22.7 ms: met.** Branch 22.0–22.3 ms, master 21.8–22.2 ms in the same back-to-back session: about +0.2 ms, within run-to-run noise. Both sit about 1.3 ms above plan 2b-1's 20.7 ms. Master shows the same shift, so it comes from the machine's state that day, not from this plan.
- **CPU over 10 s hidden, plugin loaded, no timers: 0 ticks in 11 of 12 windows; one window had 1 tick (10 ms).** A 60 s window hidden with the plugin loaded gives 0 ticks; the same 60 s on master gives 1 tick. A stray single tick is baseline noise, not the plugin host.
- **No plugin-host thread wakes.** Per-thread context switches (`/proc/<pid>/task/*/status`) over every idle window: `plugin-epoch`, `plugin-timers`, `plugin-events`, `plugin-manager`, `module-kitchen-` and `module-plugins` gained 0 switches. No `plugin-http` thread exists: the pool spawns workers per request and they exit when idle. The only threads that wake are the same on master: `blocking-1` and `blocking-2` at 2/s, the main `tomakast` thread at 0.5/s, and the Vulkan `[vkps] Update` thread at 4/s.
- **RSS, "the plugin adds at most +10 MiB": slightly over the +10 MiB bound against master.** After start the branch with the kitchen-sink is +10.7 MiB over master (mean). After 250 shows the median is +9.7 MiB, but 2 of 5 runs exceed +10 MiB (+14.2 and +10.1). The loaded plugin alone, measured against the branch without a plugin, adds +4.2 MiB after start and +3.8 MiB after 250 shows (median).

  | | after start | after 250 shows |
  |---|---|---|
  | master (4 runs) | mean 188.8 MiB | median 221.25 MiB (221.0–223.9) |
  | branch with kitchen-sink (5 runs) | mean 199.5 MiB (+10.7) | median 230.9 MiB (**+9.7**; runs +9.7 / +14.2 / +10.1 / +9.3 / +9.3) |
  | branch without a plugin (2 runs) | 195.3 MiB (+6.5) | 227.15 MiB (+5.9) |
  | the loaded plugin alone (with − without) | +4.2 MiB | +3.8 MiB (median) |

  Cold-cache and warm-cache runs end at the same RSS: 230.9 and 231.3 MiB cold, 230.6 and 230.6 MiB warm. The one high run (235.4 MiB, warm) is an outlier among five. Master has one too (223.9 MiB among 221.0–221.4). Its cause was not investigated.

  About 6 MiB comes with the branch binary even when no plugin is installed. The likely contributors are the larger binary (wasmtime, cranelift and WASI code paged in), the idle plugin-host threads, and the Task 1–3 core and UI changes; none was measured separately. The loaded kitchen-sink adds about 4 MiB on top: engine, compiled component, a store with its linear memory, and the timer thread.
- **Cold load** (cache miss: the compile child runs) **41.9 / 44.0 / 52.8 ms; warm load** (cache hit: deserialize from `<data-dir>/cache/plugins/`) **1.5–1.6 ms.** The kitchen-sink component is about 270 KB.

### wasmtime features and binary size

- `wasmtime` 50.0.2, `default-features = false`, features `runtime, std, anyhow, component-model, cranelift, parallel-compilation`. `wasmtime-wasi` 50.0.2, `default-features = false`, features `["p2"]`.
- Effective set (`cargo tree -e features -i wasmtime`): the above plus `async` (forced by `wasmtime-wasi/p2`, Ruling T), `wasmtime-jit-icache-coherence` and `wasmtime-unwinder`. tokio comes in through `wasmtime-wasi` (`rt-multi-thread, net, time, io-util, io-std, sync`). The host uses only the sync bindings.
- Stripped release `tomakast` (`strip` on copies of `target/release/tomakast`; master built from `git archive ba66171` with its own target dir):

  | | stripped size |
  |---|---|
  | master `ba66171` | 50 483 568 B (48.14 MiB) |
  | this branch | 70 800 752 B (67.52 MiB) |
  | delta | **+20 317 184 B (+19.38 MiB)** |

  Task 5 measured +12.45 MiB as a lower bound, before instantiation, WASI and deserialization were reachable. The full plugin host costs +19.4 MiB stripped.

## Threat-model suite

`crates/plugin-host/tests/threat_model.rs` drives the kitchen-sink plugin through a core, as the daemon does. Each test asserts the denial and that the host keeps running: the plugin's `hello` command still answers afterwards, or only the plugin stops. The probes are new `run` actions of kitchen-sink (`wasi-fs`, `wasi-env`, `wasi-tcp:<port>`, `wasi-stdout:<text>`, `markdown`, `bad-ids:<target>`). Each one answers what the guest saw as detail-view metadata.

| Test | What the plugin tries | What it gets |
|---|---|---|
| `wasi_has_no_filesystem` | read `/etc/passwd`, list `.` and `/`, write `./probe.txt` | `Err(No such file or directory)` for all four. There are no preopens. |
| `wasi_has_no_env_or_args` | read the environment, the arguments and the working directory | `[]`, `[]` and `/`, while the test process has both an environment and arguments |
| `wasi_has_no_sockets` | TCP connect to a real listener on `127.0.0.1`, look up `localhost` | `Err(Permission denied)` and a lookup error. The listener never sees a connection. |
| `wasi_stdout_is_not_inherited` | write a unique canary to stdout and stderr | The writes succeed into a sink. The test binary re-runs itself as a child with its output captured, and the canary is in neither stream. |
| `plugin_file_icon_cannot_escape_folder` | `../x.png`, `icons/../../x.png` or `/etc/passwd` as a `plugin-file` icon; a symlink inside the folder to a real file outside it, directly or through a linked folder | The name checks reject the whole answer with a denial toast. The symlinks pass the name check, are caught after canonicalization (Ruling U), and the item is kept without an icon. |
| `markdown_from_plugin_has_no_html_and_no_remote_images` | a detail view with `<script>`, `<iframe>`, `<img src=…>` and `![pixel](https://…)` | The host passes the text on as data. The launcher's own parser (`tomakast_ui::markdown::parse`) shows every tag as literal text and the image as `alt (url)`, and no span links anywhere. |
| `plugin_cannot_set_item_source_or_kind` | — (the WIT `item` record has no such fields; the test checks the record's field list) | A user-started and a timer-started (unsolicited) `replace-items` both get kind `Command` and source = manifest name. Init and search are covered by `init_commands_are_indexed_with_source` and `search_roundtrip`. |
| `effects_with_control_chars_in_ids_rejected` | ESC, newline, BEL, NUL, CR and U+202E in view, row, action, field, tile and item ids | `run` fails with the reason, escaped onto one line (Ruling W). An `on-event` answer is rejected as a whole with a denial toast. |
| `search_needs_the_search_permission` (final review C-1) | answer `search` without the `search` permission | `search` is never called, so the plugin never sees the query; with the permission the same query is answered. |
| `a_hung_plugin_does_not_delay_other_modules` | spin in `search` until the 100 ms deadline | A native `FakeModule` in the same core answers in under 20 ms, during the spin and again after the plugin is stopped. |

Earlier tasks already test the other denials: deadlines, memory, traps, blocking WASI calls, quotas, permissions, approval and the markdown renderer itself. The module doc of `threat_model.rs` names those tests.

## Rulings

Every ruling made while executing the plan (the SDD ledger), with what it costs if it turns out wrong.

| Ruling | Decision | Cost if wrong |
|---|---|---|
| A | Task 2 keeps the workspace compiling: the UI gets minimal arms for the new types (blank emoji/colour icons, shortcut/error/source ignored, `Closed` never sent); Task 3 renders and sends them. | none |
| B | Each dependency is added by the task that first uses it (wasmtime and wasmtime-wasi in Task 5, ureq in Task 7); the wasmtime feature set and size are recorded from Task 5 on. | none |
| C | Plugin preferences reach `WasmPlugin` as a `BTreeMap<String, String>`; `Settings::plugin_preferences` parses the `[plugins.<id>]` tables of `config.toml` (bools as `"true"`/`"false"`). | none |
| D | `tomakast_platform::open::{open_url, open_path}` (Linux `xdg-open`, macOS `open`, Windows `ShellExecuteW`), called after the host's own scheme/path checks; the apps module keeps its own reveal code. | small |
| E | The plugins module exposes `PluginsHandle::set_reviews(Vec<ReviewState>)`, which the manager calls. | none |
| F | `Item.source` is host-only, not in the WIT item. | none |
| G | An invalid shortcut, emoji or colour rejects the whole effect with an error toast, like an oversized view. | small |
| H | Manifest command items are host-owned: a plugin's `replace-items` replaces only its own items, never the commands. | small |
| I | `storage.get` returns `result<option<string>, string>` and `storage.remove` `result<_, string>`, so a missing permission or quota is reportable. | none (pre-freeze) |
| J | An action shortcut needs ctrl, alt or super and may not use Tab, Backspace, Delete, Home, End, arrows, Enter, Esc, Space or ctrl+{a,c,v,x,z,y,k}; violating it is invalid (Ruling G); a valid shortcut colliding with a launcher key is ignored with a warning. | small |
| K | A dynamic `search` answers at most 50 items; more rejects the answer (rate-limited denial toast). | one number |
| L | Aggregate and inner caps: one call's returned value ≤ 4 MiB; keywords ≤ 32 per item; accessories ≤ 8 per row; metadata ≤ 64 per detail; dropdown options ≤ 200; annotations ≤ 10 000; text-field values ≤ 64 KiB; multiline values ≤ 256 KiB. | numbers |
| M | Unicode format characters (Cf: bidi overrides, zero-width) are rejected in ids and in `emoji`; titles may contain them. | small |
| N | In `emoji` only, ZWJ (U+200D) and the tag characters U+E0020–U+E007F are allowed, for ZWJ sequences and subdivision flags. | none |
| O | The emoji cap is 32 bytes (the England flag is 28), and an emoji must contain a character that is not Cf, ZWJ or a tag. | one number |
| P | In grid tiles, emoji and colour swatches fill the tile's image slot; list rows use the icon size. | one argument |
| Q | Allowlist domains must be ASCII (punycode for IDNs); no IDNA dependency for the manifest. | authors write `xn--` forms |
| R | The manifest caps commands at 64 and preferences at 32; commands of an unapproved plugin are never indexed. | numbers |
| S | A one-line error-message fix is verified by the controller directly (grep), without a separate re-review. | none |
| T | `wasmtime/async` and tokio, pulled in by `wasmtime-wasi`'s p2, are accepted as long as only sync bindings are used. | binary size |
| U | `plugin-file` icons are resolved once at load (implemented per answer, when the effect or search result arrives): the canonical path must stay under the canonical plugin folder, with no symlink component, and be a regular file of at most 1 MiB; otherwise the icon becomes none (with a warning), not a rejected effect. | small |
| V | Remaining caps: toast message ≤ 4 KiB; metadata value ≤ 4 KiB; annotation text ≤ 1 KiB; keyword ≤ 1 KiB; icon names, dropdown values and `plugin-file` paths follow the id rule; dates are empty or `YYYY-MM-DD`; list query ≤ 64 KiB; grid columns 1–12 (0 means 1); annotation lines < 10 000 and inside the text. | numbers |
| W | A rejected `run` answer stays the call's error, shown every time; only init, search and on-event rejections go through the shared, rate-limited denial toast. | none |
| X | `copy-text` and the `open-*` imports need a user action: they work only during a call the user started (`run` or a view event); otherwise copy-text is dropped and `open-*` fails with "needs a user action". | plugins cannot write the clipboard in the background (intended) |
| Y | When a plugin stops or unloads, its queued HTTP requests are dropped and the results of in-flight ones are discarded. | none |
| Z | For name-based allowlist entries the host refuses loopback, private (RFC 1918 / ULA), link-local, CGNAT and unspecified addresses after resolution; only an IP-literal or `localhost` entry may reach local addresses. | small |
| AA | `open-path` refuses the union of the per-OS refused extensions on every OS, plus jar, appimage, webloc, inetloc, fileloc, url, desktop, flatpakref, deb, rpm, sh, py, pl, rb, command, tool, scpt, app, pkg and dmg; a refusal returns one generic error (no filesystem oracle). | some harmless files cannot be opened by plugins |
| AB | Unsolicited `push-view` counts in the same 30/s window as unsolicited `update-view`; the UI caps the view stack at 32 entries. | numbers |
| AC | `mailto` URLs keep only the to, cc, bcc, subject and body parameters. | none |
| AD | Unsolicited `replace-items` is limited to 1 per 5 s per plugin. | one number |
| AE | No accidental approval: the review view's primary (Enter) action is "Not now"; "Allow" and "Deny" are in the action panel; "Allow" chosen less than 1 s after the view was shown is ignored. | one extra keypress |
| AF | Re-reviewing a denied plugin is out of scope (a later plugin-manager feature); deleting or changing the folder re-prompts. | none |
| AG | Removing a plugin folder deletes its grant, so a re-created folder always re-prompts; an in-place update follows the subset rule. | a re-prompt after a reinstall |
| AH | The link policy takes an explicit set of native module ids from the daemon; `RESERVED_IDS` stays a manifest rule only. | none |
| AI | Only `NotFound` on a plugin folder counts as removal (unregister and forget the grant); any other read error unloads and keeps the grant; a vanished plugins folder never wipes grants. | none |
| AJ | Seeing what is typed is the permission `search = true` under `[permissions]` ("See what you type in the launcher search"), part of the subset rule, the prompt and the grant; the top-level `dynamic` key is removed. An in-place update that changes `name` or `author` asks again (grants record both; schema v3). | an extra prompt |
| AK | Background budget: timer and HTTP `on-event` calls get at most 1 s of guest time per 10 s per plugin; over budget the plugin's timers are suspended for 60 s (logged); the third suspension within 10 minutes stops the plugin (broken). The WIT says the host may delay, coalesce or suspend timers. | numbers |
| AL | A user action is only `run`, `action-chosen` and `form-submitted`; `closed`, `query-changed` and `text-changed` are not. One call opens at most once. The WIT says so in all three places. | none |
| AM | The HTTP pool is fair: per-plugin queues served round-robin; a request's 30 s still start at `fetch`. | none |
| AN | A plugin's views show "<Plugin name> · <view title>" in the footer, and its toasts are prefixed "<Plugin name>: ". | cosmetic |
| AO | `storage.set` and `storage.remove` together at most 20 per second per plugin (an error beyond), stated in the WIT. | one number |
| AP | The manifest command `mode` key is removed (it had no effect). | re-adding a key later is additive |
| AQ | `scratchpad-view` gets `actions: list<action>` (row id = view id, like a detail view), with action-panel support in the UI. | an unused field |
| AR | Removing a plugin folder while the daemon runs also deletes its `plugin_kv` rows; the startup scan never deletes grants or data (it narrows AG: a folder removed while the daemon was down keeps its grant). | data loss on uninstall (intended) |
| AS | The HTTP pool (8 workers, round-robin per plugin) can still be held entirely by two plugins that each keep 4 slow requests in flight; accepted for v1 (few plugins) and recorded as a follow-up (per-plugin worker cap, or a pool sized to the active plugins). | a third plugin's request can time out unsent under abuse |

## User checklist

Run from the worktree root. If your shell is fish, run `bash` first. Everything uses a throwaway home, so no real config, database, plugins or autostart entry is touched and **no backup is needed**. The keyboard is grabbed normally (`TOMAKAST_NO_KEYBOARD` is unset). `TOMAKAST_DEMO=1` adds the demo module for V1–V3.

```bash
cargo build --release -p tomakast-app
CARGO_TARGET_DIR=target/plugins cargo build --release --target wasm32-wasip2 \
  --manifest-path plugins/kitchen-sink/Cargo.toml
export TK="$PWD/target/release/tomakast"
export TOMAKAST_HOME=$(mktemp -d -p "$XDG_RUNTIME_DIR" tk-accept.XXXX)
unset TOMAKAST_NO_KEYBOARD
TOMAKAST_DEMO=1 RUST_LOG=info,tomakast_ui=debug "$TK" daemon 2> "$TOMAKAST_HOME/daemon.log" &
DAEMON=$!

# The plugin folder: plugin.toml, icons/ and the built component as plugin.wasm.
STAGE=$(mktemp -d)/kitchen-sink
mkdir -p "$STAGE"
cp -r plugins/kitchen-sink/plugin.toml plugins/kitchen-sink/icons "$STAGE/"
cp target/plugins/wasm32-wasip2/release/kitchen_sink.wasm "$STAGE/plugin.wasm"
```

At `9fbb7d5`, before the final review fixes, the agent dry-ran P1, P4, P5 and P7 headlessly (`TOMAKAST_NO_KEYBOARD=1`, the same commands, with `approve_plugin` and a daemon restart in place of the Allow keypress). An item's presence was read from `tomakast run <id>`, which logs `unknown item` only when the item is missing. Results: after a hot install into a running daemon the review item exists and `kitchen-sink/hello` does not; after Allow the plugin loads and the review item is gone; the appended `http` line parses, unloads the plugin and brings the review back; deleting the folder logged `plugin folder removed; its approval is forgotten` (now `…its approval and data are deleted`, Ruling AR) and removed the review; a re-copy brings the review back. The rows below are for what only a person can judge: the text, the keys, the toasts and the rendering.

Open the launcher with `"$TK" toggle` (or the hotkey). Do the rows in order: Ruling AF means a denied plugin is asked about again only after its folder is deleted or changed.

| # | Check | How | Expected | Result |
|---|---|---|---|---|
| P1 | Install by copying | `mkdir -p "$TOMAKAST_HOME/plugins" && cp -r "$STAGE" "$TOMAKAST_HOME/plugins/"`; wait a second; open the launcher with an empty query | Nothing pops up by itself (no window, no toast). The first Suggestion is "Review plugin: Kitchen Sink", subtitle "New plugin needs your approval". Typing `say hello` finds no plugin command yet. | |
| P2 | Review, Not now | Enter on the review item; read the view; press Enter | The view lists name, id, version, folder, "It asks to:" with storage, index, clipboard, open, timers and "See what you type in the launcher search", and a content hash prefix. Enter is "Not now": the view closes, no toast, and the review item is still there. | |
| P3 | Allow too fast | Enter on the review item, then at once Ctrl+K → Allow | Error toast "Take a moment to read the permissions"; nothing is approved. | |
| P4 | Allow | Wait a second in the view, Ctrl+K → Allow | Success toast "Plugin allowed" (Kitchen Sink); the view closes and the review item is gone. Type `say hello`, Enter: a toast "Kitchen Sink: hello" (Ruling AN). The log has `plugin loaded plugin=kitchen-sink`. | |
| P5 | Update with a new permission asks again | `printf 'http = ["example.com"]\n' >> "$TOMAKAST_HOME/plugins/kitchen-sink/plugin.toml"`; open the launcher | "Say hello" is gone (the old version was unloaded). The review item is back with subtitle "Plugin update asks for more permissions", and the view now also lists "Make web requests to example.com". | |
| P5b | A renamed update asks again | Allow (P4); then `sed -i 's/^name = .*/name = "Sink"/' "$TOMAKAST_HOME/plugins/kitchen-sink/plugin.toml"`; open the launcher | The review item is back with subtitle "Plugin update changed its name or author" (Ruling AJ). Deny it for P6, or Allow and continue. | |
| P6 | Deny | Enter on the review item, Ctrl+K → Deny | Success toast "Plugin denied"; the review item is gone and stays gone; `say hello` finds nothing. Restarting the daemon does not ask again. | |
| P7 | Delete, then reinstall re-prompts | `rm -rf "$TOMAKAST_HOME/plugins/kitchen-sink"`; wait a second; then `cp -r "$STAGE" "$TOMAKAST_HOME/plugins/"` | After the delete the log has `plugin folder removed; its approval and data are deleted` (Ruling AR). After the copy the review item is back as a new plugin (Ruling AG). Allow it again (P4) for P8. | |
| P8 | Plugin source and shortcut chip | Type `echo:hi` | A dynamic row "hi" with "Kitchen Sink" as its source on the right (its `plugin-file` icon is a 1×1 PNG, so effectively blank). Ctrl+K shows "Copy" with a `Ctrl+Shift+C` shortcut chip. | |
| P9 | Plugin name on its views | Type `indexed by init`, Enter (its first action, Echo) | The detail view's footer reads "Kitchen Sink · echo" (Ruling AN); the header shows only "echo". | |
| V1 | Emoji and colour icons (left from Task 3) | Enter on "Demo: Grid" | Tiles 5–8 show emoji glyphs and tiles 9–12 colour swatches, each filling the tile's image area (Ruling P); nothing is clipped. | |
| V2 | Form error (left from Task 3) | Enter on "Demo: Form" | The Handle field shows "This handle is taken" under it in the error colour. | |
| V3 | Shortcut chips (left from Task 3) | Enter on "Demo: List", Ctrl+K on a row | "Copy Name" shows a `Ctrl+Shift+C` chip; pressing Ctrl+Shift+C on a row (panel closed) runs it. | |
| V4 | Scratchpad actions (Ruling AQ) | Enter on "Demo: Scratchpad"; press Enter in the text; then Ctrl+K | Enter types a newline. The footer shows no "⏎" action. Ctrl+K lists "Copy total length" and "Clear" (with a `Ctrl+Shift+L` chip); choosing one shows an "Action chosen" toast naming `demo/scratchpad`. | |

Stop the daemon:

```bash
kill "$DAEMON"; rm -rf "$TOMAKAST_HOME" "$(dirname "$STAGE")"; unset TOMAKAST_HOME
```

## Known limitations

- **No proxy for plugin HTTP.** Plugin requests ignore system proxies (`HTTP_PROXY`, …): Ruling Z needs the host to resolve names itself to refuse local addresses. A user behind a mandatory proxy gets connection errors from plugins.
- **macOS and Windows `open` code is not compiled here.** `tomakast_platform::open` for macOS (`open`) and Windows (`ShellExecuteW`) is cfg-gated and was never built on this Linux machine. The first build on those platforms may need fixes.
- **HTTPS is untested without network.** The HTTP tests run against a local plain-HTTP server (IP-literal allowlist entry); TLS handshakes, certificate errors and real redirects to HTTPS hosts were not exercised.
- Re-reviewing a denied plugin needs its folder deleted or changed (Ruling AF).
- **HTTP fairness has a floor.** The pool serves plugins round-robin (Ruling AM), but it has 8 workers and a plugin may run 4 requests. Two plugins that keep 4 slow requests each running hold every worker, so a third plugin's request waits for one of them to finish, up to 30 s. Its 30 s count from `fetch`, so it can time out unsent.
- **A folder removed while the daemon is down keeps its grant (Ruling AR).** A plugin with the same id copied in later runs without a prompt if its permissions are no larger and its name and author are the same (Ruling AJ).
