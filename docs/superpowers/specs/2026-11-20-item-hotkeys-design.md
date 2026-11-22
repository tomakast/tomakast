# Item hotkeys: assign from the launcher, apply live

Status: approved by the user on 2026-11-20; revised the same day after the KDE probe (see
"Revision: KDE through kglobalaccel"). The revision was announced to the user, who asked not
to be put through more probing.

## Intent

The core spec promises that "any command and any application can be assigned a global
hotkey". Today that is only half true: `config.toml` has a `[hotkeys]` table (hotkey → item
id), the daemon binds it once at start through the `org.freedesktop.portal.GlobalShortcuts`
portal (Linux only), and nothing else. The user asked for two things:

- "can the program register a system hotkey for itself" — answered below, per OS;
- "I want to be able to set a hotkey on any command or on launching any application" — from
  inside the launcher, applied at once, without editing the file or restarting.

Success: the user opens Ctrl+K on a row, picks "Assign Hotkey…", presses a combination,
presses Enter, and the combination works at once; the row shows it; it survives a
restart because it is in `config.toml`, with the user's comments and layout intact.

## What each OS lets an app register by itself

| OS / session | Who registers | User confirmation | Limits |
|---|---|---|---|
| Windows | the app, `RegisterHotKey` | none, silent | fails when another app holds the combination (`ERROR_HOTKEY_ALREADY_REGISTERED`); the OS keeps some (Win+L, Ctrl+Alt+Del) |
| macOS | the app, Carbon `RegisterEventHotKey` | none, silent; no Accessibility or Input Monitoring permission is needed (an event tap would need one; this API does not) | system shortcuts (Cmd+Space for Spotlight, Mission Control keys) win; the press never arrives |
| Linux, KDE (Wayland and X11) | the app, through KDE's own shortcut service (kglobalaccel, D-Bus) | none, silent | the service does not refuse a combination another app holds (both would hold it), so tomakast asks it who holds the combination first and refuses with that app's name |
| Linux, KDE through the GlobalShortcuts portal (the standard route, not used) | the compositor | **yes**: a KDE dialog for every shortcut id it has not seen | needs a desktop file for the app id; KDE keeps a stored trigger per id and ignores a new one; ids left out of a later bind keep holding their combination, and the portal has no call to drop them (probe, 2026-11-20) |
| Linux X11 without KDE | the app, `XGrabKey` | none, silent | fails (`BadAccess`) when another client grabbed it. Built in this work (the user's decision 6) |
| Linux Wayland, GNOME 48+ and other portal desktops | the compositor, through the portal | yes, a dialog | same limits as KDE's portal; here tomakast binds once at start, so changes apply after a restart |
| Linux Wayland without the portal (sway, older GNOME, …) | nobody | — | the user binds `tomakast run <id>` in the compositor's own settings |

Sources: the probe on this machine (`docs/experiments/2026-11-item-hotkeys-portal-probe.md`:
KDE Plasma 6.8.3, xdg-desktop-portal 1.22.1, portal version 2, kglobalacceld 6.8.3), issue
trackers (electron/electron#55611, kando-menu/kando#1546, anomalyco/hex#492,
Runic-Artifex/runic-sdk#567, Jerry0205/Auto-Clicker#76) and the portal spec.

## Revision: KDE through kglobalaccel

The probe showed that live rebinding through the portal strands combinations on KDE (dropped
ids keep holding their keys and nothing can drop them), and each change would cost the user a
dialog. KDE's own shortcut service, which the portal itself uses underneath, lets the app set,
change and remove its shortcuts directly over D-Bus, silently and at once. So on KDE (Wayland
and X11) tomakast uses kglobalaccel. Consequences for the user's decisions: decision 1 holds
(the launcher owns the combination; the file is the source of truth) minus the KDE dialog;
decision 4 holds with "accepted" meaning the service took the combination; there is no
"waiting for confirmation" state on KDE any more. Other Wayland desktops keep the portal,
bound once at start (changes apply after restart).

## Decisions

The user's:

1. **The launcher owns the combination (variant A).** The user presses the combination in
   the launcher; it is written to `config.toml`; on KDE the user confirms KDE's dialog. The
   file is the source of truth on every OS. (Rejected: KDE's own shortcut window choosing the
   key, which would make KDE the source of truth there and behave differently from macOS and
   Windows.)
2. **A conflict with another tomakast row is refused**, not reassigned: the recorder says
   which row holds the combination and Enter saves nothing until the user clears it there.
3. **Path 1: the launcher rebinds at once and writes the file**; hand edits of the file are
   picked up live too, and an unchanged set binds nothing.
4. **The file is written only after the system accepts the combination.** On KDE that is
   after "OK" in its dialog; after "Cancel" or a refusal nothing changes: the file stays as it
   was, the row's previous hotkey (or none) keeps working, and no dialog comes back later.
5. **The launcher hotkey is handled separately** from item hotkeys. It is set by
   `[launcher] hotkey` in `config.toml` (default Alt+Space; Option+Space on macOS is the same
   binding), bound on its own, never disturbed by an item rebind or a cancelled KDE dialog,
   and a hand edit of it is applied live. Assigning it from the launcher is a later, separate
   setting, out of scope here.
6. **X11 is in scope**: on an X11 session hotkeys are registered by the app itself, silently,
   like on macOS and Windows.

Everything else below is the author's call unless marked otherwise.

## Goals

- "Assign Hotkey…" / "Change Hotkey…" in the action panel of every eligible row.
- A recorder: shows the pressed combination, refuses invalid ones and conflicts with a
  reason, Enter saves, Backspace clears the row's hotkey, Esc cancels.
- The row's hotkey shown on the row as key chips, with its state.
- Live apply: the new combination is bound right after Enter and written once the system
  accepts it, on Linux (portal), macOS and Windows; hand edits of `[hotkeys]` and
  `[launcher] hotkey` are applied live as well.
- Write-back that keeps comments, order and formatting of `config.toml`.
- macOS and Windows backends behind `cfg`, type-checked here, not run here.

## Non-goals

- Changing the launcher hotkey from the launcher (decision 5): a later, separate setting.
- Hotkeys on rows inside a pushed view (a plugin's list rows): they are not items.
- Hotkeys on query-time results (calculator answers and other dynamic answers): they have no
  stable id. Only items in the index are eligible.
- A settings window listing all hotkeys. The file and the rows are enough for now.
- Detecting conflicts with other apps before Enter. On KDE the save names the holder; on
  macOS, Windows and X11 the registration fails; elsewhere nothing reports them.

## Anti-goals

- A refused or cancelled rebind must never cost the user a hotkey that worked before:
  neither the row's own previous combination, nor another row's, nor the launcher's.
- A KDE dialog for something the user did not just change. Declined combinations are never
  re-offered, because they never reach the file (decision 4). The one exception is a hand
  edit KDE declined: it is the user's file, so it stays and is offered again at the next
  start.
- Rewriting the user's `config.toml` wholesale, losing comments, or writing when the file
  does not parse.
- More KDE dialogs than changes: binding an unchanged set must be silent.
- Slowing the window's open: nothing here runs on the open path.

## Constraints

- Window open stays ~50 ms; hotkey work is off the open path.
- UI strings in code are English (the mockups in the conversation were in Russian only for
  discussion).
- `TOMAKAST_NO_KEYBOARD=1` never binds real shortcuts; it is also the headless test mode.
- Another session is editing widget code in parallel: keep changes to the hotkey, settings,
  daemon, platform and action-panel areas.
- `tomakast-platform` has no GPUI dependency and stays that way.

## Approaches considered

1. **Launcher rebinds at once and writes the file on acceptance; a file watcher catches
   hand edits (chosen).** Immediate, and a hand edit reaches the same "apply this set" entry point.
   Cost: two triggers; both end in one idempotent "apply set" so an echo of our own write is
   a no-op.
2. **Launcher only writes the file; the watcher rebinds.** One path, but ~0.5 s of debounce
   after Enter, and today's watcher lives in the plugin manager, which may not run.
3. **Hotkeys in tomakast's database.** No comment-preservation problem, but it splits the
   settings; contradicts decision 1.

On KDE the rebind itself has one sensible shape (the portal allows one `BindShortcuts` per
session, and KDE ignores a new trigger for a known id): **a new session per change, bound
with the full set, with one portal shortcut id per (item, combination)**. Alternatives
rejected: re-calling `BindShortcuts` on the same session (refused by the portal);
`ConfigureShortcuts` (KDE's window picks the key: that is the rejected variant B).

For macOS, Windows and X11: the `global-hotkey` 0.10 crate (Tauri), already in the lockfile
as a dev-dependency of the window probe. It wraps exactly `RegisterEventHotKey`,
`RegisterHotKey` and `XGrabKey` (through `x11rb`). Writing our own FFI would be ~400 lines
more for the same calls.

## Design

### What the user sees

**Panel entry.** For a root-list row whose item is in the index, the action panel ends with
a launcher-owned entry: "Assign Hotkey…" or, when the row has one, "Change Hotkey…". It is
the first launcher-owned panel entry; modules cannot see or shadow it (its id is in a
namespace module action ids cannot use, or it is not an `Action` at all — builder's call).

**Recorder.** Choosing it turns the panel into a recorder:

```
┌ Hotkey for "Kitty" ─────────────────────────┐
│   Press a combination…      [Ctrl][Alt][T]  │
│   ✕ Ctrl+Alt+T is set on "Konsole".         │
│     Clear it there first.                   │
│ ↵ Save · ⌫ Clear · Esc Cancel               │
└─────────────────────────────────────────────┘
```

- Each key-down with at least one modifier and a supported key (see Key names) replaces the
  shown combination. Modifiers alone show as pending chips.
- Refused, with the reason, and Enter saves nothing: no Ctrl, Alt or Super (Shift alone does
  not count); the launcher hotkey; a combination another row holds (decision 2); a key outside
  the supported table. Keys the launcher binds inside its window (Ctrl+K, Ctrl+Enter) are
  allowed: a global hotkey fires outside the window, and while the recorder is open it
  captures every key-down before the launcher's own bindings.
- Enter saves; Backspace without modifiers clears the row's hotkey (and saves the
  removal); Esc returns to the panel unchanged. Enter or Backspace with modifiers are
  recordable combinations.
- Hint line: "Nothing appears? The system or another app holds that combination."

**Combinations already bound by tomakast never reach the window** — the OS or the
compositor fires them instead. So while a recorder is open, the daemon routes every
activation of a tomakast hotkey to the recorder as "this combination was pressed" instead of
running it. The recorder then shows the refusal for Konsole's Ctrl+Alt+T, or "That is the
launcher hotkey" for Alt+Space, instead of launching Konsole or hiding the window.
Combinations held by KDE or another app produce nothing at all; the hint covers it.

**After Enter** (decision 4: bind first, write after acceptance):

- **KDE.** tomakast asks kglobalaccel who holds the combination. Held by another app's
  shortcut → refused, a toast names it ("Ctrl+Alt+T is used by kitty: Launch; nothing
  changed"), nothing is written. Free → set at once, then the file is written.
- **Other portal desktops (GNOME).** Bound once at start; a change is written at once and the
  row shows "after restart".
- **macOS, Windows and X11 without KDE.** Registration answers at once: the new combination is registered
  first, then the row's old one released, then the file written. If registering fails
  ("another app holds Ctrl+Alt+T"), the recorder says so and stays open; nothing is released
  or written.
- **No hotkeys in this run** (no portal, unsupported OS, `TOMAKAST_NO_KEYBOARD=1`): there is
  no system to accept, so the file is written at once and the row shows "off". The user can
  still bind `tomakast run <id>` by hand, and the combination works once hotkeys are
  available (author's call).
- If the write fails after the system accepted (disk error, the file became unparsable
  meanwhile), the previous item set is bound back (silent on KDE: its ids are known) and the
  error is toasted.

**Row chips.** A root row with a hotkey shows it on the right as key chips (the existing
`key_chip`), with one of these states (the visual treatment is the builder's call; state must
be readable without colour):

| State | Meaning | Shown |
|---|---|---|
| active | bound and working | chips |
| after restart | written, but this desktop binds only at start (portal desktops other than KDE) | chips + "after restart" |
| refused | a **hand-edited** entry the system declined (a recorded one never reaches the file) | chips + a warning mark; the panel entry's subtitle says why |
| unavailable | no hotkeys in this run: no portal, unsupported OS, or `TOMAKAST_NO_KEYBOARD=1` | chips + "off"; the recorder shows "Hotkeys are not available here: bind `tomakast run <id>` in your system settings" with a copy button |

If the system reports a different trigger than the one asked for (a portal desktop where the
user changed it in the desktop's dialog), the row shows that trigger text verbatim;
`config.toml` keeps what the user recorded.

### Settings and write-back

- New in `tomakast-core::settings`: an editor over the raw text using `toml_edit`
  (0.25 is already in the lockfile through `toml`):
  `set_item_hotkey(text, item, Option<&Hotkey>) -> Result<String>`.
  - Removes every `[hotkeys]` entry whose value is `item`, matching keys by **parsed
    equality** of the hotkey and the item id, not by string; then, for `Some`, inserts
    `"<Hotkey display form>" = "<item id>"`, creating `[hotkeys]` when missing. Comments and
    other tables are untouched.
  - Refuses (error, nothing written) when the text does not parse as TOML or as `Settings`;
    the recorder shows "config.toml has an error: fix it first".
- The write is atomic: a temp file in the same directory, then rename. A symlinked
  `config.toml` is resolved first and the target is replaced, so the link survives. A missing
  file is created.
- Conflict checks run against the settings re-read from disk, at Enter and again just before
  the write, so a hand edit made meanwhile is not overwritten blindly; if the file changed in
  a way that now conflicts, the write is skipped, the previous set re-bound, and a toast says
  why.

### Daemon: two sets, one "apply" entry point each

- Two independent sets: the **launcher set** (one binding, from `[launcher] hotkey`) and the
  **item set** (from `[hotkeys]`). Each is bound on its own (decision 5) and rebinding one
  never touches the other.
- `hotkey_bindings` (exists) stays the validator of settings → bindings, split per set. Its
  checks (no modifier, the launcher's combination, duplicates, a second hotkey for one item)
  are one pure function used by both the recorder (before binding) and the daemon (after
  reading the file).
- `apply` hands a set to the hotkey service only when it differs from the last applied one
  (logical id, combination). The watcher's echo of our own write is therefore a no-op.
- A recorder save is a **pending change**: the daemon binds the item set with the change,
  and on acceptance writes the file and makes it the applied set; on refusal it drops the
  change (the previous binding is still in place). One pending change at a time; a second
  Enter while one is being applied is refused with "Waiting for the last change".
- Triggers: the recorder's save, and a watcher on `config.toml` **owned by the daemon**,
  independent of the plugin manager (whose watcher may not run). Debounced like the plugin
  watcher. A file that fails to load keeps the previous sets, logged. A hand-edited change of
  `[launcher] hotkey` rebinds the launcher set; of `[hotkeys]`, the item set (the file is
  already written, so a refusal there shows as "refused").
- Descriptions (KDE's shortcut settings list them) are the item's title when the index has it, else the id.
- Activation: the service reports our logical id (`launcher` or the item id). The daemon
  routes it to the open recorder if there is one, else runs it as today
  (`Request::Toggle` / `Request::Run`).
- No migration: tomakast has never registered shortcuts with KDE on this machine
  (`kglobalshortcutsrc` has no entry for it).

### Platform: a long-lived hotkey service

`register_global_hotkeys` (spawn once, block until bound) becomes a handle:

```rust
pub struct HotkeyService { .. }
pub enum HotkeyEvent {
    Activated(String),                 // logical binding id
    Applied(Vec<(String, BindState)>), // per logical id, after each apply
    Unavailable { reason: String },
}
pub enum BindState { Active { trigger: Option<String> }, Refused { reason: String }, Later }

impl HotkeyService {
    /// macOS: call on the main thread; Windows: on a thread with a message loop.
    /// The daemon creates it inside the GPUI `run` callback, which is both.
    pub fn start(on_event: impl Fn(HotkeyEvent) + Send + 'static) -> HotkeyService;
    /// Non-blocking; the outcome arrives as `Applied { set, .. }`. `set` names the
    /// launcher set or the item set; each is bound independently.
    pub fn set(&self, set: SetKind, bindings: Vec<HotkeyBinding>);
}
```

The shapes are a sketch; the builder settles names.

- **KDE (kglobalaccel).** Component `dev.tomakast.Tomakast` (friendly name `tomakast`);
  action ids are the logical ids (`launcher`, the item id), stable across changes; the action's
  friendly name is the item title. `set` for a set: for each binding, `globalShortcutsByKey`;
  held by an action other than this one → `Refused { reason: "used by <app>: <action>" }`
  (the launcher hotkey is set regardless, with a warning naming the holders: it is the user's
  own setting); else `doRegister` + `setShortcutKeys(flags = SetPresent | NoAutoloading)`.
  Ids no longer wanted are `unregister`ed. At start, actions of the component not in either
  set (strays from earlier runs) are unregistered. Presses arrive as
  `globalShortcutPressed` on `/component/dev_tomakast_Tomakast`. Keys go over D-Bus as Qt key
  codes (a column of the key table). The two sets stay independent: a set's change never
  touches the other's actions.
- **Other Wayland desktops (portal).** Today's behaviour: one session, both sets bound once
  when both have arrived after start; later `set`s answer at once with `BindState::Later` for
  changed ids (the row's "after restart").
- **Choosing the Linux backend at start:** `org.kde.kglobalaccel` has an owner on the session
  bus → kglobalaccel; else a Wayland session (`WAYLAND_DISPLAY`) → portal; else an X11 session
  (`DISPLAY`) → `global-hotkey`'s X11 backend; else the null service. A Wayland session
  without the portal does not fall back to X11 (XWayland grabs see only XWayland windows'
  keys).
- **macOS, Windows and X11 (`global-hotkey`).** On the UI thread (X11 does not require it, but
  one code path is simpler). `set` diffs against the
  registered set: registers added first, then unregisters removed; a failure registering any
  added one unregisters the others just added and reports the whole change `Refused`, so the
  old set is intact. Events come from `GlobalHotKeyEvent::set_event_handler`; only `Pressed`
  counts.
- **Other targets, and `TOMAKAST_NO_KEYBOARD=1`.** A null service that reports
  `Unavailable` once and accepts every change for the file's sake (see After Enter); rows
  show "off".

### Key names

One tested table maps: the key GPUI reports → the config name (`t`, `f1`, `space`) → the xkb
keysym for the portal trigger (`T`, `F1`, `space`, `Return`, `Prior`) → the Qt key code for
kglobalaccel (`0x54`, `0x01000030`, `0x20`, `0x01000004`, `0x01000016`) → the `global-hotkey`
`Code`. Today `to_xdg_trigger` emits the config name as is, which is wrong for named keys;
this table fixes it. The recorder accepts only keys in the table: letters, digits, F1–F12,
space, enter, tab, arrows, home, end, page up/down, insert, delete, and common punctuation
(builder's call on which). With a non-Latin layout (the user types Russian), the recorder
must record the Latin key of the physical key, the one the portal and the OS APIs bind; the
builder checks what GPUI reports under a Russian layout and maps through the key code if it
reports the Cyrillic letter.

## Step 1: the KDE probe (done)

Done on 2026-11-20; findings in `docs/experiments/2026-11-item-hotkeys-portal-probe.md`. They
led to the revision above. kglobalaccel's behaviour was checked over D-Bus by the agent
(encoding, flags, conflicts, `invokeShortcut` for tests) without further help from the user.

## macOS, Windows and X11: estimate and what is not run

- One backend through `global-hotkey` for all three: ~150 lines plus the key table's `Code`
  column and the Linux session choice; about half a day including tests of the diffing and
  the key table, which run on Linux.
- Checked here: `cargo clippy -p tomakast-platform --target aarch64-apple-darwin` and
  `--target x86_64-pc-windows-gnu` (both targets are installed). CI's macOS and Windows jobs
  compile and run the unit tests.
- X11 can be checked here headlessly only if `Xvfb` and `xdotool` are installed (they are
  not now; installing them is the user's call). Otherwise X11 joins the manual list: log in
  to a "Plasma (X11)" session and run `just try`.
- **Not run here**: a real press on macOS or Windows, the main-thread requirement on macOS,
  the message-loop requirement on Windows, system-shortcut precedence. The acceptance notes
  list these for a manual run on the user's Mac and Windows machines.

## Testing

TDD throughout. Pure parts get unit tests: the config editor (comments kept, parsed-equality
removal, refusing a broken file, creating `[hotkeys]`), validation and conflicts, the
set-diff ("unchanged set binds nothing"), the key table in every direction, the portal id
hashing and the session bookkeeping (behind a fake portal), the macOS/Windows diffing (behind
a fake registrar). UI: the recorder's states through the view's test support, headless with
`TOMAKAST_NO_KEYBOARD=1`. The live KDE path is verified by the user through `just try`.

## Left to the builder

- Names and module layout; whether the panel entry is an `Action` in a reserved namespace or
  a separate panel row type.
- Chip visuals per state.
- The punctuation in the key table.
- Where the daemon's config watcher lives (a small reuse of `tomakast-plugin-host::watch` or
  its own), as long as it does not depend on plugins loading.
- How the recorder's "capture mode" reaches the daemon's activation routing (a flag on the
  controller the daemon reads on the UI thread is enough).
