//! General (start at login, the config file) and Location (the city, the Wi-Fi lookup,
//! the time zone):
//! the built-in keys of `config.toml`, as the schema describes them.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, Entity, InteractiveElement as _, IntoElement as _, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, TextRun,
    WeakEntity, Window, div, font, px,
};
use tomakast_core::settings_schema::{KeyValue, SettingKey};

use super::appearance::choice_row;
use super::{KeyState, Reset, error_line, key_control, with_unrecognized};
use crate::settings::SettingsPage;
use crate::settings::layout::{Group, Page, PageIcon, Row, RowOptions, Section, control_button};
use crate::settings::window::{
    CITY, CONFIG_FILE, LAUNCHER_HOTKEY, LEADER, LEADER_MODE, SettingsWindow, TIME_ZONE, key,
};
use crate::theme;

const AUTOSTART: &[&str] = &["launcher", "autostart"];
const WIFI_LOOKUP: &[&str] = &["location", "wifi_lookup"];

pub(super) fn general(view: &SettingsWindow, this: &WeakEntity<SettingsWindow>) -> Page {
    let page = Page::new(
        SettingsPage::General,
        Section::Launcher,
        "General",
        PageIcon::Named("settings"),
    )
    .group(
        Group::new()
            .row(switch_row(view, this, AUTOSTART))
            .row(config_file(view, this)),
    );
    // Top-level keys, and those of `[launcher]`; a `location` or `home` that is not a table is
    // on its own page.
    with_unrecognized(page, view, this, |path| {
        path[0] == "launcher" || (path.len() == 1 && path[0] != "location" && path[0] != "home")
    })
}

/// The Config file row: where the file is, and buttons to open it or show it in its folder.
/// Both make sure the file exists first; they stay usable when the file has an error, since
/// opening it is how that is fixed.
fn config_file(view: &SettingsWindow, this: &WeakEntity<SettingsWindow>) -> Row {
    let path = view.config_path().to_path_buf();
    let error = view.errors.get(CONFIG_FILE).cloned();
    let this = this.clone();
    Row::new("Config file", move |_, window, cx| {
        let palette = theme::palette(cx);
        let full = path.display().to_string();
        let shown = match std::env::home_dir().and_then(|home| path.strip_prefix(home).ok()) {
            Some(rest) => format!("~/{}", rest.display()),
            None => full.clone(),
        };
        let button = |id: &'static str, label: &'static str, name: &'static str, reveal: bool| {
            let this = this.clone();
            control_button(id, &palette, cx)
                .label(label)
                .accessibility_label(name)
                .on_click(move |_, _, cx| {
                    let _ = this.update(cx, |view, cx| view.open_config(reveal, cx));
                })
        };
        // Cut at its start, at a folder, so the file name stays in view; the whole path is in
        // the tooltip and told to assistive technology.
        let font_size = theme::text(theme::SMALL_TEXT_SIZE);
        let run = TextRun {
            len: 0,
            font: font(cx.theme().font_family.clone()),
            color: palette.muted,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let fits = |text: &str| {
            let run = TextRun {
                len: text.len(),
                ..run.clone()
            };
            let line = window.text_system().shape_line(
                SharedString::from(text.to_string()),
                font_size,
                &[run],
                None,
            );
            line.width <= px(theme::SETTINGS_CONFIG_PATH_WIDTH)
        };
        let shown = shorten_path(&shown, fits);
        let tooltip = SharedString::from(full.clone());
        let path_box = div()
            .id("config-path")
            .aria_label(full)
            .aria_value(shown.clone())
            .test_support()
            .flex_none()
            .whitespace_nowrap()
            .text_size(font_size)
            .text_color(palette.muted)
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(shown);
        div()
            .flex()
            .flex_col()
            .items_end()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(path_box)
                    .child(button("open", "Open", "Open config file", false))
                    .child(button(
                        "reveal",
                        "Show in folder",
                        "Show config file in folder",
                        true,
                    )),
            )
            .children(
                error
                    .clone()
                    .map(|error| error_line("config-error".into(), error, &palette)),
            )
    })
}

/// `path` as it fits: whole, or cut at its start at a folder (`…/tomakast/config.toml`), or,
/// when even the file name does not fit, at a character (`…ig.toml`).
fn shorten_path(path: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(path) {
        return path.to_string();
    }
    let at_folders = path
        .match_indices('/')
        .map(|(ix, _)| ix)
        .filter(|&ix| ix > 0)
        .map(|ix| format!("…{}", &path[ix..]));
    let at_characters = path
        .char_indices()
        .skip(1)
        .map(|(ix, _)| format!("…{}", &path[ix..]));
    at_folders
        .chain(at_characters)
        .find(|shown| fits(shown))
        .unwrap_or_else(|| "…".to_string())
}

/// The launcher hotkey; on the Launcher page only. Its field resets it and shows how it stands.
pub(super) fn launcher_hotkey(view: &SettingsWindow) -> Row {
    let field = view.hotkey.clone();
    row(view, key(LAUNCHER_HOTKEY), move |_, _, _| {
        field.clone().into_any_element()
    })
}

/// The leader key; on the Launcher page and on Hotkeys. Its field clears it and shows how it stands.
pub(super) fn leader_key(view: &SettingsWindow) -> Row {
    let field = view.leader.clone();
    row(view, key(LEADER), move |_, _, _| {
        field.clone().into_any_element()
    })
}

/// How the leader is used (hold it, or press it then a key): its list field; on the
/// Launcher page and on Hotkeys.
pub(super) fn leader_mode(view: &SettingsWindow, this: &WeakEntity<SettingsWindow>) -> Row {
    choice_row(view, this, LEADER_MODE)
}

pub(super) fn location(view: &SettingsWindow, this: &WeakEntity<SettingsWindow>) -> Page {
    let city = {
        let this = this.clone();
        entity_row(
            view,
            key(CITY),
            key_state(view, key(CITY)),
            view.city.clone(),
            move |window, cx| {
                let _ = this.update(cx, |view, cx| view.set_key(CITY, None, window, cx));
            },
        )
    };
    let time_zone = {
        let this = this.clone();
        entity_row(
            view,
            key(TIME_ZONE),
            key_state(view, key(TIME_ZONE)),
            view.time_zone.clone(),
            move |window, cx| {
                let _ = this.update(cx, |view, cx| view.set_key(TIME_ZONE, None, window, cx));
            },
        )
    };
    let page = Page::new(
        SettingsPage::Location,
        Section::Launcher,
        "Location",
        PageIcon::Named("map-pin"),
    )
    .group(
        Group::new()
            .row(city)
            .row(switch_row(view, this, WIFI_LOOKUP))
            .row(time_zone),
    );
    with_unrecognized(page, view, this, |path| path[0] == "location")
}

/// The reset button and the lines under the control: a value of the wrong type is shown as
/// the default in effect, with what the file holds.
pub(super) fn key_state(view: &SettingsWindow, key: &SettingKey) -> KeyState {
    let value = view.file.value(key);
    let mut lines = Vec::new();
    if let KeyValue::Invalid(value) = &value {
        lines.push(format!("Not valid here: {value}; the default is used"));
    }
    lines.extend(view.errors.get(&key.path.join(".")).cloned());
    KeyState {
        held: value != KeyValue::Missing,
        lines,
    }
}

/// A row whose control is one of the window's own fields.
pub(super) fn entity_row<V: Render>(
    view: &SettingsWindow,
    key: &'static SettingKey,
    state: KeyState,
    field: Entity<V>,
    reset: impl Fn(&mut Window, &mut App) + 'static,
) -> Row {
    let reset: Reset = Rc::new(reset);
    let render = move |options: RowOptions, _: &mut Window, cx: &mut App| {
        let control: AnyElement = field.clone().into_any_element();
        key_control(
            &state,
            key.title,
            control,
            options.disabled,
            reset.clone(),
            cx,
        )
    };
    row(view, key, render)
}

/// A boolean key: the kit's switch, written at once.
pub(super) fn switch_row(
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    path: &'static [&'static str],
) -> Row {
    let key = key(path);
    let state = key_state(view, key);
    let on = match view.file.value(key) {
        KeyValue::Valid(toml::Value::Boolean(on)) => on,
        _ => key.default == "true",
    };
    let reset: Reset = {
        let this = this.clone();
        Rc::new(move |window, cx| {
            let _ = this.update(cx, |view, cx| view.set_key(path, None, window, cx));
        })
    };
    let this = this.clone();
    let render = move |options: RowOptions, _: &mut Window, cx: &mut App| {
        let this = this.clone();
        let switch = Switch::new("switch")
            .accessibility_label(key.title)
            .checked(on)
            .disabled(options.disabled)
            .on_click(move |checked: &bool, window, cx| {
                let value = Some(toml::Value::Boolean(*checked));
                let _ = this.update(cx, |view, cx| view.set_key(path, value, window, cx));
            })
            .into_any_element();
        key_control(
            &state,
            key.title,
            switch,
            options.disabled,
            reset.clone(),
            cx,
        )
    };
    row(view, key, render)
}

/// The row for `key`: its title, description and search words (its key name among them);
/// disabled while the file has an error.
pub(super) fn row(
    view: &SettingsWindow,
    key: &'static SettingKey,
    render: impl Fn(RowOptions, &mut Window, &mut App) -> AnyElement + 'static,
) -> Row {
    let row = Row::new(key.title, render)
        .keywords(key.keywords.iter().copied())
        .keywords([key.path.join(".")])
        .disabled(view.file.error().is_some());
    match key.description {
        Some(description) => row.description(description),
        None => row,
    }
}

#[cfg(test)]
mod tests {
    use super::shorten_path;

    /// A fit of at most `n` characters.
    fn chars(n: usize) -> impl Fn(&str) -> bool {
        move |text: &str| text.chars().count() <= n
    }

    #[test]
    fn a_path_that_fits_is_whole() {
        assert_eq!(
            shorten_path("~/.config/tomakast/config.toml", chars(40)),
            "~/.config/tomakast/config.toml"
        );
    }

    #[test]
    fn a_long_path_is_cut_at_a_folder_from_its_start() {
        let path = "/run/user/1000/a-long-folder-name/tomakast/config.toml";
        assert_eq!(shorten_path(path, chars(25)), "…/tomakast/config.toml");
        assert_eq!(
            shorten_path(path, chars(45)),
            "…/a-long-folder-name/tomakast/config.toml"
        );
    }

    #[test]
    fn a_file_name_too_long_is_cut_at_a_character() {
        assert_eq!(shorten_path("/home/ə/config.toml", chars(8)), "…ig.toml");
        assert_eq!(shorten_path("/home/ə/config.toml", chars(0)), "…");
    }
}
