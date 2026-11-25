//! The desktop's fonts: the family it sets for its interface and for monospace text. On macOS
//! the system's own (SF Pro, through GPUI's name for it, and SF Mono or Menlo). On KDE
//! `kdeglobals` (`[General] font` and `fixed`, Plasma's defaults when unset), on GNOME and the
//! desktops built on its settings `org.gnome.desktop.interface`, elsewhere fontconfig's match
//! for `sans-serif` or `monospace`. Only the family is read; sizes are the launcher's own.

/// Which of the desktop's fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontKind {
    /// The interface font.
    Ui,
    Monospace,
}

/// Plasma's fonts when `kdeglobals` names none.
const PLASMA_DEFAULTS: (&str, &str) = ("Noto Sans", "Hack");

/// Desktops whose interface font is GNOME's setting.
const GNOME_DESKTOPS: &[&str] = &["GNOME", "Unity", "Budgie", "X-Cinnamon", "Pantheon"];

/// The name GPUI gives the platform's interface font, whatever its family: on macOS SF Pro.
/// It is in no list of installed families, and GPUI always draws it.
pub const SYSTEM_UI_FAMILY: &str = ".SystemUIFont";

/// macOS's monospace families, the first `installed` one taken: SF Mono where it is installed
/// (with Xcode, or by the user), else Menlo, in every macOS since 10.6.
const MACOS_MONO: [&str; 2] = ["SF Mono", "Menlo"];

/// The family of the desktop's `kind` font, or `None` when it cannot be told. `installed`
/// tells whether a family is installed (macOS picks its monospace family by it).
pub fn system_font(kind: FontKind, installed: &dyn Fn(&str) -> bool) -> Option<String> {
    // Never fontconfig on a Mac: one installed there (Homebrew) answers Verdana.
    if cfg!(target_os = "macos") {
        return Some(macos_font(kind, installed).to_string());
    }
    let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let on = |names: &[&str]| desktops.split(':').any(|d| names.contains(&d));
    let found = if on(&["KDE"]) {
        Some(kde_font(&kdeglobals().unwrap_or_default(), kind))
    } else if on(GNOME_DESKTOPS) {
        gnome_font(kind)
    } else {
        None
    };
    found.or_else(|| fontconfig_font(kind))
}

/// macOS's `kind` font: the system UI font, or its first `installed` monospace family.
pub fn macos_font(kind: FontKind, installed: &dyn Fn(&str) -> bool) -> &'static str {
    match kind {
        FontKind::Ui => SYSTEM_UI_FAMILY,
        FontKind::Monospace => MACOS_MONO
            .into_iter()
            .find(|family| installed(family))
            .unwrap_or(MACOS_MONO[MACOS_MONO.len() - 1]),
    }
}

/// How a family the desktop names reads to the user: GPUI's name for the macOS interface font
/// as the font it stands for, any other family as it is.
pub fn font_display_name(family: &str) -> &str {
    if family == SYSTEM_UI_FAMILY {
        "SF Pro"
    } else {
        family
    }
}

/// `kdeglobals` under `XDG_CONFIG_HOME` (else `~/.config`).
fn kdeglobals() -> Option<String> {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".config")))?;
    std::fs::read_to_string(dir.join("kdeglobals")).ok()
}

/// The family of `[General] font` (or `fixed`) of a `kdeglobals` text: the first field of
/// Qt's font string (`Noto Sans,10,-1,5,400,…`); Plasma's default when the key is missing.
pub fn kde_font(text: &str, kind: FontKind) -> String {
    let wanted = match kind {
        FontKind::Ui => "font",
        FontKind::Monospace => "fixed",
    };
    let mut in_general = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_general = line == "[General]";
        } else if in_general
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == wanted
            && let Some(family) = value.split(',').next().map(str::trim)
            && !family.is_empty()
        {
            return family.to_string();
        }
    }
    match kind {
        FontKind::Ui => PLASMA_DEFAULTS.0,
        FontKind::Monospace => PLASMA_DEFAULTS.1,
    }
    .to_string()
}

/// GNOME's `font-name` or `monospace-font-name`, through `gsettings`.
fn gnome_font(kind: FontKind) -> Option<String> {
    let key = match kind {
        FontKind::Ui => "font-name",
        FontKind::Monospace => "monospace-font-name",
    };
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", key])
        .output()
        .ok()
        .filter(|out| out.status.success())?;
    pango_family(&String::from_utf8_lossy(&out.stdout))
}

/// The family of a Pango font description as `gsettings` prints it (`'Cantarell 11'`): the
/// text without its quotes and its trailing size.
pub fn pango_family(description: &str) -> Option<String> {
    let text = description.trim().trim_matches('\'').trim();
    let family = match text.rsplit_once(' ') {
        Some((family, size)) if size.parse::<f32>().is_ok() => family.trim(),
        _ => text,
    };
    (!family.is_empty()).then(|| family.to_string())
}

/// fontconfig's family for `sans-serif` or `monospace`.
fn fontconfig_font(kind: FontKind) -> Option<String> {
    let pattern = match kind {
        FontKind::Ui => "sans-serif",
        FontKind::Monospace => "monospace",
    };
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{family[0]}", pattern])
        .output()
        .ok()
        .filter(|out| out.status.success())?;
    let family = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!family.is_empty()).then_some(family)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kde_fonts_are_the_first_field() {
        let text = "[Colors:View]\nfont=Wrong,1\n\n[General]\nfixed=Iosevka Term,11,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\
                    font=Inter Display,10,-1,5,50,0,0,0,0,0\n";
        assert_eq!(kde_font(text, FontKind::Ui), "Inter Display");
        assert_eq!(kde_font(text, FontKind::Monospace), "Iosevka Term");
    }

    #[test]
    fn plasma_defaults_when_kdeglobals_names_none() {
        let text = "[General]\nColorScheme=BreezeDark\n";
        assert_eq!(kde_font(text, FontKind::Ui), "Noto Sans");
        assert_eq!(kde_font(text, FontKind::Monospace), "Hack");
        assert_eq!(kde_font("", FontKind::Ui), "Noto Sans");
    }

    #[test]
    fn macos_uses_the_system_ui_font() {
        assert_eq!(macos_font(FontKind::Ui, &|_| true), ".SystemUIFont");
        assert_eq!(macos_font(FontKind::Ui, &|_| false), ".SystemUIFont");
        assert_eq!(font_display_name(".SystemUIFont"), "SF Pro");
        assert_eq!(font_display_name("Noto Sans"), "Noto Sans");
    }

    #[test]
    fn macos_monospace_is_sf_mono_where_installed_else_menlo() {
        assert_eq!(
            macos_font(FontKind::Monospace, &|f| f == "SF Mono"),
            "SF Mono"
        );
        assert_eq!(
            macos_font(FontKind::Monospace, &|f| f == "Menlo" || f == "SF Mono"),
            "SF Mono"
        );
        assert_eq!(macos_font(FontKind::Monospace, &|f| f == "Menlo"), "Menlo");
        // Menlo ships with every macOS: it stands even when the list lacks it.
        assert_eq!(macos_font(FontKind::Monospace, &|_| false), "Menlo");
    }

    #[test]
    fn pango_descriptions_lose_their_size() {
        assert_eq!(
            pango_family("'Cantarell 11'\n").as_deref(),
            Some("Cantarell")
        );
        assert_eq!(
            pango_family("'Source Code Pro 10.5'").as_deref(),
            Some("Source Code Pro")
        );
        assert_eq!(pango_family("'Ubuntu'").as_deref(), Some("Ubuntu"));
        assert_eq!(pango_family("''"), None);
    }
}
