# Leader Key Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A user-chosen leader key (any key, combination or modifiers alone) that turns any key into a hotkey: hold the leader and press F (or, in tap mode, press the leader, then F) to run the item assigned to `Leader+F`.

**Architecture:** The leader is a third global-hotkey set. Backends report its press and its release. While it is down, the daemon opens a 1×1 invisible window that takes the keyboard. A pure state machine in `tomakast-core` turns key events into "run item X" and "close". A modifier leader in hold mode needs no window: `Leader+F` is registered directly as `<leader mods>+F`. Hotkeys are stored as `Leader+F` in `[hotkeys]`, so they follow a new leader without rewriting.

**Tech Stack:** Rust, GPUI (gpui-kit =0.10.0, patched gpui-pre-linux), zbus (kglobalaccel), `global-hotkey` 0.10, `toml_edit`.

**Spec:** `docs/superpowers/specs/2026-11-22-leader-key-design.md`. Read it first; this plan does not repeat its reasoning.

**Base:** branch `claude/leader-key-238dbd` is rebased onto `claude/settings-gui-3345c2` (unmerged). Code names below (`HotkeyTarget`, `HotkeyOrigin`, `set_key`, `HotkeyField`, `settings_schema`) come from that branch.

## Global Constraints

- UI strings in code are English.
- `tomakast-platform` has no GPUI dependency.
- `TOMAKAST_NO_KEYBOARD=1`: nothing is bound and no keyboard-taking window opens. This includes the leader window: the daemon never opens it under this variable.
- Run the app only through `just` recipes. Never open a keyboard-taking window on the user's session. Tests are headless.
- The launcher window's open path stays as it is (~18 ms warm). Leader work runs only on a leader press.
- Safety close: the leader window closes 5 s after the last key event, in both modes.
- `just check` passes at the end of every task (or at least `cargo fmt --check`, clippy `-D warnings`, and the crate's tests, with full `just check` before the final commit).
- No attribution lines in commits. No screenshots in commits.

## Review Focus

1. **The leader's release never arrives** (KDE does not report it while our window has the keyboard, or a backend drops it). The window must close within 5 s by itself. Test: Task 3 `safety_timeout_closes_held` and `safety_timeout_closes_armed`.
2. **The leader is released before the window has opened** (a quick tap in hold mode). Nothing may stay open. Test: Task 3 `release_before_open_closes_at_once`.
3. **Any non-Latin layout is active** (Russian, Greek, Hebrew, Thai, …). Letters must find the same hotkeys as on US: GPUI already names them by the US key on every platform, so a key that arrives as a non-ASCII letter is still tried through the tables, and ASCII punctuation a Cyrillic layout moves (`.` on the `/` key) is mapped back. Test: Task 3 `russian_layout_keys_map_to_us`.
4. **The leader changes to one that makes two hotkeys the same keys, or is the key of its own hotkey.** It must be refused before anything is bound, and the old leader must keep working. Tests: Task 2 `leader_change_clash_is_refused` and `leader_key_of_own_hotkey_is_refused`; Task 6 `refused_leader_change_keeps_the_old_leader`.
5. **Key auto-repeat while holding F under the leader.** The item runs once, not 25 times a second. Test: Task 3 `auto_repeat_runs_once`.

---

## File Map

| File | Change |
|---|---|
| `crates/core/src/settings.rs` | `Modifiers::LEADER`; `Hotkey::parse_leader`; `Hotkey::is_modifiers_only`; Display of a modifiers-only hotkey; `Settings.leader`, `Settings.leader_mode` |
| `crates/core/src/settings_schema.rs` | keys `launcher.leader` (`ValueKind::Leader`) and `launcher.leader_mode` (`ValueKind::Choice`) |
| `crates/core/src/keys.rs` | rows `pause`, `scrolllock`, `menu`, `f13`–`f24`; Qt codes of lone modifiers; `us_key` through layout tables |
| `crates/core/src/hotkeys.rs` | `HotkeyTarget::Leader`; `Keyspace`; leader-aware `check`; `check_leader`; new `Refusal`s; `HotkeyState::NoLeader`; table fields |
| `crates/core/src/leader.rs` (new) | `LeaderMode`, `LeaderMachine`, `LeaderInput`, `LeaderEffect`, `window_key`, `LeaderRecording` |
| `crates/platform/src/hotkeys.rs` + `hotkeys/{kde,native,linux,null}.rs` | `HotkeyEvent::Released`; `SetKind::Leader`; release forwarding; lone-modifier binding on KDE; `HotkeyService::grabs_keyboard_while_held` |
| `crates/app/src/hotkeys.rs` | leader set; item set as sent to the system; `Leader` requests; `Outcome::Released`; table |
| `crates/app/src/leader.rs` (new) | daemon-side driver: machine + window + timer |
| `crates/app/src/daemon.rs` | wiring, routing of press/release to recorders |
| `crates/ui/src/leader_window.rs` (new) | the invisible window |
| `crates/ui/src/views/hotkey_recorder.rs`, `crates/ui/src/settings/fields.rs` | "leader down" recording; modifiers-only recording for the leader field |
| `crates/ui/src/hotkeys.rs`, `crates/ui/src/shortcut.rs` | `Leader` chip, `NoLeader` state |
| `crates/ui/src/settings/pages/{general,hotkeys}.rs`, `settings/window.rs` | leader field and mode choice |
| `modules/settings/src/lib.rs` | "Set Leader Key…" item |
| `docs/experiments/2026-11-leader-key-acceptance.md` (new) | acceptance note |

**Planning ruling (recorded in the spec):** "Set Leader Key…" opens the settings window on the Hotkeys page, where the leader field records the leader. It does not get a recorder of its own in the launcher. The settings window already has the launcher-hotkey field with bind-first/write-after, which is the flow the spec asks for. A second launcher recorder for a non-item target would duplicate it.

---

### Task 1: Leader values in settings and the key table

**Files:**
- Modify: `crates/core/src/settings.rs`, `crates/core/src/settings_schema.rs`, `crates/core/src/keys.rs`, `crates/ui/src/settings/pages/plugins.rs` (exhaustive `ValueKind` match), `crates/platform/src/hotkeys/native.rs` (`code` for new keys)
- Test: in-file `#[cfg(test)]` modules of those files

**Interfaces:**
- Produces:
  - `Modifiers::LEADER` (bit `1 << 4`). It is never sent to a backend.
  - `Hotkey::parse_leader(s: &str) -> Result<Hotkey, HotkeyError>`: like `parse`, but also accepts modifiers alone (`"Ctrl"`, `"Ctrl+Alt"`) as `Hotkey { mods, key: "" }`. Rejects `Leader` inside a leader.
  - `Hotkey::parse` accepts `Leader` as a modifier name (case-insensitive): `"Leader+Shift+F"` → `mods = LEADER|SHIFT, key = "f"`.
  - `Hotkey::is_modifiers_only(&self) -> bool`.
  - `Display`: `Leader` is printed first (`Leader+Shift+F`). A modifiers-only hotkey prints without a trailing `+` (`Ctrl+Alt`).
  - `Settings.leader: Option<Hotkey>` (from `[launcher] leader`) and `Settings.leader_mode: LeaderMode` (from `[launcher] leader_mode`, default `Hold`). `LeaderMode` is defined in `crates/core/src/leader.rs` (create the file with just the enum in this task: `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)] pub enum LeaderMode { #[default] Hold, Tap }`, with `as_str()` returning `"hold"`/`"tap"`).
  - `ValueKind::Leader`. Schema keys:
    - `["launcher","leader"]`: kind `Leader`, default `""`, title `"Leader key"`, description `"Hold it and press a key to run that key's hotkey."`, page General, keywords `["leader", "shortcut", "keyboard"]`, `OnInvalid::Error`.
    - `["launcher","leader_mode"]`: kind `Choice` with `hold` → `"Hold the leader"` and `tap` → `"Press, then press a key"`, default `"hold"`, title `"Leader mode"`, `OnInvalid::Warn`.
  - `keys` table rows, as (config name, xkb keysym, Qt code):
    - `("pause","Pause",0x0100_0008)`, `("scrolllock","Scroll_Lock",0x0100_0026)`, `("menu","Menu",0x0100_0055)`;
    - `("f13","F13",0x0100_003c)` through `("f24","F24",0x0100_0047)`.
  - `SUPPORTED_KEYS` follows the table.
  - `keys::qt_code`: a modifiers-only hotkey with exactly one modifier maps to that modifier's key: Shift `0x0100_0020`, Ctrl `0x0100_0021`, Super `0x0100_0022`, Alt `0x0100_0023`. Several modifiers → `None`. `LEADER` in `mods` → `None`.
  - `keys::gpui_alias(key: &str) -> &str`: `"scroll_lock"` → `"scrolllock"`; everything else unchanged.
  - `native::code`: `Pause`, `ScrollLock`, `ContextMenu`, `F13`–`F24`.

- [ ] **Step 1: Write failing tests**

```rust
// settings.rs tests
#[test]
fn leader_hotkeys_parse_and_display() {
    let h = Hotkey::parse("leader+shift+F").unwrap();
    assert_eq!(h.mods, Modifiers::LEADER | Modifiers::SHIFT);
    assert_eq!(h.key, "f");
    assert_eq!(h.to_string(), "Leader+Shift+F");
}
#[test]
fn a_leader_may_be_modifiers_alone() {
    assert_eq!(Hotkey::parse_leader("Ctrl").unwrap().to_string(), "Ctrl");
    assert!(Hotkey::parse_leader("Ctrl+Alt").unwrap().is_modifiers_only());
    assert_eq!(Hotkey::parse_leader("F12").unwrap().key, "f12");
    assert!(Hotkey::parse_leader("Leader+F").is_err());
    assert!(Hotkey::parse("Ctrl").is_err());
}
#[test]
fn leader_settings_are_read() {
    let s = Settings::parse("[launcher]\nleader = \"F12\"\nleader_mode = \"tap\"\n").unwrap();
    assert_eq!(s.leader, Some(Hotkey::parse_leader("F12").unwrap()));
    assert_eq!(s.leader_mode, LeaderMode::Tap);
    let d = Settings::parse("").unwrap();
    assert_eq!((d.leader, d.leader_mode), (None, LeaderMode::Hold));
    assert!(Settings::parse("[launcher]\nleader = \"Ctrl+\"\n").is_err());
    // A bad mode warns and keeps the default.
    assert_eq!(Settings::parse("[launcher]\nleader_mode = \"x\"\n").unwrap().leader_mode, LeaderMode::Hold);
}
// keys.rs tests
#[test]
fn new_keys_have_names() {
    assert_eq!(xkb_keysym("scrolllock"), Some("Scroll_Lock"));
    assert_eq!(qt_key("f19"), Some(0x0100_0042));
    assert_eq!(qt_key("menu"), Some(0x0100_0055));
}
#[test]
fn a_lone_modifier_is_its_key_code() {
    assert_eq!(qt_code(&Hotkey::parse_leader("Ctrl").unwrap()), Some(0x0100_0021));
    assert_eq!(qt_code(&Hotkey::parse_leader("Ctrl+Alt").unwrap()), None);
    assert_eq!(qt_code(&Hotkey::parse("Leader+F").unwrap()), None);
}
```

Also update `non_latin_and_reserved_keys_are_not_supported`: `f13` is now supported, `f25` is not.

- [ ] **Step 2: Run** `cargo test -p tomakast-core settings keys`. Expected: FAIL (no `LEADER`, no `parse_leader`).
- [ ] **Step 3: Implement** in the files above. `Settings::parse` gets two schema arms: `(["launcher","leader"], String(s))` → `Hotkey::parse_leader` (an empty string means no leader); `(["launcher","leader_mode"], String(s))` → `LeaderMode`. `value_fits` and `expected` handle `ValueKind::Leader` ("a leader key", valid when `parse_leader` succeeds or the string is empty). Add a `ValueKind::Leader` arm to `pages/plugins.rs` that draws nothing, like `Hotkey`. Add native `code` rows and extend `every_supported_key_has_a_native_code`.
- [ ] **Step 4: Run** `cargo test -p tomakast-core && cargo test -p tomakast-platform && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "core: leader key values in settings, F13–F24, Pause, Scroll Lock, Menu"`

### Task 2: Leader-aware hotkey rules

**Files:**
- Modify: `crates/core/src/hotkeys.rs`, and its callers to compile (`crates/app/src/hotkeys.rs`, `crates/ui/src/views/hotkey_recorder.rs`, `crates/ui/src/settings/window.rs`, `crates/ui/src/settings/pages/*.rs`, `crates/ui/src/hotkeys.rs`). Callers only adapt; leader behaviour comes in later tasks.
- Test: `crates/core/src/hotkeys.rs` tests

**Interfaces:**
- Consumes: Task 1 (`LEADER`, `is_modifiers_only`, `LeaderMode`).
- Produces:
  - `HotkeyTarget::Leader`.
  - `pub struct Keyspace { pub launcher: Hotkey, pub leader: Option<Hotkey>, pub mode: LeaderMode }`, with `Keyspace::of(settings: &Settings) -> Keyspace`.
  - `pub fn system_form(hotkey: &Hotkey, keyspace: &Keyspace) -> Option<Hotkey>`: what the system registers.
    - No `LEADER` bit → `Some(hotkey)`.
    - `LEADER` with a modifiers-only leader in `Hold` → `Some(mods − LEADER + leader mods, same key)`.
    - Otherwise (key leader, or `Tap`, or no leader) → `None`: the leader window handles it.
  - `pub fn same_keys(a: &Hotkey, b: &Hotkey, keyspace: &Keyspace) -> bool`: `a == b`, or both have a `system_form` and the forms are equal.
  - `check(assigned, keyspace: &Keyspace, item, hotkey)`: replaces the `launcher: &Hotkey` parameter.
    - `usable` accepts `LEADER` as the needed modifier, so `Leader+F` and `Leader+Shift+1` pass.
    - Duplicates are found with `same_keys`.
    - `Refusal::LeaderKey` when `hotkey` has `LEADER`, the leader has a key, and `hotkey.key == leader.key`.
  - `check_launcher(assigned, keyspace, hotkey)`: also refuses `Refusal::LeaderHotkey` when `hotkey == leader`.
  - `pub fn check_leader(assigned: &[(ItemId, Hotkey)], keyspace: &Keyspace, leader: &Hotkey) -> Result<(), Refusal>`:
    - `Refusal::LauncherHotkey` when the leader equals `keyspace.launcher`;
    - `Refusal::LeaderKey` when an assigned `Leader+…` has the leader's key;
    - `Refusal::LeaderClash { leader_form: Hotkey, item: ItemId, other: ItemId }` when, under a keyspace with the new leader, two assigned hotkeys become `same_keys`;
    - `Refusal::Unavailable(String)` is *not* decided here (platform-dependent; Task 6).
  - New `Refusal` variants and their `Display` text:
    - `LeaderKey` → `"That key is the leader's own."`
    - `LeaderHotkey` → `"That is the leader key."`
    - `LeaderClash { leader_form, item, .. }` → `"With this leader, {leader_form} is set on {item} twice."`
    - `Unavailable(reason)` → `"{reason}"`

    The recorders' `refusal_text` maps `LeaderClash` to `With {leader}, {hotkey} is {form}, already set on "{title}".`, using the leader shown and titles. Example: `With Ctrl, Leader+F is Ctrl+F, already set on "Kitty".`
  - `HotkeyState::NoLeader`: a `Leader+…` hotkey while no leader is set.
  - `HotkeyTable` gains `leader: Option<Hotkey>`, `leader_state: Option<HotkeyState>`, `leader_mode: LeaderMode`. `check_target` builds its `Keyspace` from them, and `HotkeyTarget::Leader` goes to `check_leader`.
  - `sort_hotkeys` / `item_hotkeys` / `dropped_hotkeys` use `Keyspace::of(settings)` ("first entry wins" stays).

- [ ] **Step 1: Write failing tests**

```rust
fn ks(leader: Option<&str>, mode: LeaderMode) -> Keyspace {
    Keyspace { launcher: hk("alt+space"), leader: leader.map(|l| Hotkey::parse_leader(l).unwrap()), mode }
}
#[test]
fn leader_hotkeys_need_no_other_modifier() {
    assert_eq!(check(&[], &ks(Some("F12"), LeaderMode::Hold), &id("apps/f"), &hk("leader+f")), Ok(()));
}
#[test]
fn with_a_modifier_leader_in_hold_mode_leader_f_is_ctrl_f() {
    let assigned = [(id("apps/kitty"), hk("ctrl+f"))];
    let k = ks(Some("Ctrl"), LeaderMode::Hold);
    assert_eq!(check(&assigned, &k, &id("apps/ff"), &hk("leader+f")), Err(Refusal::TakenBy(id("apps/kitty"))));
    assert_eq!(system_form(&hk("leader+shift+f"), &k), Some(hk("ctrl+shift+f")));
    // In tap mode they are different gestures.
    assert_eq!(check(&assigned, &ks(Some("Ctrl"), LeaderMode::Tap), &id("apps/ff"), &hk("leader+f")), Ok(()));
    assert_eq!(system_form(&hk("leader+f"), &ks(Some("F12"), LeaderMode::Hold)), None);
}
#[test]
fn leader_key_of_own_hotkey_is_refused() {
    assert_eq!(check(&[], &ks(Some("Ctrl+Space"), LeaderMode::Hold), &id("apps/a"), &hk("leader+space")), Err(Refusal::LeaderKey));
    let assigned = [(id("apps/a"), hk("leader+f"))];
    assert_eq!(check_leader(&assigned, &ks(Some("F12"), LeaderMode::Hold), &Hotkey::parse_leader("F").unwrap()), Err(Refusal::LeaderKey));
}
#[test]
fn leader_change_clash_is_refused() {
    let assigned = [(id("apps/ff"), hk("leader+f")), (id("apps/kitty"), hk("ctrl+f"))];
    let r = check_leader(&assigned, &ks(Some("F12"), LeaderMode::Hold), &Hotkey::parse_leader("Ctrl").unwrap());
    assert!(matches!(r, Err(Refusal::LeaderClash { .. })), "{r:?}");
    assert_eq!(check_leader(&assigned, &ks(Some("F12"), LeaderMode::Tap), &Hotkey::parse_leader("Ctrl").unwrap()), Ok(()));
}
#[test]
fn the_leader_may_not_be_the_launcher_hotkey() {
    assert_eq!(check_leader(&[], &ks(None, LeaderMode::Hold), &hk("alt+space")), Err(Refusal::LauncherHotkey));
    assert_eq!(check_launcher(&[], &ks(Some("F12"), LeaderMode::Hold), &Hotkey::parse_leader("F12").unwrap()), Err(Refusal::LeaderHotkey));
}
```

- [ ] **Step 2: Run** `cargo test -p tomakast-core hotkeys`. Expected: FAIL to compile.
- [ ] **Step 3: Implement**, then adapt the callers. They pass `Keyspace::of(&settings)` or the table's keyspace. Exhaustive matches on `HotkeyTarget` get a `Leader` arm: `todo!()` is not allowed. Return a `Refusal::Unavailable("not yet")` placeholder only where Task 6 replaces it, and mark it `// Task 6`. `HotkeyState::NoLeader` renders like `Off` with the label `"no leader key"` in `crates/ui/src/hotkeys.rs::label`.
- [ ] **Step 4: Run** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "core: hotkey rules know the leader: Leader+key, same keys under a modifier leader, leader changes"`

### Task 3: The leader state machine and window keys

**Files:**
- Modify: `crates/core/src/leader.rs`, `crates/core/src/keys.rs` (`us_key`), `crates/core/src/lib.rs` (`pub mod leader`)
- Test: `crates/core/src/leader.rs` tests

**Interfaces:**
- Consumes: Task 1, Task 2 (`Keyspace` not needed; plain `Hotkey`s).
- Produces:

```rust
pub const SAFETY_CLOSE: Duration = Duration::from_secs(5);

pub enum LeaderInput {
    /// The leader's global shortcut fired (a press; in tap mode with a modifier leader, a tap).
    Pressed,
    Released,
    /// A key-down in the leader window, already in `Leader+…` form (see `window_key`);
    /// `None`: a key that names no supported key. `repeat`: auto-repeat.
    Key { hotkey: Option<Hotkey>, repeat: bool },
    Escape,
    FocusLost,
    /// Time passed; the driver sends it at `deadline()`.
    Tick,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LeaderEffect { Open, Close, Run(ItemId) }

pub struct LeaderMachine { /* mode, state Idle|Held|Armed, last event Instant */ }
impl LeaderMachine {
    pub fn new(mode: LeaderMode) -> LeaderMachine;
    pub fn set_mode(&mut self, mode: LeaderMode);  // closes an open state: returns nothing, the driver closes the window
    /// `bound`: the item of a `Leader+…` hotkey, if any.
    pub fn on(&mut self, input: LeaderInput, now: Instant, bound: &dyn Fn(&Hotkey) -> Option<ItemId>) -> Vec<LeaderEffect>;
    /// When to send `Tick`; `None` while idle.
    pub fn deadline(&self) -> Option<Instant>;
    pub fn is_open(&self) -> bool;
}

/// A window key-down as a `Leader+…` hotkey: the leader's own modifiers taken off, the key mapped to
/// its US name (`keys::gpui_alias`, then `keys::us_key` when it is not supported), `LEADER` added.
/// `None` when the key is not supported.
pub fn window_key(pressed: &Hotkey, leader: &Hotkey) -> Option<Hotkey>;

/// The recorders' "leader down" state.
#[derive(Default)]
pub struct LeaderRecording { /* down: bool */ }
impl LeaderRecording {
    pub fn pressed(&mut self);
    pub fn released(&mut self, mode: LeaderMode);   // Hold: up; Tap: stays down until a key
    /// The hotkey a recorder shows for a key-down `pressed`. If the leader is down, it is `window_key`
    /// (and in tap mode the state goes up). With a modifiers-only leader in hold mode, a press whose
    /// mods contain all the leader's mods becomes `Leader+<rest>`. Otherwise `pressed` unchanged.
    pub fn key(&mut self, pressed: &Hotkey, leader: Option<&Hotkey>, mode: LeaderMode) -> Hotkey;
    pub fn is_down(&self) -> bool;
}
```

`keys::us_key(key: &str) -> Option<String>`: a one-character key not in the table is read through `layout::LAYOUTS` (`Layout::to_latin` on it, layouts in priority order). The first result that is a supported key is returned, lowercased.

State tables, exactly as in the spec ("Hold mode" and "Tap mode"), plus:
- Esc closes in both modes.
- In tap mode `Released` is ignored.
- `Pressed` while `Held` (repeat) is ignored. `Pressed` while `Armed` closes.
- `Key` with `repeat: true` does nothing.
- `Tick` at or after `last event + SAFETY_CLOSE` closes. Every input except `Tick` updates `last event`.

- [ ] **Step 1: Write failing tests** (`t0` is an `Instant`, and `b` binds `Leader+F` → `apps/ff`):

```rust
#[test] fn hold_runs_each_key_until_release() {
    let mut m = LeaderMachine::new(LeaderMode::Hold);
    assert_eq!(m.on(LeaderInput::Pressed, t0, &b), [LeaderEffect::Open]);
    assert_eq!(m.on(key("leader+f"), t0, &b), [LeaderEffect::Run(id("apps/ff"))]);
    assert_eq!(m.on(key("leader+f"), t0, &b), [LeaderEffect::Run(id("apps/ff"))]);
    assert_eq!(m.on(key("leader+x"), t0, &b), []);
    assert_eq!(m.on(LeaderInput::Released, t0, &b), [LeaderEffect::Close]);
    assert!(!m.is_open());
}
#[test] fn auto_repeat_runs_once() { /* Pressed, key f, key f repeat:true → one Run */ }
#[test] fn release_before_open_closes_at_once() { /* Pressed then Released → [Open], [Close] */ }
#[test] fn tap_runs_one_key_and_closes() { /* Pressed→Open; Released→[]; key f → [Run, Close] */ }
#[test] fn tap_unknown_key_or_escape_closes() { /* key x → [Close]; Escape → [Close] */ }
#[test] fn tap_leader_again_closes() { /* Pressed, Pressed → [Open], [Close] */ }
#[test] fn safety_timeout_closes_held() {
    // Pressed at t0; deadline() == Some(t0 + SAFETY_CLOSE); Tick at t0+4s → []; Tick at t0+5s → [Close]
}
#[test] fn safety_timeout_closes_armed() { /* same in Tap */ }
#[test] fn focus_lost_closes() { /* Hold and Tap */ }
#[test] fn russian_layout_keys_map_to_us() {
    let l = Hotkey::parse_leader("F12").unwrap();
    assert_eq!(window_key(&hk("а"), &l), Some(hk("leader+f")));
    assert_eq!(window_key(&hk("shift+ф"), &l), Some(hk("leader+shift+a")));
    assert_eq!(window_key(&hk("."), &l), Some(hk("leader+.")));          // US "." stays
    assert_eq!(window_key(&hk("ctrl+f"), &Hotkey::parse_leader("Ctrl+Space").unwrap()), Some(hk("leader+f")));
    assert_eq!(window_key(&hk("€"), &l), None);
}
#[test] fn recording_follows_the_mode() {
    let mut r = LeaderRecording::default();
    let l = Hotkey::parse_leader("F12").unwrap();
    r.pressed();
    assert_eq!(r.key(&hk("f"), Some(&l), LeaderMode::Hold), hk("leader+f"));
    r.released(LeaderMode::Hold);
    assert_eq!(r.key(&hk("ctrl+alt+t"), Some(&l), LeaderMode::Hold), hk("ctrl+alt+t"));
    r.pressed(); r.released(LeaderMode::Tap);
    assert_eq!(r.key(&hk("f"), Some(&l), LeaderMode::Tap), hk("leader+f"));
    assert!(!r.is_down());
    let ctrl = Hotkey::parse_leader("Ctrl").unwrap();
    assert_eq!(r.key(&hk("ctrl+shift+f"), Some(&ctrl), LeaderMode::Hold), hk("leader+shift+f"));
    assert_eq!(r.key(&hk("ctrl+f"), Some(&ctrl), LeaderMode::Tap), hk("ctrl+f"));
}
```

`hk("а")` needs `Hotkey::parse` to accept a Cyrillic one-character key. It already does: a key is any last part, lowercased.

- [ ] **Step 2: Run** `cargo test -p tomakast-core leader`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p tomakast-core && cargo clippy -p tomakast-core --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "core: leader state machine, window keys through the layout tables, recording state"`

### Task 4: Backends report releases and bind the leader

**Files:**
- Modify: `crates/platform/src/hotkeys.rs`, `crates/platform/src/hotkeys/kde.rs`, `native.rs`, `linux.rs`, `null.rs`
- Test: in-file tests (fakes already exist)

**Interfaces:**
- Consumes: Task 1 (`qt_code` of a lone modifier, `is_modifiers_only`).
- Produces:
  - `HotkeyEvent::Released(String)` (logical id).
  - `SetKind::Leader`: one binding, id `"leader"`.
  - `HotkeyService::grabs_keyboard_while_held(&self) -> bool`: `true` only for the `global-hotkey` backend on Linux X11 (`XGrabKey`), else `false`. It is a field set in `start_backend`.
  - Modifiers-only bindings:
    - KDE: bound with `setShortcutKeys(…, flags 6)`, like any other, through `qt_code`. The `globalShortcutsByKey` conflict check stays (it sees Plasma's Meta).
    - native and portal: answered `BindState::Refused { reason: "a modifier alone is not available on this system" }`.

- [ ] **Step 1: Write failing tests**
  - `native.rs`: `a_release_is_forwarded`. Feed the handler-closure logic, extracted as `fn event_of(state: HotKeyState, id: String) -> HotkeyEvent`: Pressed → `Activated`, Released → `Released`.
  - `native.rs`: `a_modifier_alone_is_refused`. Set a binding `Hotkey::parse_leader("Ctrl")`; the `Applied` state is `Refused` with that reason, and nothing is registered with the fake OS.
  - `kde.rs`: `signals_map_to_events`. Extract `fn event_of(member: &str, comp: &str, action: String, own: &str) -> Option<HotkeyEvent>`: `globalShortcutPressed` → `Activated`, `globalShortcutReleased` → `Released`, `globalShortcutRepeated` → `None`, other component → `None`.
  - `kde.rs`: `keys_of_a_lone_ctrl`. `keys_of(&Hotkey::parse_leader("Ctrl").unwrap())` → `Ok(vec![(vec![0x0100_0021],)])`.
  - `linux.rs`: `deactivated_is_a_release` (pure mapping fn).
- [ ] **Step 2: Run** `cargo test -p tomakast-platform`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - KDE: one match rule per member, or a rule on the interface filtered by member. Same thread.
  - native: forward `Released`.
  - portal: subscribe to `Deactivated` next to `Activated`.
  - null: nothing new.
- [ ] **Step 4: Run** `cargo test -p tomakast-platform && cargo clippy -p tomakast-platform --all-targets -- -D warnings && cargo clippy -p tomakast-platform --all-targets --target aarch64-apple-darwin -- -D warnings && cargo clippy -p tomakast-platform --all-targets --target x86_64-pc-windows-gnu -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "platform: hotkey releases, the leader set, a lone modifier on KDE"`

### Task 5: The invisible leader window

**Files:**
- Create: `crates/ui/src/leader_window.rs`
- Modify: `crates/ui/src/window.rs` (share `options`/`kind` with a transparent, no-blur, no-inset variant), `crates/ui/src/lib.rs` (exports)
- Test: `crates/ui/tests/leader_window.rs` (headless)

**Interfaces:**
- Produces:

```rust
pub enum CaptureEvent {
    /// A key-down as a hotkey (the recorder's `hotkey_of` with `keys::gpui_alias`); `None`: a
    /// modifier alone or a key with no name.
    Key { hotkey: Option<Hotkey>, repeat: bool },
    Escape,
    FocusLost,
}
pub type CaptureSink = Rc<dyn Fn(CaptureEvent, &mut App)>;

/// Global. Owns at most one leader window.
pub struct LeaderWindow { .. }
impl Global for LeaderWindow {}
impl LeaderWindow {
    pub fn new() -> LeaderWindow;
    /// Opens the 1×1 transparent window that takes the keyboard; no-op when open.
    /// Under `TOMAKAST_NO_KEYBOARD=1` the window opens without the keyboard (tests rely on it).
    pub fn open(&mut self, sink: CaptureSink, cx: &mut App);
    pub fn close(&mut self, cx: &mut App);
    pub fn is_open(&self) -> bool;
}
```

- Window: size 1×1, `WindowBackgroundAppearance::Transparent`, no `set_client_inset`, input region an empty rectangle, centred.
  - Wayland: layer `Overlay`, namespace `utility`, `KeyboardInteractivity::Exclusive` (`None` under `TOMAKAST_NO_KEYBOARD=1`).
  - Elsewhere: `WindowKind::PopUp` with `focus: true`.
- The view tracks a focus handle, focused at open, and renders an empty `div`.
  - `on_key_down`: `escape` with no modifiers → `Escape`; otherwise `Key { hotkey, repeat: event.is_held }`.
  - Window deactivation after having been active (the `launcher_view.rs:183` pattern) → `FocusLost`.
- Log `debug!(ms, "leader window focused")` on the first activation, measured from `open`. The acceptance note reads it.

- [ ] **Step 1: Write failing tests** in `crates/ui/tests/leader_window.rs`:
  - `keys_reach_the_sink`. Open with a recording sink, `simulate_keystrokes("f")` on its window: one `Key { hotkey: Some(f), repeat: false }`. Then `"escape"`: `Escape`.
  - `close_removes_the_window`. After `close`, `cx.windows()` no longer holds it and `is_open()` is false.
  - `a_second_open_is_a_no_op`.
- [ ] **Step 2: Run** `cargo test -p tomakast-ui --test leader_window`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p tomakast-ui && cargo clippy -p tomakast-ui --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "ui: the invisible leader window"`

### Task 6: The daemon's hotkeys know the leader

**Files:**
- Modify: `crates/app/src/hotkeys.rs`
- Test: its test module (fake `Registrar` exists)

**Interfaces:**
- Consumes: Tasks 1, 2, 4.
- Produces:
  - `pub(crate) const LEADER_ID: &str = "leader";`
  - `Hotkeys::new(registrar, config_file, grabs_keyboard_while_held: bool)`. It remembers whether a key leader acts as tap here.
  - `Hotkeys::effective_mode(&self) -> LeaderMode`: the file's mode, or `Tap` when `grabs_keyboard_while_held` and the leader has a key.
  - `Hotkeys::leader(&self) -> Option<&Hotkey>`.
  - `Hotkeys::leader_item(&self, hotkey: &Hotkey) -> Option<ItemId>`: the item of a `Leader+…` hotkey in the applied item set.
  - `Outcome::Released { id: String }`.
- Behaviour:
  - **Item set as sent:** `applied[Items]` keeps the logical bindings (`Leader+F`), and `send` for `Items` maps each through `system_form(&b.hotkey, &keyspace)`. Bindings whose form is `None` are not sent.
  - **Leader set:** one binding (`LEADER_ID`, description `"tomakast leader key"`, the leader hotkey) when a leader is set and it is needed: a key leader (any mode), or a modifiers-only leader in tap mode. Otherwise empty. Bound by `load` and `follow_file` like the launcher set. A change of `leader` or `leader_mode` in the file also re-sends the item set, because its system forms change.
  - **States in `table()`** for a `Leader+…` item not sent to the system: `NoLeader` when there is no leader, else the leader binding's state (`Active` if absent). `HotkeyTable.leader`, `leader_state` and `leader_mode` (effective) are filled in.
  - **`request(HotkeyTarget::Leader, Some(h) | None, …)`:**
    - `check_leader` against the file;
    - a modifiers-only leader in effective tap mode on a backend that refuses it is answered by the backend (Task 4), and the pending change is refused through the existing `answer` path;
    - bind first: send the new leader set and, when system forms change, the item set;
    - on acceptance, write `set_key(&text, &["launcher","leader"], Some(h.to_string().into()))`, or `None` to remove the key.
  - **`save`** re-checks with `check_leader` against the file as it is now.
  - **`on_event(HotkeyEvent::Released(id))`** → `Outcome::Released { id }` when `id` is bound. **`Activated("leader")`** → `Outcome::Pressed { id: "leader", hotkey: leader }`.

- [ ] **Step 1: Write failing tests** (existing helpers `setup`, `load`, `last`, `applied`):
  - `a_key_leader_binds_the_leader_set_and_keeps_leader_items_off_the_system`. Config `[launcher] leader = "F12"` with `[hotkeys] "Leader+F" = "apps/ff.desktop"`. The leader set is sent with `("leader","F12")`. The item set is sent without `apps/ff.desktop`. `table().of(ff).state` is `Active`.
  - `a_modifier_leader_in_hold_mode_registers_ctrl_f`. Leader `"Ctrl"`: no leader set is sent (or it is empty), and the item set holds `("apps/ff.desktop","Ctrl+F")`.
  - `a_modifier_leader_in_tap_mode_binds_the_lone_modifier`. Leader set `("leader","Ctrl")`; item set without ff.
  - `without_a_leader_leader_items_show_no_leader`. State `NoLeader`.
  - `a_leader_change_is_written_after_acceptance`. `request(Leader, Some(F11))`, answered `Active`: the file has `leader = "F11"`, comments kept.
  - `refused_leader_change_keeps_the_old_leader`. Answered `Refused`: the file is unchanged and the leader set is re-sent as before, or kept (follow the existing `answer` rules for refused ids).
  - `leader_change_clash_is_refused_before_binding`. Config with `Leader+F` and `Ctrl+F`; `request(Leader, Some(Ctrl))` → `Err(Refusal::LeaderClash{..})`, and nothing is sent.
  - `x11_key_leader_acts_as_tap`. `Hotkeys::new(…, true)` with leader F12 in hold mode → `effective_mode() == Tap`; with leader Ctrl → `Hold`.
  - `releases_are_reported`. `on_event(Released("leader"))` → `[Outcome::Released { id: "leader" }]`.
- [ ] **Step 2: Run** `cargo test -p tomakast-app hotkeys`. Expected: FAIL.
- [ ] **Step 3: Implement.** Remove the Task 2 placeholders marked `// Task 6`.
- [ ] **Step 4: Run** `cargo test -p tomakast-app && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "app: the leader set, Leader+key hotkeys in the item set, leader changes bound first and written after"`

### Task 7: Daemon wiring: the leader runs items

**Files:**
- Create: `crates/app/src/leader.rs`
- Modify: `crates/app/src/daemon.rs`, `crates/app/src/lib.rs`
- Test: `crates/app/src/leader.rs` tests (driver logic with a fake window and clock)

**Interfaces:**
- Consumes: Task 3 (`LeaderMachine`, `window_key`), Task 5 (`LeaderWindow`, `CaptureEvent`), Task 6 (`Hotkeys::leader`, `leader_item`, `effective_mode`, `Outcome::Released`).
- Produces: `pub(crate) struct LeaderDriver`, a global on the UI thread:

```rust
impl LeaderDriver {
    pub fn new() -> LeaderDriver;
    /// After each hotkey load: the leader, the effective mode and a snapshot of Leader+… → item.
    pub fn configure(&mut self, leader: Option<Hotkey>, mode: LeaderMode, items: Vec<(Hotkey, ItemId)>);
    /// Runs the machine and carries out its effects: Open/Close the window, Run(item) via `run`.
    pub fn input(&mut self, input: LeaderInput, now: Instant, cx: &mut App, run: &dyn Fn(&ItemId, &mut App));
}
```

  The window side is behind a small trait (`open`, `close`) so tests use a fake. Production uses `LeaderWindow`. After every `input`, if `deadline()` is `Some(d)`, spawn a timer to `d` that sends `Tick` when its generation is still current.
- Daemon routing (`apply_outcomes`):
  - `Pressed { id: "leader" }`:
    - if the settings window or the launcher is recording → `leader_pressed(cx)` on that controller (Task 8);
    - otherwise, unless `TOMAKAST_NO_KEYBOARD=1` → `LeaderDriver::input(Pressed)`.
  - `Released { id: "leader" }` → the same routing with `leader_released` / `Released`.
  - Capture sink: `CaptureEvent::Key { hotkey, repeat }` → `LeaderInput::Key { hotkey: hotkey.and_then(|h| window_key(&h, &leader)), repeat }`; `Escape` → `Escape`; `FocusLost` → `FocusLost`.
  - `run` = the existing `run_item`.
  - The driver is re-`configure`d whenever an `Outcome::Table` is applied, from the `HotkeyTable` (leader, `leader_mode`) and the items whose hotkey has `LEADER`.

- [ ] **Step 1: Write failing tests**
  - `press_opens_key_runs_release_closes`. Fake window counts open/close; `run` records items.
  - `a_tick_after_five_seconds_closes`.
  - `reconfigure_while_open_closes`. `configure` with another mode while open → closed.
- [ ] **Step 2: Run** `cargo test -p tomakast-app leader`. Expected: FAIL.
- [ ] **Step 3: Implement** the driver and the daemon wiring.
- [ ] **Step 4: Run** `cargo test -p tomakast-app && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "app: the leader opens the invisible window and runs Leader+key items"`

### Task 8: Recording, rows and the settings window

**Files:**
- Modify: `crates/ui/src/views/hotkey_recorder.rs`, `crates/ui/src/launcher_view.rs`, `crates/ui/src/window.rs` (`LauncherController::leader_pressed/leader_released`), `crates/ui/src/settings/{mod.rs,window.rs,fields.rs}`, `crates/ui/src/settings/pages/{general.rs,hotkeys.rs}`, `crates/ui/src/hotkeys.rs`, `crates/ui/src/shortcut.rs`, `modules/settings/src/lib.rs`
- Test: `crates/ui/tests/hotkeys.rs`, `crates/ui/tests/settings.rs`, `modules/settings` tests

**Interfaces:**
- Consumes: Task 2 (table fields, `HotkeyTarget::Leader`, refusals), Task 3 (`LeaderRecording`).
- Produces:
  - `LauncherController::leader_pressed(&mut self, cx: &mut App)` and `leader_released(&mut self, cx: &mut App)`, with the same on `SettingsController`. They forward to the open recorder or field.
  - **Launcher recorder:** holds a `LeaderRecording`. `press` turns `hotkey_of(keystroke)` into `recording.key(&h, table.leader.as_ref(), table.leader_mode)` before `show`. While the leader is down and nothing is shown yet, the shown area reads `"Leader + …"`.
  - **Settings `HotkeyField`:** the same for item fields and the launcher field. A new constructor flag `leader_field: bool` (the leader's own field):
    - accepts any key with or without modifiers;
    - records a modifiers-only leader when modifiers are released with no key between (handle `ModifiersChangedEvent`: remember the largest set held, and record it when all are released if no key-down came);
    - its check is `table.check_target(&HotkeyTarget::Leader, h)`.
  - **Settings pages:** General and Hotkeys show "Leader key" (a leader `HotkeyField`, requests through `request_hotkey(HotkeyTarget::Leader, …)`) and "Leader mode" (a choice bound to `["launcher","leader_mode"]` through the existing generic choice control for schema keys). Hotkeys page rows with `Leader+…` hotkeys show the `Leader` chip and the `NoLeader` state.
  - **Chips:** `shortcut::chips(&Hotkey)` yields `"Leader"` first for `LEADER`, and a modifiers-only hotkey yields only modifier chips. `hotkeys::label` gives `NoLeader` the text `"no leader key"`.
  - **Module:** `modules/settings` adds item `settings/leader`, title `"Set Leader Key…"`, subtitle `"Hold it and press a key to run that key's hotkey"`, keywords `["leader","hotkey","shortcut","keyboard"]`, `ItemKind::Command`, action `open` → `Effect::OpenSettings(Some("hotkeys".into()))`. Use the Hotkeys page id the settings window already uses; check `settings/pages/mod.rs`.

- [ ] **Step 1: Write failing tests**
  - `crates/ui/tests/hotkeys.rs`, `leader_down_records_leader_plus_key`. Open the recorder on a row, with a table whose leader is `F12` (hold). `controller.leader_pressed`, `simulate_keystrokes("f")`, Enter: `FakeRequests` got `(item, Some(Leader+F))`.
  - `crates/ui/tests/hotkeys.rs`, `a_ctrl_leader_records_ctrl_f_as_leader_f`. Leader `Ctrl`, hold mode, keystroke `ctrl-f` → `Leader+F`.
  - `crates/ui/tests/hotkeys.rs`, `leader_rows_show_the_leader_chip`. A row whose hotkey is `Leader+F` shows chips `Leader`, `F`. With `NoLeader` its label contains `"no leader key"`.
  - `crates/ui/tests/settings.rs`, `the_leader_field_records_a_lone_modifier`. Focus the Leader key field, press and release Ctrl with no key, Enter: request `(Leader, Some(Ctrl))`.
  - `crates/ui/tests/settings.rs`, `the_leader_field_refuses_the_launcher_hotkey`. Recording `alt-space` shows `"That is the launcher hotkey."`.
  - `crates/ui/tests/settings.rs`, `the_leader_mode_is_a_choice`. The General page lists "Leader mode" with both options.
  - `modules/settings`, `leader_item_opens_the_hotkeys_page`.
- [ ] **Step 2: Run** `cargo test -p tomakast-ui --test hotkeys --test settings && cargo test -p tomakast-module-settings` (use the crate name from `modules/settings/Cargo.toml`). Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "ui: record Leader+key and the leader, Leader chips, the leader in the settings window"`

### Task 9: Acceptance note and full check

**Files:**
- Create: `docs/experiments/2026-11-leader-key-acceptance.md`

- [ ] **Step 1: Run** `just check`. Expected: exit 0. Fix anything it finds in the tasks' code.
- [ ] **Step 2: Run** the headless daemon smoke test: `TOMAKAST_NO_KEYBOARD=1` with a throwaway `TOMAKAST_HOME`, the way existing acceptance notes do it (see `docs/experiments/2026-11-item-hotkeys-acceptance.md`). Use a config with `leader = "F12"` and a `Leader+F` entry. In `just log`, the leader set is bound by the null backend and no window opens.
- [ ] **Step 3: Write the note.** Sections:
  - What was checked automatically (test names per spec section).
  - The headless run.
  - **The user's manual checklist on KDE** (`just try`, then set the leader in Settings → Hotkeys):
    1. **The release check, first.** Leader F12 in hold mode, `Leader+F` on Firefox. Hold F12, press F: Firefox starts. Release F12, type `f` in an editor: the letter appears, so the window closed on release. If the letter is lost for up to 5 s, KDE did not report the release; note it.
    2. Hold F12, press F then T: both items start. Holding F starts Firefox once.
    3. A non-Latin layout active (Russian, or any other you use): hold F12, press the key where F is on US: Firefox starts.
    4. Leader Ctrl, hold mode: `Leader+F` works as Ctrl+F everywhere. KDE names a holder if Ctrl+F is taken by another global shortcut.
    5. Tap mode: `leader_mode = "tap"`. Press and release F12, then F: Firefox starts. Press F12 and wait 5 s: typing works again. Press F12, then Esc: closed. Leader Ctrl in tap mode: tap Ctrl, then F.
    6. Change the leader from F12 to Ctrl while `Ctrl+F` is set on another row: refused, with the message naming the row.
    7. Remove the leader: `Leader+F` rows show "no leader key".
    8. `just log`: `leader window focused ms=…` lines, all under 50.
    9. Ctrl+C `just try`: KDE's shortcut settings keep no tomakast entries, the leader included.
  - **macOS / Windows (manual, not run here):** steps 1–3 and 5. Expectation: the window takes focus; on Windows the foreground rule after `WM_HOTKEY`.
  - **Known limits:**
    - X11: a key leader acts as tap.
    - A modifier leader in tap mode is KDE-only.
    - Keys pressed within the first ~20 ms after the leader go to the previous program.
- [ ] **Step 4: Commit** `git commit -m "docs: leader key acceptance"`
