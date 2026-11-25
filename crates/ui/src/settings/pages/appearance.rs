//! Appearance: the theme, then the text: the font, the text size, the monospace font and its
//! size. Each is written at once; the daemon applies the written values when it reloads the
//! file.

use gpui_kit::WeakEntity;

use super::general::{entity_row, key_state};
use crate::settings::SettingsPage;
use crate::settings::layout::{Group, Page, PageIcon, Row, Section};
use crate::settings::window::{FONT, MONO_FONT, MONO_SIZE, SettingsWindow, TEXT_SIZE, THEME, key};

pub(super) fn page(view: &SettingsWindow, this: &WeakEntity<SettingsWindow>) -> Page {
    let field_row = |path: &'static [&'static str], field| {
        let this = this.clone();
        entity_row(
            view,
            key(path),
            key_state(view, key(path)),
            field,
            move |window, cx| {
                let _ = this.update(cx, |view, cx| view.set_key(path, None, window, cx));
            },
        )
    };
    let font = field_row(FONT, view.font.clone());
    let mono_font = field_row(MONO_FONT, view.mono_font.clone());
    let mono_size = {
        let this = this.clone();
        entity_row(
            view,
            key(MONO_SIZE),
            key_state(view, key(MONO_SIZE)),
            view.mono_size.clone(),
            move |window, cx| {
                let _ = this.update(cx, |view, cx| view.set_key(MONO_SIZE, None, window, cx));
            },
        )
    };
    Page::new(
        SettingsPage::Appearance,
        Section::Launcher,
        "Appearance",
        PageIcon::Named("palette"),
    )
    .group(Group::new().row(choice_row(view, this, THEME)))
    .group(
        Group::new()
            .title("Text")
            .row(font)
            .row(choice_row(view, this, TEXT_SIZE))
            .row(mono_font)
            .row(mono_size),
    )
}

/// The row of the built-in choice key at `path`: its list field, written at once.
pub(super) fn choice_row(
    view: &SettingsWindow,
    this: &WeakEntity<SettingsWindow>,
    path: &'static [&'static str],
) -> Row {
    let this = this.clone();
    entity_row(
        view,
        key(path),
        key_state(view, key(path)),
        view.choice(path),
        move |window, cx| {
            let _ = this.update(cx, |view, cx| view.set_key(path, None, window, cx));
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::settings::window::{THEME, THEME_ICONS, key};
    use tomakast_core::settings_schema::ValueKind;

    /// Every theme has an icon, and the bundle has it.
    #[test]
    fn every_theme_has_a_bundled_icon() {
        let ValueKind::Choice(choices) = &key(THEME).kind else {
            unreachable!("the theme is a choice");
        };
        for choice in choices {
            let (_, name) = THEME_ICONS
                .iter()
                .find(|(value, _)| *value == choice.value)
                .unwrap_or_else(|| panic!("{} has no icon", choice.value));
            assert!(
                crate::icons::bundled_path(name).is_some(),
                "{name} is not bundled"
            );
        }
    }
}
