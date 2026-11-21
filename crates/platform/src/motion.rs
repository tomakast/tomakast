//! Whether the desktop asks for reduced motion: an animation the user did not start (the
//! Home edit mode's jiggle) stays still when it does.
//!
//! `TOMAKAST_REDUCED_MOTION` decides first, on every OS: `1` asks for reduced motion, `0`
//! for motion. On Linux, KDE's `[KDE] AnimationDurationFactor` in `kdeglobals` comes next
//! (0 turns animations off), then the Settings portal's
//! `org.gnome.desktop.interface enable-animations`. Elsewhere only the variable is read,
//! for now.

/// The variable that overrides the desktop's setting.
pub const REDUCED_MOTION_ENV: &str = "TOMAKAST_REDUCED_MOTION";

/// Whether the desktop asks for reduced motion: `Some(true)` when it does, `Some(false)` when
/// it says it does not, `None` when it cannot be read. Blocking (it may wait up to 500 ms for
/// the portal): call it off the UI thread.
pub fn reduced_motion() -> Option<bool> {
    if let Some(reduced) = env_reduced(std::env::var(REDUCED_MOTION_ENV).ok().as_deref()) {
        return Some(reduced);
    }
    #[cfg(target_os = "linux")]
    {
        linux::kde().or_else(linux::portal)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// What `TOMAKAST_REDUCED_MOTION` says, given its value: `1` reduced, `0` not; anything
/// else, or no variable, says nothing.
fn env_reduced(value: Option<&str>) -> Option<bool> {
    match value?.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// What a `kdeglobals` file says: `[KDE] AnimationDurationFactor` 0 is reduced motion, any
/// other number is not; no factor, or one that is no number, says nothing.
#[cfg(target_os = "linux")]
fn kde_reduced(kdeglobals: &str) -> Option<bool> {
    let factor: f32 = crate::icon_theme::ini_value(kdeglobals, "KDE", "AnimationDurationFactor")?
        .parse()
        .ok()?;
    Some(factor == 0.)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::time::Duration;

    use futures_util::future::{Either, select};

    use crate::icon_theme::{KDEGLOBALS, config_home, current_desktops};

    /// How long the portal may take to answer.
    pub(super) const PORTAL_TIMEOUT: Duration = Duration::from_millis(500);

    /// KDE's setting, on KDE only: the first `kdeglobals` that names a factor.
    pub(super) fn kde() -> Option<bool> {
        if !current_desktops().iter().any(|d| d == "KDE") {
            return None;
        }
        let config_home = config_home()?;
        KDEGLOBALS.iter().find_map(|rel| {
            let text = std::fs::read_to_string(config_home.join(rel)).ok()?;
            super::kde_reduced(&text)
        })
    }

    /// The Settings portal's `enable-animations`, reduced when it is off; `None` without a
    /// portal, without the key, or with no answer within [`PORTAL_TIMEOUT`].
    pub(super) fn portal() -> Option<bool> {
        async_io::block_on(async {
            let read = std::pin::pin!(async {
                let settings = ashpd::desktop::settings::Settings::new().await.ok()?;
                settings
                    .read::<bool>("org.gnome.desktop.interface", "enable-animations")
                    .await
                    .ok()
            });
            let timer = std::pin::pin!(async_io::Timer::after(PORTAL_TIMEOUT));
            match select(read, timer).await {
                Either::Left((animations, _)) => animations.map(|on| !on),
                Either::Right(_) => {
                    tracing::debug!("the Settings portal did not answer in time");
                    None
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn kde_animation_factor_zero_is_reduced() {
        assert_eq!(
            kde_reduced("[KDE]\nAnimationDurationFactor=0\n"),
            Some(true)
        );
        assert_eq!(
            kde_reduced("[General]\nX=1\n\n[KDE]\nAnimationDurationFactor=0.0\n"),
            Some(true)
        );
        assert_eq!(
            kde_reduced("[KDE]\nAnimationDurationFactor=0.5\n"),
            Some(false)
        );
        // Only the `[KDE]` group counts; no factor, or one that is no number, says nothing.
        assert_eq!(
            kde_reduced("[Other]\nAnimationDurationFactor=0\n[KDE]\nSingleClick=true\n"),
            None
        );
        assert_eq!(kde_reduced("[KDE]\nAnimationDurationFactor=fast\n"), None);
        assert_eq!(kde_reduced(""), None);
    }

    #[test]
    fn env_override_wins() {
        assert_eq!(env_reduced(Some("1")), Some(true));
        assert_eq!(env_reduced(Some("0")), Some(false));
        assert_eq!(env_reduced(Some("")), None);
        assert_eq!(env_reduced(None), None);
        assert_eq!(env_reduced(Some("maybe")), None);
    }
}
