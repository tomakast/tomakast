//! The bundled icon set and the monogram drawn when an item has no icon.
//!
//! A theme icon name (`Icon::Named`) is a Lucide icon name (lucide.dev, the
//! catalog pinned by gpui-kit-assets), the same on every OS. A name outside
//! the bundle, an icon that cannot be drawn and no icon at all show a
//! monogram instead: a letter on a neutral rounded square.

use std::collections::HashSet;
use std::sync::OnceLock;

use gpui_kit::AssetSource;
use gpui_kit::assets::AllAssets;

/// The asset source the application registers: the whole Lucide catalog,
/// so a plugin may name any icon in it.
pub fn assets() -> impl AssetSource {
    AllAssets
}

/// The asset path of the bundled icon `name`, if the bundle has it. Only
/// plain Lucide file names (`[a-z0-9-]`) are looked up, so a name never
/// reaches another path of the asset source.
pub(crate) fn bundled_path(name: &str) -> Option<String> {
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    let plain = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !plain {
        return None;
    }
    let names = NAMES.get_or_init(|| {
        AllAssets
            .list("icons/")
            .unwrap_or_default()
            .into_iter()
            .filter_map(|path| {
                let name = path.strip_prefix("icons/")?.strip_suffix(".svg")?;
                Some(name.to_string())
            })
            .collect()
    });
    names.contains(name).then(|| format!("icons/{name}.svg"))
}

/// An item's icon, and the text whose monogram is drawn when it has none
/// or it cannot be drawn.
#[derive(Clone, Copy)]
pub(crate) struct IconSpec<'a> {
    pub icon: Option<&'a tomakast_core::Icon>,
    pub monogram: &'a str,
}

/// A monogram's letter: the first letter or digit of `text`, uppercased;
/// nothing when `text` has none.
pub(crate) fn monogram_letter(text: &str) -> Option<String> {
    let c = text.chars().find(|c| c.is_alphanumeric())?;
    Some(c.to_uppercase().collect())
}

#[cfg(test)]
mod tests {
    use super::{bundled_path, monogram_letter};

    #[test]
    fn bundled_names_resolve_to_their_svg() {
        assert_eq!(bundled_path("bell").as_deref(), Some("icons/bell.svg"));
        assert_eq!(
            bundled_path("face-slightly-smiling").as_deref(),
            Some("icons/face-slightly-smiling.svg")
        );
    }

    #[test]
    fn only_plain_bundled_names_resolve() {
        for name in [
            "",
            "no-such-icon",
            "Bell",
            "bell.svg",
            "icons/bell",
            "../icons/bell",
            "..",
            "bell\0",
            " bell",
        ] {
            assert_eq!(bundled_path(name), None, "{name:?}");
        }
    }

    #[test]
    fn monogram_letter_is_the_first_alphanumeric_uppercased() {
        let letter = monogram_letter;
        assert_eq!(letter("kitchen sink").as_deref(), Some("K"));
        assert_eq!(letter("\u{200b}\u{202e}éclair").as_deref(), Some("É"));
        assert_eq!(letter("  42 apples").as_deref(), Some("4"));
        assert_eq!(letter("ß").as_deref(), Some("SS"));
        assert_eq!(letter("--- !"), None);
        assert_eq!(letter(""), None);
    }
}
