//! The pages of the settings window, built from the schema, the file and the plugin catalog
//! (the window keeps them until those change): General, Launcher, Appearance, Location, Hotkeys, Home (shown as Widgets), About, then (after a gap in the sidebar) one page per
//! installed plugin in name order and one per `[plugins.<id>]` table of a plugin that is not
//! installed, by id (Plugins).

mod about;
mod appearance;
mod general;
mod home;
mod hotkeys;
mod launcher;
mod plugins;

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, IconName, Sizable as _};
use gpui_kit::{
    AnyElement, App, Context, ElementId, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, WeakEntity, Window, div, px,
};
use tomakast_core::PluginCatalog;
use tomakast_core::hotkeys::{HotkeyState, HotkeyTarget};
use tomakast_core::settings_schema::unrecognized;

pub(crate) use home::size_lists;
pub(crate) use plugins::{command_hotkey, pick_file_value, pick_list, unset_text, value_text};

use super::SettingsPage;
use super::fields::{HotkeyStand, StandTone};
use super::layout::{Group, Page, Row, RowOptions, control_button};
use super::window::{LAUNCHER_HOTKEY, LEADER, SettingsWindow, key};
use crate::theme::{self, Palette};

/// Every page, in sidebar order, for `catalog` and the file as a table. Plugin pages (and the
/// pages of `[plugins.x]` tables) exist only once the plugins folder is listed.
pub(crate) fn page_keys(
    catalog: &PluginCatalog,
    catalog_known: bool,
    file: Option<&toml::Table>,
) -> Vec<SettingsPage> {
    let mut keys = vec![
        SettingsPage::General,
        SettingsPage::Launcher,
        SettingsPage::Appearance,
        SettingsPage::Location,
        SettingsPage::Hotkeys,
        SettingsPage::Home,
        SettingsPage::About,
    ];
    if !catalog_known || catalog.unavailable.is_some() {
        return keys;
    }
    // The catalog comes sorted by name.
    let installed = catalog
        .plugins
        .iter()
        .map(|plugin| plugin.id.clone())
        .collect::<Vec<_>>();
    let mut tables: Vec<String> = plugins::table_ids(file)
        .into_iter()
        .filter(|id| !installed.contains(id))
        .collect();
    tables.sort();
    keys.extend(
        installed
            .into_iter()
            .chain(tables)
            .map(SettingsPage::Plugin),
    );
    keys
}

/// The pages for `keys`, as the window shows them now.
pub(crate) fn build(
    view: &SettingsWindow,
    keys: &[SettingsPage],
    this: &WeakEntity<SettingsWindow>,
    cx: &mut Context<SettingsWindow>,
) -> Vec<Page> {
    let palette = theme::palette(cx);
    keys.iter()
        .map(|key| match key {
            SettingsPage::General => general::general(view, this),
            SettingsPage::Launcher => launcher::page(view, this, &palette),
            SettingsPage::Appearance => appearance::page(view, this),
            SettingsPage::Location => general::location(view, this),
            SettingsPage::Hotkeys => hotkeys::page(view, this),
            SettingsPage::About => about::page(&palette),
            SettingsPage::Home => home::page(view, this, &palette),
            SettingsPage::Plugin(id) => match view.catalog.plugins.iter().find(|p| p.id == *id) {
                Some(plugin) => plugins::plugin_page(view, this, plugin, &palette),
                None => plugins::table_page(view, this, id, &palette),
            },
        })
        .collect()
}

/// A line of muted text, as a row of its own.
pub(super) fn note(id: &'static str, text: String, palette: &Palette) -> Row {
    let muted = palette.muted;
    Row::custom(move |_, _, _| {
        div()
            .id(id)
            .aria_label(text.clone())
            .test_support()
            .text_size(theme::text(theme::SMALL_TEXT_SIZE))
            .text_color(muted)
            .child(text.clone())
    })
}

/// `page` with a "Not recognized" group for the file's keys outside the schema whose path
/// `belongs`, if there are any: each with the parser's warning and a remove button. The
/// window removes nothing by itself.
fn with_unrecognized(
    page: Page,
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    belongs: impl Fn(&[String]) -> bool,
) -> Page {
    let Some(file) = &view.file.file else {
        return page;
    };
    let entries: Vec<_> = unrecognized(file)
        .into_iter()
        .filter(|u| belongs(&u.path))
        .collect();
    if entries.is_empty() {
        return page;
    }
    let disabled = view.file.error().is_some();
    let rows = entries.into_iter().map(|entry| {
        let name = entry.path.join(".");
        let this = this.clone();
        let path = entry.path;
        let label = format!("Remove {name}");
        // A removal that failed says why under the button.
        let error = view.errors.get(&name).cloned();
        let render = move |options: RowOptions, _: &mut Window, cx: &mut App| {
            let this = this.clone();
            let path = path.clone();
            let palette = theme::palette(cx);
            let remove = control_button("remove", &palette, cx)
                .label("Remove")
                .accessibility_label(label.clone())
                .disabled(options.disabled)
                .on_click(move |_, window, cx| {
                    let path: Vec<&str> = path.iter().map(String::as_str).collect();
                    let _ = this.update(cx, |view, cx| view.set_key(&path, None, window, cx));
                });
            div()
                .flex()
                .flex_col()
                .items_end()
                .gap_1()
                .child(remove)
                .children(
                    error
                        .clone()
                        .map(|error| error_line("remove-error".into(), error, &palette)),
                )
        };
        Row::new(format!("{name} = {}", entry.value), render)
            .description(entry.reason)
            .disabled(disabled)
    });
    page.group(
        Group::new()
            .title("Not recognized")
            .description("The launcher does not use these keys.")
            .rows(rows),
    )
}

/// What a reset button does.
pub(super) type Reset = Rc<dyn Fn(&mut Window, &mut App)>;

/// What a built-in key's item shows besides its control.
#[derive(Default)]
pub(super) struct KeyState {
    /// The file holds the key, so it can be reset.
    pub(super) held: bool,
    /// Lines under the control in the error colour: an invalid value, a write that failed.
    pub(super) lines: Vec<String>,
}

/// A control, the reset button when the file holds the key, and the lines under them.
pub(super) fn key_control(
    state: &KeyState,
    title: &str,
    control: AnyElement,
    disabled: bool,
    reset: Reset,
    cx: &App,
) -> AnyElement {
    let palette = theme::palette(cx);
    // The reset button sits left of the control and exists only while the file holds the key,
    // so the control always ends at the row's right edge and never moves.
    let reset_button = state.held.then(|| {
        Button::new("reset")
            .icon(IconName::Undo2)
            .ghost()
            .xsmall()
            .accessibility_label(format!("Reset {title}"))
            .tooltip("Reset to default")
            .disabled(disabled)
            .on_click(move |_, window, cx| reset(window, cx))
    });
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .children(reset_button)
                .child(control),
        )
        .children(
            state
                .lines
                .iter()
                .enumerate()
                .map(|(ix, line)| error_line(("key-line", ix).into(), line.clone(), &palette)),
        )
        .into_any_element()
}

/// How `target`'s hotkey stands, for the icon inside its field: the answer to the last
/// change asked for here when it was `target`'s, the daemon's table, and for the launcher
/// hotkey and the leader key what is wrong with the file's value. `None`: nothing to say.
pub(crate) fn hotkey_stand(view: &SettingsWindow, target: &HotkeyTarget) -> Option<HotkeyStand> {
    let mut problems = Vec::new();
    match target {
        HotkeyTarget::Launcher => {
            problems.extend(general::key_state(view, key(LAUNCHER_HOTKEY)).lines);
        }
        HotkeyTarget::Leader => problems.extend(general::key_state(view, key(LEADER)).lines),
        HotkeyTarget::Item(_) => {}
    }
    if let Some((asked, text)) = &view.hotkey_notice
        && asked == target
    {
        problems.push(text.clone());
    }
    let table = view.table.borrow();
    let current = match target {
        HotkeyTarget::Launcher => table.launcher_state.as_ref(),
        HotkeyTarget::Leader => table.leader_state.as_ref(),
        HotkeyTarget::Item(item) => table.of(item).map(|h| &h.state),
    };
    let note = match current {
        Some(HotkeyState::AfterRestart) => Some((StandTone::Restart, "Applies after a restart")),
        Some(HotkeyState::Off) => Some((StandTone::Off, "Off")),
        Some(HotkeyState::NoLeader) => Some((StandTone::Off, "Off: no leader key set")),
        Some(HotkeyState::Refused { reason }) => {
            problems.push(format!("Refused: {reason}"));
            None
        }
        Some(HotkeyState::Active { .. }) | None => None,
    };
    match note {
        _ if !problems.is_empty() => Some(HotkeyStand {
            tone: StandTone::Problem,
            text: problems
                .into_iter()
                .chain(note.map(|(_, text)| text.to_string()))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Some((tone, text)) => Some(HotkeyStand {
            tone,
            text: text.into(),
        }),
        None => None,
    }
}

/// A line in the error colour, findable by its text.
pub(super) fn error_line(id: ElementId, text: String, palette: &Palette) -> AnyElement {
    div()
        .id(id)
        .aria_label(SharedString::from(text.clone()))
        .test_support()
        .max_w(px(320.))
        .text_size(theme::text(theme::SMALL_TEXT_SIZE))
        .text_color(palette.error)
        .child(text)
        .into_any_element()
}
