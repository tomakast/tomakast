//! About: the name and the version of the launcher.

use gpui_kit::component::label::Label;
use gpui_kit::{FontWeight, ParentElement as _, Styled as _, div, px};

use crate::settings::SettingsPage;
use crate::settings::layout::{Group, Page, PageIcon, Row, Section};
use crate::theme::{self, Palette};

/// The workspace's version: `tomakast-ui` takes it from the workspace.
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(super) fn page(palette: &Palette) -> Page {
    let (text, muted) = (palette.text, palette.muted);
    // The name over its version, on the page itself: what the page is about, not a setting.
    let name = Row::custom(move |_, _, _| {
        div()
            .flex()
            .flex_col()
            .gap(px(theme::SETTINGS_DESCRIPTION_GAP))
            .child(
                Label::new("tomakast")
                    .text_size(theme::text(theme::HEADER_TEXT_SIZE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text),
            )
            .child(
                Label::new(format!("Version {VERSION}"))
                    .text_size(theme::text(theme::MUTED_TEXT_SIZE))
                    .text_color(muted),
            )
    });
    Page::new(
        SettingsPage::About,
        Section::Launcher,
        "About",
        PageIcon::Named("info"),
    )
    .group(Group::new().plain().row(name))
}
