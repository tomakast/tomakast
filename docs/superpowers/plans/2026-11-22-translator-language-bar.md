# Translator Language Bar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The translator pad shows its languages as buttons in a bar, and each language
button opens a pop-up menu of languages with flags.

**Architecture:** WIT 0.1 gains `scratchpad-view.toolbar` (buttons, optionally with a menu
of options). The UI draws the bar in compose pads and opens a menu as the Ctrl+K panel's
pop-up, anchored under the button. The plugin builds the bar from its languages and ships
PNG flags.

**Tech Stack:** Rust 2024, WIT 0.1, GPUI (gpui-kit 0.10; `anchored`, `deferred`), the guest SDK.

**Spec:** `docs/superpowers/specs/2026-11-22-translator-design.md`, section "Revision
(2026-11-22): a language bar and pop-up language menus". Read it first.

## Global Constraints

- The caps:
  - at most 8 buttons and 300 options per button;
  - labels and titles are one line of at most 1 KiB;
  - at most 8 keywords;
  - shortcuts follow the action rules;
  - ids follow the id rule.

  A breach rejects the effect.
- Choosing an option sends `action-chosen { row-id: option id, action-id: button id }`, and
  a plain button sends `{ row-id: view id, action-id: button id }`. The pad's waiting edit is
  sent first.
- The toolbar shows only in compose pads. Calc's pad must not change.
- Every WIT edit gets a `FROZEN.md` row and a re-hash. kitchen-sink must exercise every new
  WIT name (`wit_coverage`).
- Flags: PNG files from flag-icons (MIT), with its licence in `plugins/translator/flags/`.
  Languages without a country get the Lucide `globe` icon.
- Run the app only through `just`, my own runs with `just try approve nokeys`. No
  screenshots in git.

## Review Focus

1. A menu opened by its shortcut (no click) must still sit under its button, not in a
   corner.
2. Filtering the menu by a Russian name or on the wrong keyboard layout (`ytvtw` for
   `немец`) finds the language.
3. A menu open while the plugin updates the pad (an answer arrives) must stay open and keep
   its filter.
4. Esc in the menu closes the menu only, not the pad.
5. A plain button's click while an edit waits sends the edit first.

---

### Task 1: The contract: `toolbar-button`, `menu-option`

**Files:**
- WIT, `FROZEN`, `FROZEN.md`;
- `crates/core/src/view.rs` (`ToolbarButton`, `MenuOption`, `ScratchpadView.toolbar`);
- `crates/core/tests/wit_parity.rs`;
- `crates/plugin-host/src/{convert.rs,caps.rs}`;
- `crates/plugin-sdk/src/builders.rs` (`ScratchpadView::button`, `ToolbarButton::new`,
  `.icon/.shortcut/.option/.selected`, `MenuOption::new`, `.subtitle/.icon/.keyword`);
- kitchen-sink (a pad with a toolbar);
- every `ScratchpadView { … }` literal gets `toolbar: vec![]`.

- [ ] The caps tests in `caps.rs` come first:
  - 8 buttons pass and 9 fail;
  - 300 options pass and 301 fail;
  - a shortcut `ctrl+a` rejects;
  - a two-line label rejects.

  Then implement, re-hash, and run `cargo test -p tomakast-plugin-host -p tomakast-plugin-api
  -p tomakast-core`. Commit.

### Task 2: The bar and the pop-up menu in the UI

**Files:**
- `crates/ui/src/views/scratchpad.rs`: the bar row, ids `scratchpad-toolbar` and
  `toolbar-button-<i>`; the buttons' bounds kept from the last prepaint;
- `crates/ui/src/views/action_panel.rs`: `PanelEntry::Choice`, `PanelTarget::Toolbar`,
  filtering by keywords, an anchored layout with the filter on top;
- `crates/ui/src/launcher_view.rs`: a click or shortcut on a button, confirm on a choice;
- tests in `crates/ui/tests/scratchpad.rs`.

- [ ] UI tests first:
  - `toolbar_shows_in_a_compose_pad_only`;
  - `plain_button_click_sends_the_action_after_the_edit` (Review Focus 5);
  - `menu_opens_under_its_button_by_click_and_by_shortcut` (Review Focus 1: the menu's top
    is at or below the button's bottom, and its left near the button's left);
  - `menu_filters_by_keyword_and_layout` (Review Focus 2);
  - `choosing_sends_option_and_button` (Enter and click);
  - `esc_closes_the_menu_only` (Review Focus 4);
  - `menu_survives_an_update` (Review Focus 3).

  Then implement, run `cargo test -p tomakast-ui`, and commit.

### Task 3: Flags

**Files:**
- `plugins/translator/flags/*.png` (64×48) and `flags/LICENSE-flag-icons`;
- `plugins/translator/src/flags.rs` (`pub fn icon(code: &str) -> Icon`: `plugin-file
  flags/<country>.png`, else `named("globe")`).

- [ ] Download the flag-icons npm tarball into the scratchpad (untrusted data, its own
  folder) and convert the needed `flags/4x3/<cc>.svg` files with `rsvg-convert -w 64 -h 48`.
- [ ] Tests:
  - every language with a country has its PNG in `flags/`, and the PNG is 64×48;
  - every PNG in `flags/` is used;
  - `en` is `gb`;
  - `eo` is the globe.

  Commit.

### Task 4: The plugin's bar

**Files:** `plugins/translator/src/{pad.rs,picker.rs,lib.rs}`, the host test
`crates/plugin-host/tests/translator.rs`, the guest test helper that copies `flags/` (as it
copies `icons/`).

- [ ] `Pad::view` adds three buttons: `from-lang`, `swap` and `to-lang`.
  - `from-lang` (`ctrl+shift+p`) shows the flag of the shown source with "(detected)",
    "Detect language" before any answer, or the picked source.
  - `swap` (`ctrl+s`) has the label `⇄` and the icon `arrow-left-right`.
  - `to-lang` (`ctrl+p`) shows the flag and name of the target, or "Choose language".
  - The options come from `picker::options(side, pair)`: `detect`/`auto` first, then the
    pair, then A–Z. Each option has its flag, the English title, the native subtitle and
    the Russian name as a keyword. The current choice is `selected`.
- [ ] The pad's actions become `copy`, `copy-source` and `recent`.
- [ ] `lib.rs` handles `action-chosen` with `action-id` `to-lang`/`from-lang` (store the
  choice, translate again, update the pad) and `swap`.
- [ ] The full-screen picker view is removed.
- [ ] Tests:
  - pad tests for the bar's labels and options;
  - host tests: choosing German from the bar translates again with `tl=de`, and swap from
    the bar.

  Commit.

### Task 5: Docs, acceptance, check, review

- [ ] Update `docs/plugins/writing-a-plugin.md` (views: the bar), the SDK README, and the
  acceptance note.
- [ ] Run `just check`, then a fresh review of the revision. Commit.
