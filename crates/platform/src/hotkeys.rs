//! Global hotkeys: a long-lived [`HotkeyService`] binds two independent sets (the launcher's,
//! the items') and reports presses. Backends: KDE's kglobalaccel, the GlobalShortcuts portal
//! (other Wayland desktops, bound once), `global-hotkey` (X11, macOS, Windows) and a null one.

use std::sync::Arc;

use tomakast_core::Hotkey;

#[cfg(target_os = "linux")]
mod kde;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod native;
mod null;

/// One shortcut to register. `id` is `"launcher"` or `ItemId::as_str()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub id: String,
    pub description: String,
    pub hotkey: Hotkey,
}

/// The two sets the daemon binds independently.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SetKind {
    Launcher,
    Items,
}

/// What became of one binding after a `set`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindState {
    /// Bound. `trigger` is the system's own name for it, when it has one.
    Active { trigger: Option<String> },
    /// The system did not take it.
    Refused { reason: String },
    /// Accepted for later: this backend binds only at start.
    Later,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// A press of the binding with this id.
    Activated(String),
    /// The outcome of the `set` with this generation. `Err`: the whole set was not applied and
    /// the previous one stays bound.
    Applied {
        set: SetKind,
        generation: u64,
        outcome: Result<Vec<(String, BindState)>, String>,
    },
    /// No hotkeys in this run.
    Unavailable { reason: String },
}

/// Where the backends report.
pub type EventSink = Arc<dyn Fn(HotkeyEvent) + Send + Sync>;

/// Binds a whole set at a time; the outcome arrives as [`HotkeyEvent::Applied`].
pub trait Registrar {
    fn set(&mut self, set: SetKind, generation: u64, bindings: Vec<HotkeyBinding>);
}

/// The hotkey backend of this run.
pub struct HotkeyService {
    backend: Box<dyn Registrar>,
}

impl HotkeyService {
    /// Picks the backend: `TOMAKAST_NO_KEYBOARD=1` → null; Linux: kglobalaccel when KDE runs,
    /// else the portal on Wayland, else `global-hotkey` on X11, else null; macOS and Windows →
    /// `global-hotkey`. On macOS call it on the main thread; on Windows on a thread with a
    /// message loop (the GPUI `run` callback is both), and call [`Registrar::set`] there too.
    pub fn start(on_event: impl Fn(HotkeyEvent) + Send + Sync + 'static) -> HotkeyService {
        let sink: EventSink = Arc::new(on_event);
        if std::env::var_os("TOMAKAST_NO_KEYBOARD").is_some_and(|v| v == "1") {
            return HotkeyService::null("TOMAKAST_NO_KEYBOARD=1".into(), sink);
        }
        start_backend(sink)
    }

    fn null(reason: String, sink: EventSink) -> HotkeyService {
        HotkeyService {
            backend: Box::new(null::NullBackend::new(reason, sink)),
        }
    }
}

impl Registrar for HotkeyService {
    fn set(&mut self, set: SetKind, generation: u64, bindings: Vec<HotkeyBinding>) {
        self.backend.set(set, generation, bindings);
    }
}

#[cfg(target_os = "linux")]
fn start_backend(sink: EventSink) -> HotkeyService {
    let started = if kde::available() {
        kde::KdeBackend::start(kde::COMPONENT, sink.clone())
            .map(|b| Box::new(b) as Box<dyn Registrar>)
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        // Without the portal there is nothing to fall back to: XWayland grabs see only
        // XWayland windows' keys.
        linux::PortalBackend::start(sink.clone()).map(|b| Box::new(b) as Box<dyn Registrar>)
    } else if std::env::var_os("DISPLAY").is_some() {
        native::NativeBackend::new(sink.clone()).map(|b| Box::new(b) as Box<dyn Registrar>)
    } else {
        Err("no graphical session".into())
    };
    match started {
        Ok(backend) => HotkeyService { backend },
        Err(reason) => HotkeyService::null(reason, sink),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn start_backend(sink: EventSink) -> HotkeyService {
    match native::NativeBackend::new(sink.clone()) {
        Ok(backend) => HotkeyService {
            backend: Box::new(backend),
        },
        Err(reason) => HotkeyService::null(reason, sink),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn start_backend(sink: EventSink) -> HotkeyService {
    HotkeyService::null("global hotkeys are not supported on this OS".into(), sink)
}

/// Removes tomakast's hotkeys from KDE's shortcut service, which keeps them grabbed after
/// the daemon exits. Elsewhere the system drops them with the process: nothing to do.
pub fn release_hotkeys() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if kde::available() {
        return kde::release(kde::COMPONENT);
    }
    Ok(())
}

/// The outcome of the portal's one-time bind.
#[cfg(target_os = "linux")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HotkeyBackend {
    Portal,
    Unsupported { reason: String },
}

/// Pure mapping to portal shortcuts: `(id, description, preferred trigger)`. A binding whose
/// key has no portal name is skipped with a warning.
pub fn portal_shortcuts(bindings: &[HotkeyBinding]) -> Vec<(String, String, String)> {
    bindings
        .iter()
        .filter_map(|b| match b.hotkey.to_xdg_trigger() {
            Some(trigger) => Some((b.id.clone(), b.description.clone(), trigger)),
            None => {
                tracing::warn!(id = b.id, hotkey = %b.hotkey, "hotkey key has no portal name; skipped");
                None
            }
        })
        .collect()
}
