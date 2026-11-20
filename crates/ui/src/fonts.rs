//! The fonts the launcher ships: Inter for its text and JetBrains Mono for
//! monospace text, embedded in the binary so the launcher looks the same
//! whatever the machine has installed. Sources and licenses are in
//! `crates/ui/assets/fonts/`.

use std::borrow::Cow;

use gpui_kit::App;
use gpui_kit::component::Theme;

/// The family of the launcher's text.
pub(crate) const UI_FAMILY: &str = "Inter";
/// The family of its monospace text.
pub(crate) const MONO_FAMILY: &str = "JetBrains Mono";

/// Every bundled face: the weights and styles the launcher draws. Inter's
/// italics are for markdown emphasis; monospace text is never lighter than
/// regular nor bolder than bold.
const FACES: [&[u8]; 10] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Inter-Bold.ttf"),
    include_bytes!("../assets/fonts/Inter-Italic.ttf"),
    include_bytes!("../assets/fonts/Inter-BoldItalic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-BoldItalic.ttf"),
];

/// Adds the bundled faces to the text system. Before the kit's init: it
/// lists the installed families once, and the bundled ones must be there.
pub(crate) fn register(cx: &mut App) {
    let faces = FACES.iter().map(|face| Cow::Borrowed(*face)).collect();
    if let Err(err) = cx.text_system().add_fonts(faces) {
        tracing::error!("Could not add the bundled fonts: {err:#}");
    }
}

/// Names the bundled families on the theme. Its light and dark configs name
/// no family, so a mode change keeps them.
pub(crate) fn name_on_theme(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.font_family = UI_FAMILY.into();
        theme.mono_font_family = MONO_FAMILY.into();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{ActiveTheme as _, ThemeMode};

    /// The faces of `family` in the bundle alone (no system font), as
    /// (weight, italic).
    fn faces_of(family: &str) -> Vec<(u16, bool)> {
        let mut db = fontdb::Database::new();
        for face in FACES {
            db.load_font_data(face.to_vec());
        }
        assert_eq!(db.len(), FACES.len(), "every blob is one font face");
        let mut faces: Vec<_> = db
            .faces()
            .filter(|face| face.families.iter().any(|(name, _)| name == family))
            .map(|face| (face.weight.0, face.style != fontdb::Style::Normal))
            .collect();
        faces.sort_unstable();
        faces
    }

    #[test]
    fn the_bundle_has_every_weight_the_launcher_draws() {
        assert_eq!(
            faces_of(UI_FAMILY),
            [
                (400, false),
                (400, true),
                (500, false),
                (600, false),
                (700, false),
                (700, true)
            ]
        );
        assert_eq!(
            faces_of(MONO_FAMILY),
            [(400, false), (400, true), (700, false), (700, true)]
        );
    }

    /// JetBrains Mono's ligatures are contextual alternates (`calt`), which
    /// `theme::mono_features` turns on.
    #[test]
    fn the_mono_faces_have_ligatures() {
        for face in &FACES[6..] {
            let face = ttf_parser::Face::parse(face, 0).expect("a font");
            let gsub = face.tables().gsub.expect("a GSUB table");
            let calt = ttf_parser::Tag::from_bytes(b"calt");
            assert!(gsub.features.index(calt).is_some(), "no calt feature");
        }
    }

    #[gpui_kit::test]
    fn init_names_the_bundled_families_for_good(cx: &mut gpui_kit::TestAppContext) {
        cx.update(crate::init);
        let families = |cx: &mut gpui_kit::TestAppContext| {
            cx.read(|cx| {
                let theme = cx.theme();
                (
                    theme.font_family.to_string(),
                    theme.mono_font_family.to_string(),
                )
            })
        };
        let bundled = (UI_FAMILY.to_owned(), MONO_FAMILY.to_owned());
        assert_eq!(families(cx), bundled);
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            cx.update(|cx| Theme::change(mode, None, cx));
            assert_eq!(families(cx), bundled, "after switching to {mode:?}");
        }
    }
}
