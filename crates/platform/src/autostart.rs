//! Launch-at-login. Linux: an XDG autostart `.desktop` file.

use std::path::Path;

/// Writes (`enabled`) or removes the autostart entry inside `dir` (`$XDG_CONFIG_HOME/autostart`).
#[cfg(target_os = "linux")]
pub fn set_autostart_in(dir: &Path, enabled: bool, exe: &Path) -> anyhow::Result<()> {
    let file = dir.join("tomakast.desktop");
    if enabled {
        std::fs::create_dir_all(dir)?;
        let contents = format!(
            "[Desktop Entry]\nType=Application\nName=tomakast\nExec=\"{}\" daemon\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        std::fs::write(&file, contents)?;
    } else {
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn set_autostart_in(_dir: &Path, _enabled: bool, _exe: &Path) -> anyhow::Result<()> {
    tracing::warn!("autostart is not implemented on this OS yet");
    Ok(())
}

/// Enables or disables autostart for the current executable in the user's autostart dir.
pub fn set_autostart(enabled: bool) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            Some(dir) => std::path::PathBuf::from(dir),
            None => std::path::PathBuf::from(
                std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?,
            )
            .join(".config"),
        };
        set_autostart_in(
            &config.join("autostart"),
            enabled,
            &std::env::current_exe()?,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        set_autostart_in(Path::new(""), enabled, Path::new(""))
    }
}
