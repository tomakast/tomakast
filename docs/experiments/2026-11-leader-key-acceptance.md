# Leader key: acceptance (2026-11-23)

Spec: `docs/superpowers/specs/2026-11-22-leader-key-design.md`.

## What was checked automatically

`just check` (fmt, clippy `-D warnings`, every workspace test, both guest plugins): exit 0.
Tests by spec section:

- **Core, leader parsing and display; `Leader+...` parsing, equality, validation:**
  `leader_hotkeys_parse_and_display`, `a_leader_may_be_modifiers_alone`,
  `leader_settings_are_read`, `a_launcher_hotkey_with_the_leader_is_an_error`,
  `leader_hotkeys_need_no_other_modifier`, `leader_key_of_own_hotkey_is_refused`,
  `leader_change_clash_is_refused`, `the_leader_may_not_be_the_launcher_hotkey`,
  `the_launcher_hotkey_cannot_use_the_leader`, `the_launcher_compares_by_the_keys_a_modifier_leader_makes`,
  `with_a_modifier_leader_in_hold_mode_leader_f_is_ctrl_f`, `the_file_rules_follow_a_modifier_leader`,
  `a_leader_that_is_an_items_hotkey_is_refused`, `an_item_may_not_take_the_leader_key`,
  `a_combination_leader_in_hold_mode_makes_leader_f_its_modifiers_and_f`,
  `a_leader_whose_key_is_not_supported_is_refused`, `shortcut_may_not_use_the_leader`,
  `a_blank_leader_is_no_leader`.
- **Core, hold and tap state machines, safety timeout, release-before-open, layout mapping:**
  `hold_runs_each_key_until_release`, `auto_repeat_runs_once`, `leader_repeat_while_held_is_ignored`,
  `release_before_open_closes_at_once`, `tap_runs_one_key_and_closes`,
  `tap_unknown_key_or_escape_closes`, `escape_closes_hold`, `tap_leader_again_closes`,
  `safety_timeout_closes_held`, `safety_timeout_closes_armed`, `a_key_event_pushes_the_deadline`,
  `focus_lost_closes`, `set_mode_goes_idle`, `russian_layout_keys_map_to_us`,
  `recording_follows_the_mode`, `us_key_reads_layout_characters`.
- **Core, config editor for `leader`:** `set_key_changes_one_value_and_keeps_comments` and the
  other `set_key_*` tests of the editor, `every_schema_key_reaches_settings`.
- **App, the leader set and its rebinding on leader and mode changes:**
  `a_key_leader_binds_the_leader_set_and_keeps_leader_items_off_the_system`,
  `a_leader_change_is_written_after_acceptance`, `refused_leader_change_keeps_the_old_leader`,
  `a_failed_leader_set_rebinds_the_previous_one`, `leader_change_clash_is_refused_before_binding`,
  `the_leader_it_already_has_is_a_no_op`, `a_leader_a_hand_edit_made_clash_meanwhile_is_not_written`,
  `a_hand_edit_of_the_leader_mode_rebinds_both_sets`, `without_a_leader_leader_items_show_no_leader`,
  `x11_key_leader_acts_as_tap`.
- **App, modifier-leader expansion into the item set:**
  `a_modifier_leader_in_hold_mode_registers_ctrl_f`,
  `a_modifier_leader_in_tap_mode_binds_the_lone_modifier`,
  `a_change_to_a_modifier_leader_rebinds_the_items_first`,
  `a_failed_change_to_a_modifier_leader_rebinds_the_old_items` (and the broken-file variant),
  `a_refused_change_from_a_modifier_leader_rebinds_the_old_items`,
  `a_modifier_leader_in_tap_mode_the_backend_refuses_is_not_written`.
- **App, routing of press and release to a recorder:** covered by reading only. The daemon's
  `leader_event` hands a press or release of the leader to an open recorder before the leader
  driver; no test drives it (it needs both controller globals). Tested around it: the release
  is reported (`releases_are_reported`), the recorders take it (the UI tests below), and the
  window's keys reach the machine (`a_window_key_becomes_a_leader_hotkey`,
  `the_key_up_of_the_leader_key_is_its_release`, `escape_and_focus_loss_pass_through`).
- **App, the 5 s close driven by the driver's own timer (fake clock):**
  `the_driver_closes_by_itself_after_five_seconds` (hold and tap),
  `a_key_event_pushes_the_close_back`, `a_stale_timer_does_not_close_a_newer_hold`.
- **App, a hand-edited leader equal to the launcher hotkey:**
  `a_hand_edited_leader_equal_to_the_launcher_hotkey_is_not_bound`.
- **Platform, key table rows for the new keys:** `new_keys_have_names`,
  `a_lone_modifier_is_its_key_code`, `scroll_lock_is_scrolllock_in_gpui`.
- **Platform, KDE release signal parsing behind the existing fake:** `signals_map_to_events`,
  `keys_of_a_lone_ctrl`, `deactivated_is_a_release`.
- **Platform, `global-hotkey` release forwarding:** `a_release_is_forwarded`,
  `a_modifier_alone_is_refused`, `a_modifier_alone_is_refused_after_the_bind_too`.
- **Platform, portal press and release ordering:** `a_press_older_than_a_delivered_release_is_dropped`,
  `a_press_stamped_as_its_release_is_kept`, `a_portal_without_timestamps_loses_no_press`,
  `presses_and_releases_in_order_are_kept`.
- **UI, the invisible leader window (headless, `crates/ui/tests/leader_window.rs`):**
  `keys_reach_the_sink`, `close_removes_the_window`, `a_second_open_is_a_no_op`,
  `the_sink_can_close_the_window`, `losing_focus_after_having_it_reaches_the_sink`,
  `key_ups_reach_the_sink`, `a_window_closed_outside_close_reports_lost_focus`,
  `the_sink_may_close_when_told_of_a_compositor_close`.
- **UI, the recorder's "leader down" state (headless, `crates/ui/tests/hotkeys.rs`):**
  `leader_down_records_leader_plus_key`, `the_leader_is_not_taken_without_a_recorder`,
  `a_ctrl_leader_records_ctrl_f_as_leader_f`, `leader_rows_show_the_leader_chip`,
  `a_released_leader_ends_leader_plus_key`; the settings window's Leader chips and clash
  wording: `chips_name_the_leader_first_and_a_leader_by_its_modifiers`,
  `a_leader_clash_names_the_leader_the_hotkey_and_the_other_item`; a system-fired `Leader+F`
  in the launcher and leader fields: `the_launcher_field_shows_a_fired_leader_hotkey_as_its_keys`,
  `the_leader_field_shows_a_fired_leader_hotkey_as_its_keys`.
- **Settings module:** `leader_item_opens_the_hotkeys_page` ("Set Leader Key...").

## The headless run

`just build`, then the release binary run directly with `TOMAKAST_NO_KEYBOARD=1`, a throwaway
`TOMAKAST_HOME` (a short path under `/tmp`: a longer one overflows the IPC socket's path limit),
`RUST_LOG=info,tomakast_ui=debug` and a config with `[launcher] leader = "F12"` and
`[hotkeys] "Leader+F" = "settings/open"`. The log:

```
INFO tomakast_app::hotkeys: binding hotkeys set=Launcher generation=1 count=1
INFO tomakast_app::hotkeys: binding hotkeys set=Leader generation=2 count=1
INFO tomakast_app::hotkeys: binding hotkeys set=Items generation=3 count=0
WARN tomakast_app::hotkeys: global hotkeys are unavailable (TOMAKAST_NO_KEYBOARD=1); ...
```

The leader set is bound (one key: F12) and `Leader+F` stays out of the item set (count 0:
it is a leader hotkey, not a system one). No `leader window` line appears: no window opened
without a leader press. The process was stopped by its PID and the directory deleted.

## Not run

- **A physical leader press on KDE**: the release signal from kglobalaccel, window focus,
  Wayland key delivery. The checklist below covers it.
- **macOS, Windows, X11, GNOME or another portal desktop.** CI compiles and runs unit tests on
  macOS and Windows.

## Manual checklist (KDE)

Run `just try`, then set the leader in Settings -> Hotkeys.

1. **The release check, first.** Leader F12 in hold mode, `Leader+F` on Firefox. Hold F12, press
   F: Firefox starts. Release F12, type `f` in an editor: the letter appears, so the window
   closed on release. If the letter is lost for up to 5 s, KDE did not report the release; note
   it.
2. Hold F12, press F then T: both items start. Holding F starts Firefox once.
3. A non-Latin layout active (Russian, or any other you use): hold F12, press the key where F is
   on US: Firefox starts.
4. Leader Ctrl, hold mode: `Leader+F` works as Ctrl+F everywhere. KDE names a holder if Ctrl+F is
   taken by another global shortcut.
5. Tap mode: `leader_mode = "tap"`. Press and release F12, then F: Firefox starts. Press F12 and
   wait 5 s: typing works again. Press F12, then Esc: closed. Leader Ctrl in tap mode: tap Ctrl,
   then F.
6. Change the leader from F12 to Ctrl while `Ctrl+F` is set on another row: refused, with the
   message naming the row.
7. Combination leader: set the leader to Ctrl+Space (hold mode) with `Leader+F` on Firefox, then
   try to give another row Ctrl+F: refused (holding Ctrl+Space and pressing F is Ctrl+F). Hold
   Ctrl+Space, press F: Firefox starts. Note when KDE reports the leader's release: when Space
   comes up, or only when Ctrl does.
8. Remove the leader: `Leader+F` rows show "no leader key set".
9. `just log`: `leader window focused ms=...` lines, all under 50.
10. Ctrl+C `just try`: KDE's shortcut settings keep no tomakast entries, the leader included.

## macOS / Windows (manual, not run here)

Steps 1-3 and 5. Expectation: the window takes focus; on Windows, the foreground rule after
`WM_HOTKEY` allows it.

## Known limits

- X11: a key leader acts as tap.
- A modifier leader in tap mode is KDE-only.
- Keys pressed within the first ~20 ms after the leader go to the previous program.
- On a layout that moves ASCII punctuation to another key (Russian: the `/` key types `.`), that
  key gives `Leader+.` like the `ю` key; letters are always by physical key, on any non-Latin
  layout.
- Portal backend (non-KDE Wayland): a press stamped with the same time as its release, or with
  no timestamp, may still arrive after its release; the 5 s close bounds it.
- A modifiers-only leader in hold mode has no window: `Leader+F` is registered as `Ctrl+F`
  directly. Releasing a key leader is also detected from the window's own key-up when the system
  does not report it.
- The `leader window focused ms` number is measured from the moment the window is asked to open,
  not from the key press.
