# Item hotkeys: KDE probe findings (2026-11-20)

Desktop: KDE Plasma 6.8.3 on Wayland, xdg-desktop-portal 1.22.1, xdg-desktop-portal-kde 6.8.3,
GlobalShortcuts portal version 2, kglobalacceld 6.8.3. Probe: `crates/platform/examples/portal_probe.rs`
(`just hotkey-probe`), three runs by the user, then D-Bus checks of kglobalaccel by the agent.

## GlobalShortcuts portal

- **Host app id needs a desktop file.** `Registry.Register` fails with "App info not found"
  unless `<app id>.desktop` exists in an applications directory, and the portal notices a new
  file only after a moment (a retry ~1 s later succeeds). Without registration KDE files the
  shortcuts under whichever app started the process: runs from the Claude app's terminal landed
  in `[com.anthropic.Claude]`, including an older `launcher=Alt+Space` from `just try`.
- **Several sessions of one app can be bound at once**, and every live session receives every
  activation of the app's component (one `Activated` per session per press).
- **New ids show KDE's dialog** (confirmed for a first bind).
- **Cancel in the dialog does not fail the bind**: the response succeeds, the new id comes back
  with an empty `trigger_description` and is stored as `none,<default>`; known ids keep their
  triggers.
- **Lowercase letter keysyms work** (`CTRL+ALT+SHIFT+j` → "Ctrl+Alt+Shift+J").
- **Ids left out of a later bind keep holding their combination.** `x2` (dropped) still held
  Ctrl+Alt+Shift+L; a new id `x4` asking for L came back with an empty trigger. Confirmed with
  `globalShortcutsByKey`, which listed `x2` as the active holder of L.
- **Unclear:** whether closing an old session silences the ids a newer session also bound (no
  activation was printed after a close, but the user may not have pressed the keys).

Conclusion: the portal cannot rebind live on KDE without stranding combinations (there is no
call to drop an id), so the spec's session-per-change design is replaced (see the spec's
revision note).

## kglobalaccel (KDE's own shortcut service), checked over D-Bus

Service `org.kde.kglobalaccel`, object `/kglobalaccel`, interface `org.kde.KGlobalAccel`; an
action id is `as` = `[component unique, action unique, component friendly, action friendly]`.

- **Keys are Qt key codes** in `a(ai)`: modifiers Shift `0x02000000`, Ctrl `0x04000000`, Alt
  `0x08000000`, Meta `0x10000000`, plus the key (`'T'` = `0x54`, F4 = `0x01000033`). Read back from
  kitty's Ctrl+Alt+T (`201326676`) and KWin's Alt+F4 (`150994995`).
- **`setShortcutKeys(id, keys, flags)`**: flags `2` (SetPresent) keeps a stored shortcut and
  ignores the new keys; flags `6` (SetPresent | NoAutoloading) sets the requested keys. No dialog.
- **No conflict refusal**: asking for a combination another action holds succeeds, and both then
  hold it. The caller must check first with `globalShortcutsByKey((ai) keys, (i) 0)`, which lists
  every holder with component and action names.
- **`unregister(component, action)`** removes an entry (used to clean up the probe entries).
- **Activation**: object `/component/<component with . replaced by _>`, interface
  `org.kde.kglobalaccel.Component`, signal `globalShortcutPressed(s component, s action, x ts)`.
  `invokeShortcut(s action)` emits the same signal without a key press, so an end-to-end test
  needs no keyboard.
- Alt+Space is currently held by a stale `launcher` entry under `com.anthropic.Claude` (from a
  `just try` run without a desktop file); left in place for the user to decide.

Not observed: a physical key press reaching tomakast through kglobalaccel (only
`invokeShortcut`).
