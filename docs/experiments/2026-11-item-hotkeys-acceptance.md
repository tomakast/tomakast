# Item hotkeys: acceptance (2026-11-20)

Spec: `docs/superpowers/specs/2026-11-20-item-hotkeys-design.md` (with its KDE revision).
Plan: `docs/superpowers/plans/2026-11-20-item-hotkeys.md`. Probe findings:
`docs/experiments/2026-11-item-hotkeys-portal-probe.md`.

## What each OS lets an app register by itself

| OS / session | Who registers | User confirmation | Limits |
|---|---|---|---|
| Windows | the app (`RegisterHotKey`) | none | fails when another app holds the combination; Win+L and the like stay the system's |
| macOS | the app (Carbon `RegisterEventHotKey`) | none; no Accessibility or Input Monitoring permission | system shortcuts (Cmd+Space, Mission Control) win |
| Linux, KDE (Wayland and X11) | the app, through KDE's shortcut service (kglobalaccel, D-Bus) | none | KDE does not refuse a combination another app holds; tomakast looks it up and refuses with the holder's name. KDE keeps the shortcuts after the app exits (`tomakast release-hotkeys` drops them) |
| Linux, X11 without KDE | the app (`XGrabKey`) | none | fails when another client grabbed it |
| Linux, Wayland with the portal (GNOME and others) | the compositor, through the GlobalShortcuts portal | yes, a dialog for each new shortcut | the portal cannot drop or rebind a shortcut, so tomakast binds once at start and changes apply after a restart |
| Linux, Wayland without the portal | nobody | — | bind `tomakast run <id>` in the desktop's settings |

## Run here (KDE Plasma 6.8.3 on Wayland)

- `just check`: fmt, clippy (`-D warnings`), every workspace test, both guest plugins. Result:
  see the commit that adds this note.
- Unit tests: key table (keysyms, Qt codes checked against values KDE stores, shifted symbols),
  hotkey rules, the comment-preserving config editor (including a symlinked `config.toml`),
  the native backend's diffing, the null backend, the KDE backend's conflict and stray logic,
  the portal's "after restart" states, the daemon's state machine (write only after
  acceptance, refusals, a conflicting hand edit, a broken file, clearing), the config watcher.
- Headless UI tests (`crates/ui/tests/hotkeys.rs`): panel entry, row chips and their states,
  recorder (combination shown, launcher keys captured, save, conflicts, no modifier,
  Ctrl+Shift+1, a non-Latin key, clear, cancel, a fired tomakast hotkey, unavailable hotkeys).
- Live on this KDE (`cargo test -p tomakast-platform --lib -- --ignored kde_live`): registers a
  test component, `invokeShortcut` reaches the backend as a press, a shifted digit is accepted
  in both of KDE's forms, a second component asking the same combination is refused with the
  first one's name, everything is unregistered.
- The vendored GPUI now names the digit row as on US when Ctrl, Alt or Super is held
  (`third_party/gpui-pre-linux/TOMAKAST.md`); tested against real xkb `ru` and `fr` keymaps.
- Daemon smoke run with its own `TOMAKAST_HOME` and `TOMAKAST_NO_KEYBOARD=1`: both sets bound,
  the null backend's warning, a hand edit of `[hotkeys]` rebinding only the item set.
- Cross-target clippy of `tomakast-platform` (lib and examples) for `aarch64-apple-darwin` and
  `x86_64-pc-windows-gnu` (C code stubbed: metadata only).

## Not run

- **A physical key press on KDE** reaching tomakast through kglobalaccel (only
  `invokeShortcut`); the user's `just try` covers it, including Ctrl+Shift+digit (registered in
  both KDE forms, a physical press unverified).
- **macOS and Windows** at all: registering, presses, the main-thread requirement on macOS,
  the message-loop requirement on Windows, system-shortcut precedence. CI compiles and runs the
  unit tests there.
- **X11** (no Xvfb or xdotool here), with or without KDE.
- **GNOME or another portal desktop**: the bind-once path.

## Manual checklist

KDE (`just try approve`, then `just toggle`):
1. Select an app, Ctrl+K → "Assign Hotkey…", press e.g. Ctrl+Alt+Y, Enter. Hide the launcher
   and press Ctrl+Alt+Y: the app starts. The row shows `Ctrl Alt Y`.
2. Assign kitty's Ctrl+Alt+T to another row: a toast says it is used by kitty; nothing saved.
3. Assign a combination another row has: the recorder refuses and names that row.
4. Ctrl+K → "Change Hotkey…" → Backspace: the hotkey is gone from the row and from the file.
5. Edit `config.toml` in the try home by hand (keep a comment): the change applies within a
   second, and the comment survives a save from the recorder.
6. Assign Ctrl+Alt+Shift+2 to a row (with the Russian layout active too): the row shows
   `Ctrl Alt Shift 2`, and the press runs it.
7. Ctrl+C `just try`: afterwards Ctrl+Alt+Y does nothing and KDE's shortcut settings have no
   tomakast entries left.

macOS / Windows: the same steps 1, 3, 4, 5; step 2 with a combination another app registered
(the registration fails and the toast names the combination).

## Known limits and follow-ups

- The real daemon does not release its KDE shortcuts when it is killed or crashes; they do
  nothing until the next start (release on SIGTERM is a follow-up).
- On KDE, item titles appear in KDE's shortcut settings only for items indexed when the set
  was bound; at startup the index is empty, so the ids show until the next change.
- A stale `launcher = Alt+Space` entry under the Claude app's section of KDE's shortcuts (from
  earlier `just try` runs without a desktop file) may compete with tomakast's Alt+Space.
- `crates/platform/tests/autostart.rs` fails cross-target clippy (pre-existing, unrelated).
- Deferred review minors (in the session's final report): a hand edit during a pending change
  can be lost on refusal; our own write's echo rebinds the item set; refusals after Enter are
  toasts, not in the recorder; a refused launcher hotkey is not shown; numpad digits record as
  main-row digits; `just try` shares KDE's component name with an installed daemon.
