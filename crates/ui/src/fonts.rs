//! The fonts the launcher ships: Inter for its text and JetBrains Mono for
//! monospace text, embedded in the binary so the launcher looks the same
//! whatever the machine has installed. Sources and licenses are in
//! `crates/ui/assets/fonts/`.

use std::borrow::Cow;

use gpui_kit::App;
use gpui_kit::component::{ActiveTheme as _, Theme};
use tomakast_core::{FontChoice, TextSettings};
use tomakast_platform::FontKind;

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

/// The kit's root text size at the default text size: its rems.
const KIT_FONT_SIZE: f32 = 16.;

/// Applies the user's fonts and text sizes (`[appearance]`) to every window: the families on
/// the theme, the text size factor and the monospace size. A family that is not installed
/// (or a desktop font that cannot be told) is the bundled one. Redraws the open windows when
/// anything changed.
pub fn set_text_preference(text: &TextSettings, cx: &mut App) {
    let installed = |family: &str| {
        cx.text_system()
            .all_font_names()
            .iter()
            .any(|f| f == family)
    };
    let ui = family(&text.font, UI_FAMILY, FontKind::Ui, &installed);
    let mono = family(
        &text.mono_font,
        MONO_FAMILY,
        FontKind::Monospace,
        &installed,
    );
    let scale = text.size.scale();
    let sizes = crate::theme::set_text_sizes(scale, text.mono_size);
    let theme = cx.theme();
    let families = theme.font_family.as_ref() != ui || theme.mono_font_family.as_ref() != mono;
    if !(sizes || families) {
        return;
    }
    tracing::info!(font = %ui, mono = %mono, scale, mono_size = text.mono_size, "text");
    Theme::update(cx, |theme| {
        theme.font_family = ui.into();
        theme.mono_font_family = mono.into();
        theme.font_size = gpui_kit::px(KIT_FONT_SIZE * scale);
        theme.mono_font_size = gpui_kit::px(f32::from(text.mono_size));
    });
    cx.refresh_windows();
}

/// The family `choice` names, if `installed` (GPUI's system UI font always is); else
/// `bundled`.
fn family(
    choice: &FontChoice,
    bundled: &str,
    kind: FontKind,
    installed: &dyn Fn(&str) -> bool,
) -> String {
    let wanted = match choice {
        FontChoice::Bundled => None,
        FontChoice::System => tomakast_platform::system_font(kind, installed),
        FontChoice::Family(family) => Some(family.clone()),
    };
    match wanted {
        // GPUI's name for the platform's interface font is in no list, and always drawn.
        Some(family)
            if family == bundled
                || family == tomakast_platform::SYSTEM_UI_FAMILY
                || installed(&family) =>
        {
            family
        }
        Some(family) => {
            tracing::warn!(%family, "font not installed; using {bundled}");
            bundled.to_string()
        }
        None => bundled.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::ThemeMode;

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

    /// GPUI's name for the platform's interface font (the macOS default) is in no list of
    /// installed families, yet stands; another family missing from the list is the bundled one.
    #[test]
    fn the_system_ui_font_stands_though_no_list_has_it() {
        let none = |_: &str| false;
        let system = FontChoice::Family(tomakast_platform::SYSTEM_UI_FAMILY.into());
        assert_eq!(
            family(&system, UI_FAMILY, FontKind::Ui, &none),
            ".SystemUIFont"
        );
        let missing = FontChoice::Family("No Such Sans".into());
        assert_eq!(family(&missing, UI_FAMILY, FontKind::Ui, &none), UI_FAMILY);
        assert_eq!(
            family(
                &FontChoice::Bundled,
                MONO_FAMILY,
                FontKind::Monospace,
                &none
            ),
            MONO_FAMILY
        );
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
