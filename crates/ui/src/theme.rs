//! Launcher colors. They come from the gpui-component theme, whose light or
//! dark mode follows the system appearance reported by the launcher window.

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::{App, Hsla, Window};

/// Corner radius of the launcher panel.
pub(crate) const PANEL_RADIUS: f32 = 14.;
/// Opacity of the panel fill; the desktop shows through it slightly.
const PANEL_OPACITY: f32 = 0.92;

/// Switches the theme to the window's appearance. `window.appearance()` is
/// used rather than `cx.window_appearance()`, which is unreliable on Linux.
/// The theme is only changed when the mode differs, because a change
/// refreshes every window.
pub(crate) fn sync(window: &Window, cx: &mut App) {
    let mode = ThemeMode::from(window.appearance());
    if cx.theme().mode.is_dark() != mode.is_dark() {
        Theme::change(mode, None, cx);
    }
}

/// Colors the launcher view paints with.
#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub panel: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub selected: Hsla,
    pub error: Hsla,
}

pub(crate) fn palette(cx: &App) -> Palette {
    let theme = cx.theme();
    Palette {
        panel: theme.background.opacity(PANEL_OPACITY),
        border: theme.border,
        text: theme.foreground,
        muted: theme.muted_foreground,
        selected: theme.list_active,
        error: theme.danger,
    }
}
