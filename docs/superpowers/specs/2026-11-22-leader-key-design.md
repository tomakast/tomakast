# Leader key: a user-chosen key that turns any key into a hotkey

Status: approved by the user on 2026-11-23.

## Intent

Global hotkeys collide: the system and every program already use most Ctrl/Alt/Super
combinations. The user wants one key of their own choosing, the **leader**, inside tomakast.
Item hotkeys can then be "Leader+F": hold the leader, press F, and the item runs. F alone stays
an ordinary letter everywhere, so the leader opens a namespace of hotkeys that conflicts with
nothing. The user's words: the leader "can be a single key or a combination", and hotkeys are
then created with it "so as not to conflict with hotkeys in the system and in programs".

Success: the user sets the leader once (for example F12, Menu, Pause or Ctrl), assigns
"Leader+F" to Firefox from Ctrl+K like any item hotkey, and from then on holding the leader and
pressing F starts Firefox from any program, on any keyboard layout, with nothing visible on
screen.

## Decisions

The user's:

1. **The leader is chosen in tomakast** and can be any single key or any combination: a plain
   key (F12, Pause, Menu), a combination (Ctrl+Space), or a modifier alone (Ctrl). tomakast
   does not forbid any of them. With a modifier leader in hold mode `Leader+F` is the same keys
   as `Ctrl+F` and takes that combination from programs; the user accepted this ("if there are
   no programs with Ctrl+F, why not"). No warning is added for it.
2. **The leader is held, like a modifier**: hold it, press F. The default mode.
3. **An accessibility setting** makes the leader "press and release, then press F". It needs a
   clear way out of the leader state (below).
4. **Nothing is shown.** Between the leader and the key tomakast reads keys through an
   invisible window. No indicator, no hint, in either mode.
5. **The hint popup is out of scope.** A which-key style list of the next keys after a delay
   was liked and passed to the main session for the backlog.

The author's calls are marked as such where they are made; the user did not see them before
this document.

## Goals

- `[launcher] leader` and `[launcher] leader_mode` in `config.toml`; hand edits apply live.
- A launcher command "Set Leader Key…" that opens the settings window's Hotkeys page, where
  the leader field records the leader (planning ruling, 2026-11-23: the settings window already
  has the launcher-hotkey field with bind-first/write-after; a second recorder in the launcher
  for a non-item target would duplicate it).
- Item hotkeys of the form `Leader+<key>` (with optional extra modifiers, `Leader+Shift+F`),
  assigned from Ctrl+K → "Assign Hotkey…" by holding the leader and pressing the key, and
  written to `[hotkeys]` next to ordinary ones.
- Running them from any program while the leader is held (hold mode) or after it was pressed
  (tap mode), on KDE Wayland, macOS, Windows and X11; GNOME and other portal desktops after a
  restart, as item hotkeys there already do.
- Layout independence: the physical key counts, on any non-Latin layout (Russian, Greek,
  Hebrew, Thai, …); on the Russian layout `А` is `F`.

## Non-goals

- The hint popup (decision 5).
- Sequences of several keys after the leader (`Leader, b, f`). One key per hotkey.
- Telling left from right modifiers. KDE Wayland merges them before any program sees the key
  (`docs/experiments/2026-11-22-side-modifiers-research.md` on branch
  `claude/side-modifiers-research-6a951b`); a user who wants "Right Ctrl" maps it with keyd to a
  spare key and makes that the leader.
- Hold mode with a key leader on X11. `global-hotkey` binds with `XGrabKey`, and the X server
  gives the grabbing connection the whole keyboard while the grabbed key is down, so the
  invisible window would not get F. On X11 a key leader therefore behaves as in tap mode
  (author's call; X11 is not a first-class target). A modifier leader in hold mode works on
  X11 (it is plain registration).
- A modifier alone as the leader in **tap** mode on macOS and Windows: it needs a key monitor
  those backends do not have (the research's "bare tap" rows). On KDE it is native. Later work.

## Anti-goals

- A stuck invisible window. If the release of the leader is missed for any reason, the user
  must never be left unable to type: the window always closes by itself (Safety, below).
- Keys leaking into the program the user is in: once the invisible window has the keyboard, a
  key pressed under the leader reaches tomakast only.
- Reading keys system-wide. tomakast hosts plugins treated as hostile; no event tap, keyboard
  hook or `/dev/input` reader enters the daemon for this.
- Slowing the launcher's own open path.

## Constraints

- Window open stays ~50 ms (a warm open is ~18 ms today). The invisible window rides the same
  prewarmed renderer.
- `TOMAKAST_NO_KEYBOARD=1` binds nothing and opens no keyboard-taking window.
- `tomakast-platform` stays free of GPUI.
- Other sessions work in parallel: the settings window (its hotkeys page), file search, the
  translator. Changes stay in the hotkey, settings, platform and recorder areas.

## Approaches considered

1. **The leader is a global shortcut; while it is down an invisible window takes the keyboard
   and reads the next key (chosen).** Every backend already reports the press, and they also
   report the release: KDE's kglobalaccel has `globalShortcutReleased` (seen on this machine's
   session bus), `global-hotkey` 0.10 reports `HotKeyState::Released` on macOS (Carbon
   `kEventHotKeyReleased`), Windows and X11, and the GlobalShortcuts portal has `Deactivated`.
   No new permission anywhere. Cost: one small window per leader press, and the first ~20 ms
   after the press belong to the previous program (a key pressed that fast is lost to it).
2. **Register every Leader+key as a system hotkey.** Only possible when the leader is made of
   modifiers (Ctrl+F is a normal hotkey); the system APIs take modifiers plus one key, never two
   ordinary keys (F12+F). Used for modifier leaders as part of approach 1, not alone.
3. **Read every key system-wide** (macOS event tap, Windows low-level hook, evdev on Linux).
   Permissions on macOS, root or the `input` group on Wayland, and a keylogger inside the
   process that runs hostile plugins. Rejected.
4. **While the leader is held, register the plain keys as temporary global shortcuts.** A D-Bus
   round trip per key per press, and a crash would leave letters grabbed system-wide. Rejected.

## Design

### Two kinds of leader (author's call)

The behaviour is the same for the user; the mechanism depends on what the leader is.

- **Key leader**: the leader has an ordinary key (F12, Pause, Menu, Insert, F13–F24, a letter,
  Ctrl+Space). The leader alone is registered as a global shortcut, in a new third set next to
  the launcher's and the items'. Its press opens the invisible window; keys read there are
  looked up among the `Leader+…` hotkeys.
- **Modifier leader**: the leader is modifiers only (Ctrl, Alt, Ctrl+Alt, Super). A held
  modifier cannot be seen on KDE (kglobalaccel fires a lone modifier only on its release, with
  no other key between), and none is needed: `Leader+F` with leader Ctrl *is* Ctrl+F. In hold
  mode each `Leader+X` is registered directly as the system hotkey `<leader mods>+X` in the
  item set; no window opens. Conflicts with other programs are reported exactly as for item
  hotkeys today (on KDE the holder is named; elsewhere registration fails).
  - In tap mode a modifier leader is the lone-modifier shortcut (KDE, native since Plasma 6.1,
    reliable from 6.8: the research above). Its activation opens the invisible window as for a
    key leader. On macOS and Windows tap mode with a modifier leader is reported as
    unavailable (non-goal).

### Hold mode (default)

State machine, pure and unit-tested (`core`), driven by the daemon on the UI thread:

| State | Event | Action → next state |
|---|---|---|
| Idle | leader pressed | open the invisible window → Held |
| Held | key K (+mods) in the window | if `Leader+<mods>+K` is assigned, run it; else nothing → Held |
| Held | leader released | close the window → Idle |
| Held | the window lost the keyboard | close → Idle |
| Held | 5 s without a key event (safety) | close → Idle |
| Held | leader pressed again (auto-repeat) | ignore |

- Several keys per hold run several items (author's call): hold F12, press F then T.
- Key auto-repeat inside the window is ignored: holding F runs Firefox once.
- The leader released before the window has opened: the window opens and closes at once
  (no key can have reached it).
- Pressing the leader alone and releasing it does nothing (author's call).

### Tap mode (`leader_mode = "tap"`, accessibility)

| State | Event | Action → next state |
|---|---|---|
| Idle | leader pressed (key leader) or lone-modifier tap (modifier leader) | open the invisible window → Armed |
| Armed | key K (+mods) | run `Leader+<mods>+K` if assigned; close → Idle |
| Armed | Esc, or a key with no hotkey | close → Idle (the key is swallowed) |
| Armed | leader pressed again | close → Idle |
| Armed | 5 s without a key | close → Idle |
| Armed | leader released / modifiers changed | ignore |

Exits (author's call, from the conversation): after the first key, on Esc, on the leader again,
after 5 s. The window stays invisible (decision 4).

### The invisible window (UI)

- Opened through the same window code as the launcher, not the launcher controller's window: a
  1×1 transparent surface with no blur, no shadow inset and an empty input region (clicks go
  through), taking the keyboard:
  - Wayland: a layer-shell `Overlay` surface, `KeyboardInteractivity::Exclusive`, namespace
    `utility` (no KWin animation);
  - macOS: GPUI's non-activating pop-up panel, made key;
  - Windows and X11: a topmost pop-up that takes focus. On Windows the process that received
    `WM_HOTKEY` may take the foreground.
- It is the shape of the existing prewarm window, so the renderer is already warm.
- Its view does nothing but forward each key-down (with the GPUI `Keystroke`) and focus loss to
  the leader state machine.
- If the launcher is open when the leader is pressed, it loses the keyboard and hides, as it
  does on any focus loss (author's call: nothing new to manage).
- Open-to-focus time is measured in the acceptance run (log line from the leader press to the
  window's first key-focus), with a target under 50 ms.

### Keys and layouts

- A key read in the window is turned into a `Hotkey` by the recorder's existing `hotkey_of`
  (shifted symbol → Shift + base key).
- GPUI names a key of a layout without ASCII letters by its ASCII equivalent on every platform
  (its `Keystroke::key` contract; on Linux `guess_ascii`, by physical key code), so the letters
  of any non-Latin layout already arrive as US keys. On Linux it also names
  the digit row by the US one with Ctrl/Alt/Super (the tomakast patch in
  `third_party/gpui-pre-linux`). A key that still arrives as a layout character (Russian `ю`, `№`) is
  mapped to the US key through `tomakast_core::layout` tables (`Layout::to_latin`), trying the
  layouts in their priority order; a shifted US symbol becomes Shift + its base key (`№` →
  Shift+3). A key that maps to nothing supported is "no hotkey".
- Known limit (implementation ruling): ASCII punctuation that a layout moves to another key
  keeps its typed name, because the active layout is not known here. On Russian the `/` key
  types `.` and gives `Leader+.`, like the `ю` key. Letters are always by physical key.
- The leader's own modifiers are taken off a key read in the window: with leader Ctrl+Space, a
  window key of Ctrl+F is `Leader+F`.

### Settings

`config.toml`:

```toml
[launcher]
hotkey = "Alt+Space"
leader = "F12"          # a key, a combination, or modifiers only ("Ctrl")
leader_mode = "hold"    # "hold" (default) or "tap"

[hotkeys]
"Leader+F" = "apps/firefox.desktop"
"Leader+Shift+F" = "apps/firefox-private.desktop"
"Ctrl+Alt+T" = "apps/kitty.desktop"
```

- No leader set: no leader is bound, and `Leader+…` entries are kept in the file but inactive
  (rows show them as "off" with the reason "no leader key set").
- `Hotkey` gains a `LEADER` modifier bit: `Leader+F` parses, displays and compares like any
  hotkey. `Leader` counts as the modifier a hotkey needs (the "Add Ctrl, Alt or Super" rule does
  not apply to it).
- The leader itself is a new type: either a `Hotkey` with a key, or a non-empty set of
  modifiers. Parsing accepts `"Ctrl"`, `"Ctrl+Alt"`, `"F12"`, `"Ctrl+Space"`. The key table in
  `crates/core/src/keys.rs` gains `pause`, `scrolllock`, `menu` and `f13`–`f24`, with their xkb,
  Qt and `global-hotkey` names; modifiers alone get their Qt key codes (`Key_Control`
  `0x01000021` etc., as the research probed).
- Validation (author's calls):
  - the leader may not equal the launcher hotkey;
  - a `Leader+…` hotkey may not equal another item's (existing `TakenBy` rule);
  - with a modifier leader **in hold mode**, `Leader+F` and an explicit `Ctrl+F` are the same
    keys and the second is refused like any duplicate. In tap mode they are different gestures
    (tap Ctrl, then F; or Ctrl+F) and both may exist. The validator takes the leader and the
    mode as inputs; after a hand edit of either, the file is re-validated with the existing
    "first entry wins" rule, and dropped entries are logged and shown as refused.
  - likewise with a combination leader (Ctrl+Space) in hold mode: holding it and pressing F
    sends Ctrl+F, so `Leader+F` and `Ctrl+F` are the same keys (implementation ruling). Only
    `Ctrl+F` is registered; the window reads `Leader+F`. In tap mode both may exist.
  - a hand-edited leader equal to the launcher hotkey is not bound; the leader shows as
    refused ("same as the launcher hotkey") until the file is fixed (implementation ruling).
- Write-back: through the generic comment-preserving `set_key` editor and atomic
  `write_config` (from the settings-window branch, see Base branch). `leader_mode` is edited by
  hand (and later by the settings window).
- The daemon's config watcher already reloads hotkeys on edit; a leader or mode change rebinds
  the leader set and, for a modifier leader, the item set.

### Changing the leader

Hotkeys are stored as `Leader+F`, never as the leader's current keys, so every `Leader+…`
hotkey follows a new leader with nothing rewritten. What a change can break, and what happens:

- **Two of tomakast's own hotkeys become the same keys.** Example: leader F12 → Ctrl in hold
  mode, while `Leader+F` and `Ctrl+F` are both assigned. The leader recorder refuses the new
  leader before Enter and names the clash ("With Ctrl, Leader+F is Ctrl+F, already set on
  Kitty"), like any item clash today (refuse, not reassign). A hand edit that causes it keeps
  the first entry, as the file rules do now.
- **The leader is the key of one of its own hotkeys** (leader `F`, hotkey `Leader+F`; leader
  Ctrl+Space, hotkey `Leader+Space`). Holding F and pressing F cannot be told from auto-repeat.
  Refused the same way.
- **Another program holds a combination** the new leader produces (leader Ctrl, and KDE says
  Ctrl+F belongs to another app). Known only when bound: the leader change goes through, that
  one row shows "refused" with the holder's name, and the other rows work.
- **The new leader cannot work on this system** (a modifier leader in tap mode on macOS or
  Windows; the launcher hotkey as the leader). Refused in the recorder with the reason.
- **The leader is removed.** The `Leader+…` entries stay in the file, shown "off" ("no leader
  key set"), and work again when a leader is set.

### Recording

- **The leader:** a "Set Leader Key…" command in the root list opens the settings window's
  Hotkeys page; its "Leader key" field records it. Any key-down records it (no modifier required). A modifier pressed and released
  alone records the modifier leader. Enter saves, Backspace clears, Esc cancels — the existing
  recorder rules. The check refuses only the launcher hotkey. Bound and written like the
  launcher hotkey (bind first, write on acceptance).
- **A Leader+key hotkey:** in the existing "Assign Hotkey…" recorder.
  - Key leader: the leader is a global shortcut, so its press never reaches the recorder's
    window; the daemon already routes presses of tomakast's own shortcuts to an open recorder.
    It now also routes the release. Between them the recorder is "leader down": the next key is
    recorded as `Leader+<key>` and shown as chips `Leader` `F`.
  - Modifier leader, hold mode: the recorder sees Ctrl+F itself; a combination whose modifiers
    are exactly the leader's is recorded as `Leader+F` (it follows a later change of the leader).
  - Tap mode (any leader): the recorder follows the mode. After the leader's activation the
    next key is recorded as `Leader+<key>`, whether or not the leader is still held.
  - The rest of the recorder is unchanged. The settings window's recorder shares this logic
    (see Base branch), so it gains the same "leader down" state.

### Rows

A row with a `Leader+F` hotkey shows the chips `Leader` `F` (author's call: the word, not the
current leader's key, so the row stays correct when the leader changes). States are the
existing ones; "off" also covers "no leader key set".

### Platform

- `HotkeyEvent` gains `Released(String)` (logical id). Backends:
  - KDE: subscribe to `globalShortcutReleased` next to `globalShortcutPressed`; ignore
    `globalShortcutRepeated`.
  - `global-hotkey` (macOS, Windows, X11): forward `HotKeyState::Released` instead of dropping it.
  - Portal: forward `Deactivated`.
- `SetKind` gains `Leader`. The leader set has one binding with id `leader`; KDE registers it
  under that action id like `launcher`.
- A modifier leader in tap mode on KDE is a lone-modifier shortcut: the Qt code of the modifier
  key (`Key_Control`), sent with `setShortcutKeys` flags 6 as the research probed. On the other
  backends this binding is refused with "not available on this system".
- `release_hotkeys` already unregisters the whole component; the leader is part of it.

### Safety

- The invisible window always closes within 5 s of the last key event, whatever the signals do.
- On KDE, kglobalaccel is the compositor's filter, so the leader's release should still be
  reported while our window holds the keyboard. This is **not verified** (no keyboard-taking
  window may be opened on the user's session by the agent). The acceptance run checks it first.
  If it fails, the window falls back to the key-up of the leader it sees itself, and failing
  that, to the 5 s close.

## Base branch (author's call)

The settings-window branch (`claude/settings-gui-3345c2`, unmerged, 26 commits ahead of
`main`) already generalised the hotkey machinery this feature extends: `HotkeyTarget::{Launcher,
Item}` with "bind first, write on acceptance" for both, `HotkeyOrigin`, a generic `set_key`
config editor, and one recorder logic shared by the launcher and the settings window. It also
rewrites the same files (`app/hotkeys.rs`, `core/hotkeys.rs`, `settings.rs`,
`settings_edit.rs`, the recorder, `daemon.rs`). Building on `main` would mean rebuilding that
flow for the leader and a guaranteed conflict in the same functions.

So the leader work is built on top of that branch: the leader is a third target,
`HotkeyTarget::Leader`, and its write is a `set_key` call. The branch is rebased onto `main` once
the settings window is merged; until then the main session merges the settings window first.
The settings session is told about this once the design is approved.

## Overlap with other work

- **Settings window** (branch `claude/settings-gui-*`, hotkeys page): it should show and edit
  `[launcher] leader` and `leader_mode`, and list `Leader+…` rows with the other hotkeys. The
  file format above is the contract; the settings session is told once this is approved.
- **Item hotkeys code** is extended, not forked: same recorder, table, validator, write-back.
- **Hint popup** (backlog, main session): the invisible window is where it would later appear.

## Testing

- Unit (core): leader parsing and display; `Leader+…` parsing, equality and validation; the
  hold and tap state machines including the safety timeout and release-before-open; key
  mapping through the layout tables (Russian `а` → `f`; `.` on the `/` key stays `.`, the known
  limit in "Keys and layouts"); the config editor for `leader`.
- Unit (app): the leader set and its rebinding on leader and mode changes; modifier-leader
  expansion into the item set; routing of press and release to an open recorder.
- Unit (platform): key table rows for the new keys; KDE release signal parsing behind the
  existing fake; `global-hotkey` release forwarding.
- UI: the recorder's "leader down" state, headless with `TOMAKAST_NO_KEYBOARD=1`.
- Manual (user, `just try` on KDE), recorded in `docs/experiments/` as the acceptance note:
  hold mode with F12 and with Ctrl; tap mode; Russian layout; the release check; open-to-focus
  time from the log.

## Left to the builder

- Module names; where the state machine lives (core, so it is testable without GPUI).
- Whether the invisible window is a second window type in `crates/ui/src/window.rs` or a small
  sibling module.
- Exact wording of the command and refusals.
