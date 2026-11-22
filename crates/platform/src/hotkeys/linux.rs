use std::collections::BTreeMap;

use super::{
    BindState, EventSink, HotkeyBackend, HotkeyBinding, HotkeyEvent, Registrar, SetKind,
    portal_shortcuts,
};
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use std::sync::mpsc;

/// The portal on desktops other than KDE: both sets are bound once, in one session, as soon
/// as both have arrived; later changes wait for a restart (the portal cannot drop a shortcut,
/// see `docs/experiments/2026-11-item-hotkeys-portal-probe.md`).
pub(super) struct PortalBackend {
    commands: mpsc::Sender<(SetKind, u64, Vec<HotkeyBinding>)>,
}

impl PortalBackend {
    pub(super) fn start(sink: EventSink) -> Result<PortalBackend, String> {
        let (commands, rx) = mpsc::channel::<(SetKind, u64, Vec<HotkeyBinding>)>();
        std::thread::Builder::new()
            .name("tomakast-portal-sets".into())
            .spawn(move || {
                let mut first: BTreeMap<SetKind, (u64, Vec<HotkeyBinding>)> = BTreeMap::new();
                let mut bound: BTreeMap<SetKind, Vec<HotkeyBinding>> = BTreeMap::new();
                while let Ok((set, generation, bindings)) = rx.recv() {
                    if let Some(earlier) = bound.get(&set) {
                        let states = later_states(earlier, &bindings);
                        sink(HotkeyEvent::Applied {
                            set,
                            generation,
                            outcome: Ok(states),
                        });
                        continue;
                    }
                    first.insert(set, (generation, bindings));
                    if first.len() < 2 {
                        continue;
                    }
                    let all: Vec<HotkeyBinding> = first
                        .values()
                        .flat_map(|(_, b)| b.iter().cloned())
                        .collect();
                    let press_sink = sink.clone();
                    let backend = register(all, move |id| press_sink(HotkeyEvent::Activated(id)));
                    for (set, (generation, bindings)) in std::mem::take(&mut first) {
                        let outcome = match &backend {
                            HotkeyBackend::Portal => Ok(bindings
                                .iter()
                                .map(|b| (b.id.clone(), BindState::Active { trigger: None }))
                                .collect()),
                            HotkeyBackend::Unsupported { reason } => Err(reason.clone()),
                        };
                        sink(HotkeyEvent::Applied {
                            set,
                            generation,
                            outcome,
                        });
                        bound.insert(set, bindings);
                    }
                    if let HotkeyBackend::Unsupported { reason } = backend {
                        sink(HotkeyEvent::Unavailable { reason });
                    }
                }
            })
            .map_err(|e| format!("cannot start the hotkey thread: {e}"))?;
        Ok(PortalBackend { commands })
    }
}

impl Registrar for PortalBackend {
    fn set(&mut self, set: SetKind, generation: u64, bindings: Vec<HotkeyBinding>) {
        let _ = self.commands.send((set, generation, bindings));
    }
}

/// After the one bind: what is bound as it was stays active, anything else waits.
fn later_states(bound: &[HotkeyBinding], wanted: &[HotkeyBinding]) -> Vec<(String, BindState)> {
    wanted
        .iter()
        .map(|b| {
            let state = if bound
                .iter()
                .any(|old| old.id == b.id && old.hotkey == b.hotkey)
            {
                BindState::Active { trigger: None }
            } else {
                BindState::Later
            };
            (b.id.clone(), state)
        })
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use tomakast_core::Hotkey;

    fn binding(id: &str, s: &str) -> HotkeyBinding {
        HotkeyBinding {
            id: id.into(),
            description: id.into(),
            hotkey: Hotkey::parse(s).unwrap(),
        }
    }

    #[test]
    fn later_changes_wait_for_a_restart() {
        let bound = [binding("a", "ctrl+alt+1"), binding("b", "ctrl+alt+2")];
        let wanted = [
            binding("a", "ctrl+alt+1"),
            binding("b", "ctrl+alt+3"),
            binding("c", "ctrl+alt+4"),
        ];
        assert_eq!(
            later_states(&bound, &wanted),
            [
                ("a".to_string(), BindState::Active { trigger: None }),
                ("b".to_string(), BindState::Later),
                ("c".to_string(), BindState::Later),
            ]
        );
    }
}
