//! The window buttons KWin puts in a title bar, and on which side: the settings window draws
//! its own title area on KDE and places its controls the way the desktop's own windows have
//! them.

/// A window control of a title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameButton {
    Minimize,
    Maximize,
    Close,
}

/// KWin's buttons when a key is missing: menu and on-all-desktops on the left; context help,
/// minimize, maximize and close on the right.
const DEFAULT_LEFT: &str = "MS";
const DEFAULT_RIGHT: &str = "HIAX";

/// The buttons on the left and on the right of a title bar, from the
/// `[org.kde.kdecoration2]` group of a `kwinrc` text (`ButtonsOnLeft`, `ButtonsOnRight`).
/// Letters other than `I` (minimize), `A` (maximize) and `X` (close) are dropped, and so is
/// a button already placed; a missing key, or group, takes KWin's default.
pub fn kwin_buttons(text: &str) -> (Vec<FrameButton>, Vec<FrameButton>) {
    let (mut left, mut right) = (None, None);
    let mut in_group = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[org.kde.kdecoration2]";
        } else if in_group && let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "ButtonsOnLeft" => left = Some(value.trim()),
                "ButtonsOnRight" => right = Some(value.trim()),
                _ => {}
            }
        }
    }
    let mut placed = Vec::new();
    let mut side = |letters: &str| {
        let mut buttons = Vec::new();
        for button in letters.chars().filter_map(button) {
            if !placed.contains(&button) {
                placed.push(button);
                buttons.push(button);
            }
        }
        buttons
    };
    let left = side(left.unwrap_or(DEFAULT_LEFT));
    let right = side(right.unwrap_or(DEFAULT_RIGHT));
    (left, right)
}

/// The button KWin's letter `c` stands for, among those the settings window draws.
fn button(c: char) -> Option<FrameButton> {
    match c {
        'I' => Some(FrameButton::Minimize),
        'A' => Some(FrameButton::Maximize),
        'X' => Some(FrameButton::Close),
        _ => None,
    }
}

/// The buttons of KWin's title bars, on KDE (`XDG_CURRENT_DESKTOP` names it):
/// [`kwin_buttons`] of `kwinrc` under `XDG_CONFIG_HOME` (else `~/.config`), of no text when
/// it cannot be read. `None` on other desktops and outside Linux.
pub fn kwin_button_layout() -> Option<(Vec<FrameButton>, Vec<FrameButton>)> {
    #[cfg(target_os = "linux")]
    {
        use crate::icon_theme::{config_home, current_desktops};
        if !current_desktops().iter().any(|desktop| desktop == "KDE") {
            return None;
        }
        let text = config_home()
            .and_then(|dir| std::fs::read_to_string(dir.join("kwinrc")).ok())
            .unwrap_or_default();
        Some(kwin_buttons(&text))
    }
    #[cfg(not(target_os = "linux"))]
    None
}

#[cfg(test)]
mod tests {
    use super::FrameButton::{Close, Maximize as Max, Minimize as Min};
    use super::*;

    #[test]
    fn defaults_when_keys_are_missing() {
        assert_eq!(
            kwin_buttons("[org.kde.kdecoration2]\n"),
            (vec![], vec![Min, Max, Close])
        );
        assert_eq!(kwin_buttons(""), (vec![], vec![Min, Max, Close]));
    }

    #[test]
    fn only_right_set() {
        assert_eq!(
            kwin_buttons("[org.kde.kdecoration2]\nButtonsOnRight=HIAX\n"),
            (vec![], vec![Min, Max, Close])
        );
    }

    #[test]
    fn left_side_close() {
        assert_eq!(
            kwin_buttons("[org.kde.kdecoration2]\nButtonsOnLeft=XIA\nButtonsOnRight=\n"),
            (vec![Close, Min, Max], vec![])
        );
    }

    #[test]
    fn unknown_letters_dropped() {
        assert_eq!(
            kwin_buttons("[org.kde.kdecoration2]\nButtonsOnLeft=\nButtonsOnRight=MSBFHLIAX\n"),
            (vec![], vec![Min, Max, Close])
        );
    }

    #[test]
    fn duplicates_dropped() {
        // Close set on the left leaves the default right side without it.
        assert_eq!(
            kwin_buttons("[org.kde.kdecoration2]\nButtonsOnLeft=XX\n"),
            (vec![Close], vec![Min, Max])
        );
    }

    #[test]
    fn other_groups_ignored() {
        let text = "[Windows]\nButtonsOnLeft=X\n\n[org.kde.kdecoration2]\n BorderSize = Normal \n \
                    ButtonsOnLeft = I \n[Plugins]\nButtonsOnRight=X\n";
        assert_eq!(kwin_buttons(text), (vec![Min], vec![Max, Close]));
    }
}
