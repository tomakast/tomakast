use super::{HotkeyBackend, HotkeyBinding, portal_shortcuts};
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use std::sync::mpsc;

/// Same as the launcher window app_id.
const APP_ID: &str = "dev.tomakast.Tomakast";

pub fn register(
    bindings: Vec<HotkeyBinding>,
    on_activate: impl Fn(String) + Send + 'static,
) -> HotkeyBackend {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let spawned = std::thread::Builder::new()
        .name("tomakast-hotkeys".into())
        .spawn(move || {
            async_io::block_on(async move {
                if let Err(reason) = run(bindings, &on_activate, &ready_tx).await {
                    // Ignored if `run` already reported readiness.
                    let _ = ready_tx.send(Err(reason));
                }
            })
        });
    if let Err(e) = spawned {
        return HotkeyBackend::Unsupported {
            reason: format!("cannot start hotkey thread: {e}"),
        };
    }
    match ready_rx.recv() {
        Ok(Ok(())) => HotkeyBackend::Portal,
        Ok(Err(reason)) => {
            tracing::warn!(%reason, "global hotkeys unavailable");
            HotkeyBackend::Unsupported { reason }
        }
        Err(_) => HotkeyBackend::Unsupported {
            reason: "hotkey thread exited before binding".into(),
        },
    }
}

async fn run(
    bindings: Vec<HotkeyBinding>,
    on_activate: &(impl Fn(String) + Send + 'static),
    ready: &mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    let err = |what: &str, e: ashpd::Error| format!("{what}: {e}");
    match ashpd::AppID::try_from(APP_ID) {
        Ok(id) => {
            if let Err(e) = ashpd::register_host_app(id).await {
                tracing::debug!(error = %e, "host app registration failed; continuing");
            }
        }
        Err(e) => tracing::debug!(error = %e, "invalid host app id; continuing"),
    }
    let portal = GlobalShortcuts::new()
        .await
        .map_err(|e| err("GlobalShortcuts portal unavailable", e))?;
    let session = portal
        .create_session(Default::default())
        .await
        .map_err(|e| err("create_session failed", e))?;
    let shortcuts: Vec<NewShortcut> = portal_shortcuts(&bindings)
        .into_iter()
        .map(|(id, description, trigger)| {
            NewShortcut::new(id, description).preferred_trigger(trigger.as_str())
        })
        .collect();
    portal
        .bind_shortcuts(&session, &shortcuts, None, Default::default())
        .await
        .map_err(|e| err("bind_shortcuts failed", e))?
        .response()
        .map_err(|e| err("bind_shortcuts refused", e))?;
    let mut activated = portal
        .receive_activated()
        .await
        .map_err(|e| err("receive_activated failed", e))?;
    let _ = ready.send(Ok(()));
    while let Some(ev) = activated.next().await {
        on_activate(ev.shortcut_id().to_string());
    }
    // E.g. the portal service restarted; nothing re-binds the shortcuts.
    tracing::warn!("global shortcut stream ended; hotkeys are no longer active");
    drop(session);
    Ok(())
}
