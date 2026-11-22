# Item Hotkeys Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Assign a global hotkey to any indexed item from the launcher's Ctrl+K panel, bind it live (KDE through kglobalaccel, X11, macOS, Windows; other Wayland desktops after restart through the portal), and write it to `config.toml` only once the system accepted it.

> **Revised 2026-11-20 after Task 1:** KDE uses kglobalaccel over D-Bus instead of a session-per-change portal (spec "Revision"). Task 5b is replaced; Tasks 2, 3, 6, 7 and 10 carry small changes marked *(rev)*.

**Architecture:** Pure rules, the key table and the comment-preserving config editor live in `tomakast-core`. `tomakast-platform` gets a long-lived `HotkeyService` with two independently bound sets (launcher, items) and four backends: kglobalaccel (KDE, Wayland and X11), the GlobalShortcuts portal bound once at start (other Wayland desktops), `global-hotkey` (X11, macOS, Windows), and a null one. The daemon (`tomakast-app`) owns a state machine that applies sets, holds one pending change, and writes the file on acceptance; the UI (`tomakast-ui`) shows chips on rows, a panel entry and a recorder, and talks to the daemon through a trait set on `LauncherController`.

**Tech Stack:** Rust 1.98, GPUI via gpui-kit =0.10.0, ashpd 0.13 (portal), global-hotkey 0.10, toml_edit 0.25, notify 8.

**Spec:** `docs/superpowers/specs/2026-11-20-item-hotkeys-design.md`

## Global Constraints

- The window opens in ~50 ms: nothing in this plan runs on the open path.
- UI strings are English.
- `TOMAKAST_NO_KEYBOARD=1` never binds real shortcuts (null service); it is the headless test mode.
- `tomakast-platform` must not depend on GPUI.
- The file is written only after the system accepts (kglobalaccel or native registration succeeded; immediately with the null service and on portal desktops, where it applies after restart).
- A refused or cancelled rebind never costs a hotkey that worked before (the row's own, another row's, the launcher's).
- The launcher set and the item set are bound independently; an item rebind never touches the launcher's.
- Conflicts with another row are refused, never reassigned.
- Only indexed items are eligible (not dynamic answers, not rows of pushed views).
- A hotkey needs Ctrl, Alt or Super; Shift alone does not count.
- Keep edits inside hotkey, settings, daemon, platform, action-panel and root-row code: another session edits widget code in parallel.
- Commits: no attribution lines; never commit screenshots. `just check` must pass at the end.
- Never start a keyboard-grabbing launcher on the user's desktop yourself; headless runs use another `TOMAKAST_HOME` with `TOMAKAST_NO_KEYBOARD=1`.

## Review Focus

1. **Shifted digits and symbols.** GPUI drops Shift for symbol keys (`ctrl-shift-1` arrives as key `!` without shift). Expected: the recorder records `Ctrl+Shift+1`, not `Ctrl+!`. Pinned in Task 2 (`unshifted`) and Task 8 (`recorder_records_shifted_digit_as_shift_and_digit`).
2. **Non-Latin layout.** Under a Russian layout GPUI maps Ctrl+letters back to the Latin key (`guess_ascii` in `third_party/gpui-pre-linux/src/linux/platform.rs`); a key that is still non-ASCII must be refused with a reason, not recorded. Pinned in Task 8 (`recorder_refuses_a_non_latin_key`).
3. **Hand-edited duplicates and spellings.** `"alt+ctrl+T" = "apps/kitty"` plus `"Ctrl+Alt+K" = "apps/kitty"`: changing Kitty's hotkey removes both and keeps comments. Pinned in Task 4.
4. **config.toml broken while the launcher is open.** Enter is refused ("config.toml has an error"), nothing is written, the bound sets stay. Pinned in Task 6 (`a_broken_file_refuses_the_change_and_keeps_the_sets`).
5. **A second Enter while a change is being applied**, or a hand edit landing while a change is pending. Expected: refused with "Waiting for the last change."; a conflicting hand edit makes the accepted change skip the write and re-bind the previous set. Pinned in Task 6.

---

### Task 1: KDE portal probe (gate)

Answers the spec's "Step 1" questions on this desktop before the portal backend is built. Needs the user: it shows KDE dialogs on the real session.

**Files:**
- Create: `crates/platform/examples/portal_probe.rs`
- Modify: `justfile` (recipe `hotkey-probe`)
- Create: `docs/experiments/2026-11-item-hotkeys-portal-probe.md`

**Interfaces:** none (throwaway example; not used by later tasks).

- [ ] **Step 1: Write the probe.** Linux-only body (`#[cfg(target_os = "linux")]`; `fn main() {}` elsewhere so `clippy --all-targets` passes on macOS/Windows CI). Host app id `dev.tomakast.HotkeyProbe` (never the real `dev.tomakast.Tomakast`). Combinations: `CTRL+ALT+SHIFT+F9`, `CTRL+ALT+SHIFT+F10`, `CTRL+ALT+SHIFT+F11`, and one lowercase-letter keysym `CTRL+ALT+SHIFT+j`. Interactive phases, each waiting for Enter on stdin and printing what to do in the dialog, every response and `Activated`/`ShortcutsChanged` signal with its session handle:
  1. Session A binds `probe-a@F9`; user presses "OK". Print the response's shortcuts (`id`, `trigger_description`).
  2. Session B binds `probe-b@F10` while A stays; user presses "OK". Then asks the user to press F9 and F10 combos: are both delivered (two live sessions)?
  3. Session C binds `{probe-b@F10, probe-c@F11}` (one new id); does the dialog list only F11? User presses "Cancel": print the result; ask to press F10: does session B still deliver?
  4. Session D binds `{probe-c2@j}` replacing B (drop B after D is accepted); does `probe-b@F10` stay in `kglobalshortcutsrc` holding F10? Print `grep -A5 dev.tomakast.HotkeyProbe ~/.config/kglobalshortcutsrc`.
  5. Ask the user to change `probe-a`'s key in KDE's dialog / System Settings; print any `ShortcutsChanged`.
  Write everything also to `$XDG_RUNTIME_DIR/tk-hotkey-probe.txt`.
- [ ] **Step 2: Add `just hotkey-probe`** that builds and runs `cargo run -p tomakast-platform --example portal_probe`.
- [ ] **Step 3: Build only.** Run: `cargo build -p tomakast-platform --example portal_probe` — Expected: builds. `cargo clippy -p tomakast-platform --all-targets --target x86_64-pc-windows-gnu -- -D warnings` — Expected: clean.
- [ ] **Step 4: Ask the user** (in Russian, plain words) to run `just hotkey-probe` from this worktree and follow its prompts; afterwards offer the cleanup command it prints (remove the `[dev.tomakast.HotkeyProbe]` section through System Settings › Shortcuts).
- [ ] **Step 5: Record the findings** in the experiments note (English): answers to spec questions 1–4, plus whether KDE accepted the lowercase-letter keysym and what `trigger_description` looks like.
- [ ] **Step 6: Gate.** If omitted ids keep holding their combination (phase 4) or two sessions cannot both deliver (phase 2), stop and bring the finding back to the user before Task 5b; the spec's fallbacks change what the user sees. Otherwise commit: `git add crates/platform/examples/portal_probe.rs justfile docs/experiments/2026-11-item-hotkeys-portal-probe.md && git commit -m "experiments: KDE GlobalShortcuts portal probe"`.

### Task 2: Key table

**Files:**
- Create: `crates/core/src/keys.rs`
- Modify: `crates/core/src/lib.rs` (pub mod), `crates/core/src/settings.rs` (`to_xdg_trigger` uses the table)
- Test: in `keys.rs` and `crates/core/tests/settings.rs`

**Interfaces:**
- Produces:
  - `pub fn is_supported(key: &str) -> bool` — config names: `a`–`z`, `0`–`9`, `f1`–`f12`, `space`, `enter`, `tab`, `backspace`, `delete`, `insert`, `home`, `end`, `pageup`, `pagedown`, `up`, `down`, `left`, `right`, `-`, `=`, `[`, `]`, `\`, `;`, `'`, `,`, `.`, `/`, `` ` ``.
  - `pub fn xkb_keysym(key: &str) -> Option<&'static str>` — e.g. `t`→`t`, `f1`→`F1`, `enter`→`Return`, `pageup`→`Prior`, `pagedown`→`Next`, `-`→`minus`, `[`→`bracketleft`, `` ` ``→`grave`, `'`→`apostrophe`.
  - *(rev)* `pub fn qt_key(key: &str) -> Option<u32>` — Qt key codes for kglobalaccel: letters → uppercase ASCII (`t`→`0x54`), digits and punctuation → ASCII, `f1`→`0x01000030` … `f12`→`0x0100003B`, `space`→`0x20`, `enter`→`0x01000004`, `tab`→`0x01000001`, `backspace`→`0x01000003`, `insert`→`0x01000006`, `delete`→`0x01000007`, `home`→`0x01000010`, `end`→`0x01000011`, `left`→`0x01000012`, `up`→`0x01000013`, `right`→`0x01000014`, `down`→`0x01000015`, `pageup`→`0x01000016`, `pagedown`→`0x01000017`; and `pub fn qt_code(hotkey: &Hotkey) -> Option<i32>` adding Shift `0x02000000`, Ctrl `0x04000000`, Alt `0x08000000`, Super (Meta) `0x10000000`.
  - `pub fn unshifted(symbol: &str) -> Option<&'static str>` — US layout: `!`→`1` … `)`→`0`, `_`→`-`, `+`→`=`, `{`→`[`, `}`→`]`, `|`→`\`, `:`→`;`, `"`→`'`, `<`→`,`, `>`→`.`, `?`→`/`, `~`→`` ` ``.
  - `pub const SUPPORTED_KEYS: &[&str]` (the list above, for other crates' table tests).
  - `Hotkey::to_xdg_trigger` now returns `None` for an unsupported key: signature changes to `pub fn to_xdg_trigger(&self) -> Option<String>`.

- [ ] **Step 1: Failing tests** in `keys.rs`:
  - `every_supported_key_has_a_keysym`: `SUPPORTED_KEYS.iter().all(|k| xkb_keysym(k).is_some())`.
  - `named_keys_map_to_xkb_names`: the pairs listed above.
  - `shifted_symbols_map_to_their_base_key`: `unshifted("!") == Some("1")`, `unshifted("?") == Some("/")`, `unshifted("a") == None`.
  - `non_latin_keys_are_not_supported`: `!is_supported("е")`, `!is_supported("escape")`.
  - *(rev)* `qt_codes_match_kde`: `qt_code(Ctrl+Alt+T) == Some(201326676)` (kitty's stored value) and `qt_code(Alt+F4) == Some(150994995)` (KWin's); `every_supported_key_has_a_qt_code`.
  - In `crates/core/tests/settings.rs`, `xdg_trigger_uses_keysyms`: `Hotkey::parse("Ctrl+Alt+Enter").unwrap().to_xdg_trigger() == Some("CTRL+ALT+Return".into())`, `"Super+F1"` → `Some("LOGO+F1")`.
- [ ] **Step 2: Run** `cargo test -p tomakast-core keys xdg_trigger` — Expected: FAIL (module missing).
- [ ] **Step 3: Implement** the table as one `&[(&str, &str)]` of (config name, keysym); letters and digits map to themselves. Update the one caller of `to_xdg_trigger` (`crates/platform/src/hotkeys.rs::portal_shortcuts`) to skip `None` with a `tracing::warn!`.
- [ ] **Step 4: Run** `cargo test -p tomakast-core && cargo test -p tomakast-platform` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): hotkey key table: supported keys, xkb keysyms, shifted symbols`.

### Task 3: Hotkey rules and the shared table

Moves the validation out of `crates/app/src/daemon.rs::hotkey_bindings` into core and adds the recorder's checks.

**Files:**
- Create: `crates/core/src/hotkeys.rs`
- Modify: `crates/core/src/lib.rs`, `crates/app/src/daemon.rs` (use the core function; delete the moved code and its tests after porting them)
- Test: in `hotkeys.rs`

**Interfaces:**
- Consumes: `keys::is_supported` (Task 2); `Settings { launcher_hotkey, hotkeys: Vec<(Hotkey, ItemId)> }`.
- Produces:
  ```rust
  pub enum Refusal { NoModifier, UnsupportedKey(String), LauncherHotkey, TakenBy(ItemId), Pending, BrokenConfig(String) }
  impl fmt::Display for Refusal // UI text, see Step 1
  /// The usable `[hotkeys]` in file order, and a log line for each dropped entry.
  pub fn item_hotkeys(settings: &Settings) -> (Vec<(ItemId, Hotkey)>, Vec<String>);
  /// Whether `item` may take `hotkey` given `assigned` (the usable item hotkeys) and the launcher's.
  pub fn check(assigned: &[(ItemId, Hotkey)], launcher: &Hotkey, item: &ItemId, hotkey: &Hotkey) -> Result<(), Refusal>;
  pub enum HotkeyState { Active { trigger: Option<String> }, AfterRestart, Refused { reason: String }, Off } // (rev) Waiting dropped
  pub struct ItemHotkey { pub item: ItemId, pub hotkey: Hotkey, pub state: HotkeyState }
  #[derive(Clone, Debug, Default, PartialEq)]
  pub struct HotkeyTable { pub launcher: Option<Hotkey>, pub items: Vec<ItemHotkey>, pub pending: bool, pub unavailable: Option<String> }
  impl HotkeyTable { pub fn of(&self, item: &ItemId) -> Option<&ItemHotkey>; pub fn check(&self, item: &ItemId, hotkey: &Hotkey) -> Result<(), Refusal>; }
  ```
  `HotkeyTable::check` uses the `Active`/`Refused`/`Off` entries other than `item` (every state counts as taken), returns `Refusal::Pending` when `pending`.

- [ ] **Step 1: Failing tests:**
  - Port `daemon.rs`'s two `hotkey_bindings` tests to `item_hotkeys` (same inputs; launcher excluded from the output now).
  - `check_refuses_without_ctrl_alt_or_super`: `shift+a` → `NoModifier`.
  - `check_refuses_the_launcher_hotkey`: `alt+space` with launcher `alt+space` → `LauncherHotkey`.
  - `check_refuses_another_items_hotkey`: assigned `[(konsole, ctrl+alt+t)]`, kitty asks `ctrl+alt+t` → `TakenBy(konsole)`.
  - `check_allows_the_items_own_hotkey`: konsole asks its own `ctrl+alt+t` → `Ok`.
  - `check_refuses_unsupported_keys`: `ctrl+escape` → `UnsupportedKey("escape")`.
  - `table_check_refuses_while_pending`.
  - `refusal_texts`: `TakenBy(konsole)` displays `Already set on {id}. Clear it there first.` — the UI replaces the id with the title (Task 8), so Display uses the id; `LauncherHotkey` → `That is the launcher hotkey.`; `NoModifier` → `Add Ctrl, Alt or Super.`; `UnsupportedKey(k)` → `{k} cannot be a hotkey.`; `Pending` → `Waiting for the last change.`; `BrokenConfig(e)` → `config.toml has an error: {e}`.
- [ ] **Step 2: Run** `cargo test -p tomakast-core hotkeys` — Expected: FAIL.
- [ ] **Step 3: Implement**; `daemon.rs` keeps building the launcher binding itself and calls `item_hotkeys` for the rest, logging each returned line with `warn!`.
- [ ] **Step 4: Run** `cargo test -p tomakast-core && cargo test -p tomakast-app` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): hotkey rules shared by the daemon and the recorder`.

### Task 4: Comment-preserving config editor and atomic write

**Files:**
- Create: `crates/core/src/settings_edit.rs`
- Modify: `Cargo.toml` (`toml_edit = "0.25"` in workspace deps), `crates/core/Cargo.toml`, `crates/core/src/lib.rs`
- Test: `crates/core/tests/settings_edit.rs`

**Interfaces:**
- Produces:
  ```rust
  /// `text` with every `[hotkeys]` entry of `item` removed (keys matched by parsed `Hotkey`,
  /// values by `ItemId`), then `"<hotkey Display>" = "<item>"` appended for `Some`.
  /// Errors when `text` is not valid settings (`Settings::from_toml` fails).
  pub fn set_item_hotkey(text: &str, item: &ItemId, hotkey: Option<&Hotkey>) -> anyhow::Result<String>;
  /// Writes `text` to `path` atomically: temp file in the target's directory, then rename.
  /// A symlinked `path` is resolved first so the link survives. Creates a missing file.
  pub fn write_config(path: &Path, text: &str) -> anyhow::Result<()>;
  ```
  `Settings::from_toml` becomes `pub(crate)` for this.

- [ ] **Step 1: Failing tests:**
  - `adding_keeps_comments_and_other_tables`: input with a top comment, `[launcher]` with a trailing comment, no `[hotkeys]`; output contains every original line verbatim and ends with `[hotkeys]\n"Ctrl+Alt+T" = "apps/kitty"`.
  - `changing_removes_every_spelling_of_the_items_entries` (Review Focus 3): `"alt+ctrl+T" = "apps/kitty"` and `"Ctrl+Alt+K" = "apps/kitty"` and `"Ctrl+Alt+X" = "apps/other"` with a comment line between; setting Kitty to `Ctrl+Alt+Y` leaves the comment, the other entry, and exactly one Kitty entry `"Ctrl+Alt+Y" = "apps/kitty"`.
  - `clearing_removes_the_entry`: `None` leaves no `apps/kitty` value; the `[hotkeys]` header stays.
  - `a_broken_file_is_refused`: `"[hotkeys\n"` → `Err`; and `"[hotkeys]\n\"Ctrl+\" = \"a/b\""` → `Err`.
  - `write_config_replaces_the_symlink_target` (`#[cfg(unix)]`): link → target; after the write the link is still a symlink and the target has the new text.
  - `write_config_creates_a_missing_file`.
  - `the_result_parses_back`: `Settings::load` of the written file has `hotkeys == [(Ctrl+Alt+Y, apps/kitty)]`.
- [ ] **Step 2: Run** `cargo test -p tomakast-core --test settings_edit` — Expected: FAIL.
- [ ] **Step 3: Implement** with `toml_edit::DocumentMut`; temp file named `.config.toml.tmp-<pid>` in the target's directory.
- [ ] **Step 4: Run** the same command — Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): edit [hotkeys] in config.toml without losing comments`.

### Task 5a: Hotkey service API, null and `global-hotkey` backends

**Files:**
- Modify: `crates/platform/src/hotkeys.rs` (new API; `register_global_hotkeys` and `HotkeyBackend` are removed in Task 9), `crates/platform/Cargo.toml` (`global-hotkey = "0.10"` for `cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))`; `sha2` workspace dep for Task 5b), `crates/platform/src/lib.rs`
- Create: `crates/platform/src/hotkeys/native.rs`, `crates/platform/src/hotkeys/null.rs`
- Test: in those files

**Interfaces:**
- Consumes: `tomakast_core::keys::SUPPORTED_KEYS` (Task 2).
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum SetKind { Launcher, Items }
  pub enum BindState { Active { trigger: Option<String> }, Refused { reason: String }, Later }
  pub enum HotkeyEvent {
      Activated(String),
      /// `Err`: the whole set was refused or cancelled; the previous one stays bound.
      Applied { set: SetKind, generation: u64, outcome: Result<Vec<(String, BindState)>, String> },
      Unavailable { reason: String },
  }
  pub trait Registrar { fn set(&mut self, set: SetKind, generation: u64, bindings: Vec<HotkeyBinding>); }
  pub struct HotkeyService { /* backend */ }
  impl HotkeyService {
      /// Picks the backend: `TOMAKAST_NO_KEYBOARD=1` → null; Linux: `org.kde.kglobalaccel`
      /// owned → kglobalaccel (Task 5b), else `WAYLAND_DISPLAY` → portal (Task 5b), else
      /// `DISPLAY` → native (X11), else null; macOS, Windows → native. macOS: call on the main thread; Windows: on a thread with a
      /// message loop (the GPUI `run` callback is both).
      pub fn start(on_event: impl Fn(HotkeyEvent) + Send + Sync + 'static) -> HotkeyService;
  }
  impl Registrar for HotkeyService {}
  ```
  Until Task 5b exists, the kglobalaccel and portal branches use the null backend.
- Native backend internals (pure, tested): `fn code(key: &str) -> Option<global_hotkey::hotkey::Code>`, `fn native_hotkey(&Hotkey) -> Option<global_hotkey::hotkey::HotKey>`, and `struct Diff` computed by `fn plan(current: &[(String, Hotkey)], wanted: &[(String, Hotkey)]) -> Diff { register: Vec<(String, Hotkey)>, unregister: Vec<(String, Hotkey)> }` over both sets. Apply order: register every `register` entry; for an id whose new registration failed, keep its old registration (do not unregister it) and report `Refused { reason }`; then unregister the rest. `Activated` maps `GlobalHotKeyEvent.id` (only `HotKeyState::Pressed`) back to the logical id.
- Null backend: emits `Unavailable { reason }` once at start; each `set` answers `Applied { outcome: Ok(all Active { trigger: None }) }` at once.

- [ ] **Step 1: Failing tests:**
  - `every_supported_key_has_a_native_code`: over `SUPPORTED_KEYS`.
  - `plan_registers_added_and_unregisters_removed`: current `[(a, ctrl+1)]`, wanted `[(a, ctrl+2), (b, ctrl+3)]` → register `[(a, ctrl+2), (b, ctrl+3)]`, unregister `[(a, ctrl+1)]`.
  - `plan_of_an_unchanged_set_is_empty`.
  - `null_backend_accepts_everything` (calls `set`, collects events through the callback): `Unavailable` first, then `Applied { generation: 7, outcome: Ok([("a", Active { trigger: None })]) }`.
- [ ] **Step 2: Run** `cargo test -p tomakast-platform` — Expected: FAIL.
- [ ] **Step 3: Implement.** Registrations run on the thread that called `start`; `set` is called from the same (UI) thread. Events come from `GlobalHotKeyEvent::set_event_handler`.
- [ ] **Step 4: Run** `cargo test -p tomakast-platform` — Expected: PASS. Cross-checks: `cargo clippy -p tomakast-platform --all-targets --target aarch64-apple-darwin -- -D warnings` and `--target x86_64-pc-windows-gnu` — Expected: clean (if the macOS target fails only for a C toolchain reason unrelated to this crate, record it in the acceptance notes; do not paper over Rust errors).
- [ ] **Step 5: Commit** `feat(platform): hotkey service with native (X11, macOS, Windows) and null backends`.

### Task 5b *(rev)*: kglobalaccel backend, and the portal bound once

**Files:**
- Create: `crates/platform/src/hotkeys/kde.rs`
- Modify: `crates/platform/src/hotkeys/linux.rs` (portal: bind once when both sets arrived), `crates/platform/src/hotkeys.rs` (backend choice), `crates/platform/Cargo.toml` (`zbus` from the workspace for Linux)
- Test: in `kde.rs`; `crates/platform/tests/kde_live.rs` (`#[ignore]`, run by hand on KDE)

**Interfaces:**
- Consumes: `Registrar`, `HotkeyEvent`, `BindState`, `SetKind` (Task 5a); `keys::qt_code` (Task 2).
- Produces: nothing new outward. D-Bus facts (from `docs/experiments/2026-11-item-hotkeys-portal-probe.md`): service `org.kde.kglobalaccel`, `/kglobalaccel`, `org.kde.KGlobalAccel`: `doRegister(as)`, `setShortcutKeys(as, a(ai), u) -> a(ai)` with flags `6`, `globalShortcutsByKey((ai), (i)) -> a(ssssssaiai)` (action, action friendly, component, component friendly, context, context friendly, keys, default keys), `unregister(ss) -> b`, `getComponent(s) -> o`; component `/component/dev_tomakast_Tomakast`, `org.kde.kglobalaccel.Component`: `shortcutNames() -> as`, `invokeShortcut(s)`, signal `globalShortcutPressed(s, s, x)`. An action id is `[component, action, component friendly, action friendly]` = `["dev.tomakast.Tomakast", <logical id>, "tomakast", <description>]`. A key list is one `(ai)` of 4 ints: `[qt_code, 0, 0, 0]`.
- Pure internals, tested: `fn holders_conflict(holders: &[Holder], own_component: &str, own_action: &str) -> Option<String>` (the first holder that is not this action, as `"used by {component friendly}: {action friendly}"`); `fn plan(current: &BTreeMap<String, Hotkey>, wanted: &[HotkeyBinding]) -> (Vec<HotkeyBinding> /*set*/, Vec<String> /*unregister*/)` per set; `fn strays(names: &[String], wanted: &BTreeSet<String>) -> Vec<String>`.

- [ ] **Step 1: Failing tests:**
  - `a_combination_held_by_another_app_is_refused_with_its_name`: holders `[("_launch","Launch","kitty.desktop","kitty")]` → `Some("used by kitty: Launch")`.
  - `our_own_action_is_not_a_conflict`.
  - `plan_sets_changed_and_unregisters_dropped`.
  - `strays_are_actions_in_neither_set`.
  - `kde_live` (`#[ignore]`): component `dev.tomakast.KdeLiveTest`, binds `Ctrl+Alt+Shift+K` to `t1` through the backend, calls `invokeShortcut("t1")` over D-Bus, expects `HotkeyEvent::Activated("t1")` within 2 s, then a second binding asking the same combination for `t2` comes back `Refused`, then unregisters both. The component name is a parameter of the backend constructor for this.
- [ ] **Step 2: Run** `cargo test -p tomakast-platform` — Expected: FAIL.
- [ ] **Step 3: Implement** on one worker thread with a `zbus` blocking connection: a command channel for `set`, a signal stream for `globalShortcutPressed` filtered on the component path. At the first `set`, unregister strays (`shortcutNames` of the component minus both sets' ids — strays are only removed once both sets have been set once). For the `Launcher` set, a conflict is logged (`warn!` with the holder) and the keys are set anyway. Report `Applied { outcome: Ok(states) }`; `Err` only when D-Bus calls fail. Portal (`linux.rs`): keep today's single session; bind once when both sets have arrived, report `Ok` (triggers from the response); every later `set` → `Ok` with `Later` for ids whose combination changed, `Active` otherwise.
- [ ] **Step 4: Run** `cargo test -p tomakast-platform` — Expected: PASS. Then `cargo test -p tomakast-platform --test kde_live -- --ignored` on this desktop — Expected: PASS (it uses a test component and unregisters what it made).
- [ ] **Step 5: Commit** `feat(platform): KDE hotkeys through kglobalaccel; portal binds once`.

### Task 6: Daemon hotkey state machine

**Files:**
- Create: `crates/app/src/hotkeys.rs`
- Modify: `crates/app/src/lib.rs` or `main.rs` (module declaration, whichever declares `daemon`)
- Test: in `hotkeys.rs` (fake `Registrar`, temp config files)

**Interfaces:**
- Consumes: Tasks 3, 4, 5a.
- Produces:
  ```rust
  pub(crate) const LAUNCHER_ID: &str = "launcher";
  pub(crate) enum Outcome { Table(HotkeyTable), Notice(String), Pressed { id: String, hotkey: Hotkey } }
  pub(crate) struct Hotkeys<R: Registrar> { /* registrar, config_file, generations, applied sets, pending, states, unavailable */ }
  impl<R: Registrar> Hotkeys<R> {
      pub fn new(registrar: R, config_file: PathBuf) -> Self;
      /// Settings (re)loaded from the file: binds each set whose (id, hotkey) list differs
      /// from the last applied one. `title` names an item in KDE's shortcut settings.
      pub fn load(&mut self, settings: &Settings, title: &dyn Fn(&ItemId) -> Option<String>) -> Vec<Outcome>;
      /// The recorder's Enter (`Some`) or Backspace (`None`). Re-reads the file, checks with
      /// `hotkeys::check`, and binds the item set with the change as the pending change.
      pub fn request(&mut self, item: &ItemId, hotkey: Option<Hotkey>, title: &dyn Fn(&ItemId) -> Option<String>) -> Result<Vec<Outcome>, Refusal>;
      pub fn on_event(&mut self, event: HotkeyEvent) -> Vec<Outcome>;
      pub fn table(&self) -> HotkeyTable;
  }
  ```
  Behaviour (spec "After Enter" and "Daemon"):
  - `request` while a change is pending → `Err(Pending)`; file unparsable → `Err(BrokenConfig)` and nothing bound (Review Focus 4).
  - *(rev)* `Applied` with the pending id `Later` → write the file, state `AfterRestart`.
  - `Applied` for the pending generation: `Ok` with the pending id `Active` → re-read the file; if the change still passes `check` against it, `set_item_hotkey` + `write_config`, make it the applied set, emit `Table`; if not (a conflicting hand edit), re-bind the previous applied set and emit `Notice("config.toml changed meanwhile; nothing was saved")`. `Err(reason)` or the id `Refused` → drop the pending change (native: re-issue the previous set) and emit `Notice("{hotkey} for {title}: {reason}; nothing changed")` (reason e.g. `used by kitty: Launch`).
  - A write error after acceptance → re-bind the previous set, `Notice("Cannot save config.toml: {e}")`.
  - `load` with an unchanged set binds nothing (the watcher's echo of our own write).
  - `load` of a broken file is not called (the caller logs and keeps the sets).
  - `Activated(id)` → `Pressed { id, hotkey }` using the applied set; unknown id → nothing.
  - `Unavailable` → table `unavailable = Some(reason)`, states `Off`.
  - Hand-edited item bindings the system refused → state `Refused`.
- [ ] **Step 1: Failing tests:**
  - `startup_binds_both_sets_once`: `load` → fake saw `set(Launcher, …)` and `set(Items, …)`; a second identical `load` → no new calls.
  - `an_accepted_change_is_written_after_acceptance`: `request(kitty, Some(ctrl+alt+t))` → file unchanged; then `Applied { Ok([kitty Active]) }` → file contains `"Ctrl+Alt+T" = "apps/kitty"`, table shows kitty `Active`.
  - `a_cancelled_change_writes_nothing_and_keeps_the_previous_set`: `Applied { Err("cancelled") }` → file byte-identical, a `Notice`, table has no pending, no extra `set` for the portal case.
  - `a_second_request_while_pending_is_refused` (Review Focus 5).
  - `a_conflicting_hand_edit_skips_the_write` (Review Focus 5): pending kitty `ctrl+alt+t`; before `Applied`, the test writes the file with konsole on `ctrl+alt+t`; on `Applied Ok` → file is the hand-edited text, the previous set is re-bound, `Notice` emitted.
  - `a_broken_file_refuses_the_change_and_keeps_the_sets` (Review Focus 4).
  - `the_launcher_set_is_never_rebound_by_an_item_change`.
  - `presses_are_reported_with_their_hotkey`.
  - `clearing_is_a_pending_change_too`: `request(kitty, None)` → on `Ok` the Kitty entry is gone from the file.
- [ ] **Step 2: Run** `cargo test -p tomakast-app hotkeys` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p tomakast-app` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(app): hotkey state machine: bind first, write on acceptance`.

### Task 7: Controller API, panel entry and row chips

**Files:**
- Create: `crates/ui/src/hotkeys.rs` (trait, chip element)
- Modify: `crates/core/src/core.rs` (`is_indexed`), `crates/ui/src/window.rs` (controller fields and methods), `crates/ui/src/launcher_view.rs` (root rows; panel entry), `crates/ui/src/views/action_panel.rs` (entry id), `crates/ui/src/views/mod.rs` (`row` takes a trailing element), `crates/ui/src/lib.rs` (re-exports)
- Test: `crates/ui/tests/hotkeys.rs`

**Interfaces:**
- Consumes: `HotkeyTable`, `HotkeyState` (Task 3); `shortcut::chips` (exists).
- Produces:
  ```rust
  pub trait HotkeyRequests { fn assign(&self, item: &ItemId, hotkey: Option<Hotkey>); }
  impl LauncherController {
      pub fn set_hotkey_requests(&mut self, requests: Rc<dyn HotkeyRequests>);
      pub fn set_hotkey_table(&mut self, table: HotkeyTable, cx: &mut App); // re-renders the open window
      pub fn hotkey_notice(&mut self, text: String, cx: &mut App);         // toast now, or at the next show
      pub fn is_recording(&self) -> bool;
      pub fn recorded_press(&mut self, hotkey: Hotkey, cx: &mut App);      // Task 8 fills the body
  }
  pub(crate) const ASSIGN_HOTKEY: &str = "\u{1}assign-hotkey"; // module ids cannot hold control chars (`is_valid_id`)
  ```
  - The panel entry is appended only for a `PanelTarget::Root` whose item is in the index (`Core` gains `pub fn is_indexed(&self, item: &ItemId) -> bool`) and only when requests are set (so existing panel tests are unchanged). Title `Assign Hotkey…`, or `Change Hotkey…` when the table has the item.
  - Chips on a root row, right of the other accessories, with `aria_label` `Hotkey {Display}` plus ` (after restart)`, ` (not set: {reason})` or ` (off)` per state (`Active` with a `trigger` that differs from the recorded combination labels KDE's text instead: `Hotkey {trigger}`).
- [ ] **Step 1: Failing tests** (setup like `crates/ui/tests/action_panel.rs`, plus a fake `HotkeyRequests` recording calls):
  - `no_entry_without_hotkey_support`: panel titles unchanged.
  - `entry_is_last_for_an_indexed_root_item`: `["Open", "Reveal in Files", "Copy Path", "Assign Hotkey…"]`.
  - `entry_says_change_when_the_item_has_one`.
  - `row_shows_its_hotkey`: table with `fake/files` Active `ctrl+alt+f` → an element labelled `Hotkey Ctrl+Alt+F`.
  - `row_shows_the_state`: AfterRestart → `Hotkey Ctrl+Alt+F (after restart)`; Off → `(off)`.
  - `no_entry_for_rows_of_a_pushed_view`.
  - `notice_while_hidden_shows_at_next_open`.
- [ ] **Step 2: Run** `cargo test -p tomakast-ui --test hotkeys` — Expected: FAIL.
- [ ] **Step 3: Implement.** Choosing the entry opens the recorder (Task 8); until then it closes the panel.
- [ ] **Step 4: Run** `cargo test -p tomakast-ui` — Expected: PASS (all existing UI tests too).
- [ ] **Step 5: Commit** `feat(ui): hotkey chips on rows and the Assign Hotkey entry`.

### Task 8: The recorder

**Files:**
- Create: `crates/ui/src/views/hotkey_recorder.rs`
- Modify: `crates/ui/src/launcher_view.rs` (recorder state, key routing, key context), `crates/ui/src/views/mod.rs`, `crates/ui/src/window.rs` (`is_recording`, `recorded_press`)
- Test: `crates/ui/tests/hotkeys.rs`

**Interfaces:**
- Consumes: Task 7; `keys::{is_supported, unshifted}` (Task 2); `HotkeyTable::check` (Task 3).
- Produces: `pub(crate) struct HotkeyRecorder { item: ItemId, title: String, shown: Option<Hotkey>, refusal: Option<String>, focus: FocusHandle }` with `fn press(&mut self, keystroke: &Keystroke, table: &HotkeyTable, titles: &dyn Fn(&ItemId) -> String) -> RecorderStep` where `enum RecorderStep { Shown, Save(Hotkey), Clear, Cancel, Ignored }`.
  - Key routing: while a recorder is open, the root element's key context is `HotkeyRecorder` instead of the launcher's contexts (no launcher or input binding matches), focus is on the recorder's handle, and the root `on_key_down` hands every key-down to `press`.
  - Plain `escape` → Cancel (back to the panel over the same row); plain `enter` → Save when `shown` passes `check`, else stays with the refusal; plain `backspace` → Clear (when the row has a hotkey; otherwise Ignored); modifiers-only → Ignored; a symbol key with `unshifted` → its base key with Shift; a key that is not `is_supported` → refusal `UnsupportedKey`. `TakenBy(id)` is shown with the other row's title: `Already set on "Konsole". Clear it there first.`
  - Save/Clear call `HotkeyRequests::assign` and close the recorder; the row then shows the table the daemon sends.
  - `recorded_press(hotkey)` feeds the open recorder as if pressed in it (the combination is a tomakast hotkey the system fired instead of the window).
  - Unavailable table: the recorder shows `Hotkeys are not available here: bind "tomakast run {id}" in your system settings.` and a Copy button that copies `tomakast run {id}`; Enter still saves (null service writes at once).
  - Hint line: `Nothing appears? The system or another app holds that combination.` Footer: `↵ Save · ⌫ Clear · Esc Cancel`.
- [ ] **Step 1: Failing tests:**
  - `recorder_shows_the_pressed_combination`: Ctrl+K, choose entry, press `ctrl-alt-t` → element `hotkey-recorder` labelled with `Ctrl+Alt+T`.
  - `recorder_captures_launcher_keys`: press `ctrl-k` inside the recorder → shows `Ctrl+K`, the recorder stays open.
  - `recorder_saves_on_enter`: → fake requests saw `(fake/firefox, Some(ctrl+alt+t))`; recorder closed.
  - `recorder_refuses_a_taken_combination` (decision 2): table has `fake/files` on `ctrl+alt+t`; press it, Enter → no request; refusal text names `"Files"`.
  - `recorder_refuses_without_modifier`: press `t` → refusal `Add Ctrl, Alt or Super.`
  - `recorder_records_shifted_digit_as_shift_and_digit` (Review Focus 1): press `ctrl-!` (how GPUI reports Ctrl+Shift+1) → shows `Ctrl+Shift+1`.
  - `recorder_refuses_a_non_latin_key` (Review Focus 2): press `ctrl-е` → refusal `е cannot be a hotkey.`
  - `backspace_clears`: row with a hotkey → request `(item, None)`.
  - `escape_returns_to_the_panel`.
  - `a_fired_hotkey_is_recorded`: `recorded_press(ctrl+alt+t)` while recording → shows `Ctrl+Alt+T`.
  - `recorder_explains_unavailable_hotkeys`.
- [ ] **Step 2: Run** `cargo test -p tomakast-ui --test hotkeys` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p tomakast-ui` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(ui): hotkey recorder in the action panel`.

### Task 9: Daemon wiring and its own config watcher

**Files:**
- Create: `crates/app/src/config_watch.rs`
- Modify: `crates/app/src/daemon.rs` (replace `register_hotkeys`, `hotkey_bindings`, `activation_request`), `crates/app/Cargo.toml` (`notify.workspace = true`), `crates/platform/src/hotkeys.rs` (remove `register_global_hotkeys`, `HotkeyBackend`)
- Test: in `config_watch.rs`; existing daemon tests keep passing

**Interfaces:**
- Consumes: Tasks 5a/5b, 6; `LauncherController` API from Task 7 (`set_hotkey_table`, `hotkey_notice`, `is_recording`, `recorded_press`, `set_hotkey_requests`).
- Produces: `pub(crate) fn watch_config(path: &Path, on_change: impl Fn() + Send + 'static) -> anyhow::Result<ConfigWatch>` — notify on the parent directory (non-recursive), filtered on the file name, debounced 300 ms; dropping `ConfigWatch` stops it.

- [ ] **Step 1: Failing test** `config_watch_fires_once_per_burst` (tempdir; three writes 50 ms apart → exactly one callback within 2 s) and `config_watch_ignores_other_files`.
- [ ] **Step 2: Run** `cargo test -p tomakast-app config_watch` — Expected: FAIL.
- [ ] **Step 3: Implement the watcher**, then wire the daemon inside the GPUI `run` callback:
  - `HotkeyService::start` with a callback posting each `HotkeyEvent` into a new `async_channel`; a `cx.spawn` loop on the UI thread feeds `Hotkeys::on_event` and applies its outcomes.
  - `Hotkeys` lives in an `Rc<RefCell<…>>` on the UI thread; `load(&settings, title)` at start, where `title` reads `core.borrow()` for the indexed item's title.
  - The watcher posts a reload; the UI thread does `Settings::load` (error → `warn!`, sets kept) and `load`.
  - Outcomes: `Table` → `LauncherController::set_hotkey_table`; `Notice` → `hotkey_notice`; `Pressed { id, hotkey }` → if `is_recording()` then `recorded_press(hotkey)` else `Request::Toggle` for `LAUNCHER_ID` or `run_item` for an item id, stamped with `Instant::now()` as today.
  - `set_hotkey_requests(Rc<impl HotkeyRequests>)` whose `assign` calls `Hotkeys::request`; a `Refusal` goes back as a notice.
- [ ] **Step 4: Run** `cargo test -p tomakast-app && cargo clippy -p tomakast-app --all-targets -- -D warnings` — Expected: PASS. Headless smoke run with `TOMAKAST_HOME=$(mktemp -d) TOMAKAST_NO_KEYBOARD=1 target/release/tomakast daemon` for 3 s; log shows the null backend's "unavailable" line and no panic; kill only that PID.
- [ ] **Step 5: Commit** `feat(app): live hotkeys: service, config watcher, recorder routing`.

### Task 10: Acceptance

**Files:**
- Create: `docs/experiments/2026-11-item-hotkeys-acceptance.md`

- [ ] **Step 1: Run** `just check` — Expected: all steps pass. Then the cross-target clippy commands from Task 5a for `tomakast-platform` (the app crate pulls GPUI, whose macOS build needs a Metal toolchain; CI covers it).
- [ ] **Step 2: Write the acceptance note** (English): what was run here (unit, headless UI, the probe on KDE), what was not (macOS, Windows, X11 presses; macOS main-thread and Windows message-loop requirements; system-shortcut precedence), and the manual checklist for each OS. Include the precise per-OS answer from the spec's table.
- [ ] **Step 3: Give the user `just try` steps** (in Russian): `just try approve`, `just toggle`, select Kitty or any app, Ctrl+K → "Assign Hotkey…", press a combination, Enter, press the combination with the launcher hidden; a combination another app holds (e.g. kitty's Ctrl+Alt+T) is refused with its name; a conflict with another row; `just log` for the bind lines; check `config.toml` in the try home kept a hand-added comment. Remind that only one `just try` runs at a time across worktrees.
- [ ] **Step 4: Commit** `docs(experiments): item hotkeys acceptance`.
