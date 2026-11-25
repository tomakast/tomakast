use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use super::{
    BindState, EventSink, HotkeyBackend, HotkeyBinding, HotkeyEvent, MODIFIER_ALONE, Registrar,
    SET_COUNT, SetKind, portal_shortcuts,
};
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use std::sync::mpsc;

/// The portal on desktops other than KDE: all sets are bound once, in one session, as soon
/// as all have arrived; later changes wait for a restart (the portal cannot drop a shortcut,
/// see `docs/experiments/2026-11-item-hotkeys-portal-probe.md`). A modifier alone is refused:
/// the portal's triggers name a key.
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
                    if first.len() < SET_COUNT {
                        continue;
                    }
                    let all: Vec<HotkeyBinding> = first
                        .values()
                        .flat_map(|(_, b)| b.iter())
                        .filter(|b| !b.hotkey.is_modifiers_only())
                        .cloned()
                        .collect();
                    let backend = register(all, sink.clone());
                    for (set, (generation, bindings)) in std::mem::take(&mut first) {
                        let outcome = match &backend {
                            HotkeyBackend::Portal => Ok(bindings
                                .iter()
                                .map(|b| (b.id.clone(), bound_state(b)))
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

/// The state of `binding` after the one bind took it (with the others).
fn bound_state(binding: &HotkeyBinding) -> BindState {
    if binding.hotkey.is_modifiers_only() {
        BindState::Refused {
            reason: MODIFIER_ALONE.into(),
        }
    } else {
        BindState::Active { trigger: None }
    }
}

/// After the one bind: what is bound as it was stays active, anything else waits.
fn later_states(bound: &[HotkeyBinding], wanted: &[HotkeyBinding]) -> Vec<(String, BindState)> {
    wanted
        .iter()
        .map(|b| {
            let state = if b.hotkey.is_modifiers_only()
                || bound
                    .iter()
                    .any(|old| old.id == b.id && old.hotkey == b.hotkey)
            {
                bound_state(b)
            } else {
                BindState::Later
            };
            (b.id.clone(), state)
        })
        .collect()
}

/// A portal shortcut signal.
#[derive(Clone, Copy, Debug)]
enum Signal {
    Activated,
    Deactivated,
}

/// The event of `signal` for the shortcut `id`.
fn event_of(signal: Signal, id: String) -> HotkeyEvent {
    match signal {
        Signal::Activated => HotkeyEvent::Activated(id),
        Signal::Deactivated => HotkeyEvent::Released(id),
    }
}

/// Puts the portal's two signal streams back in order. They are polled in turn, so a release
/// can come out before its own press; that press, delivered late, would leave the leader held.
#[derive(Debug, Default)]
struct Releases {
    /// Per shortcut id, the time of the latest release delivered.
    last: HashMap<String, Duration>,
}

impl Releases {
    /// Whether to deliver `signal` of `id`, stamped `at` by the portal. A press strictly older
    /// than a release already delivered for the same id is dropped; a release is always
    /// delivered. A time of zero is unknown (the portal does not fix the clock), so a press
    /// stamped zero is kept.
    fn keep(&mut self, signal: Signal, id: &str, at: Duration) -> bool {
        match signal {
            Signal::Activated => {
                at.is_zero() || self.last.get(id).is_none_or(|released| at >= *released)
            }
            Signal::Deactivated => {
                let released = self.last.entry(id.to_string()).or_default();
                *released = (*released).max(at);
                true
            }
        }
    }
}

/// Same as the launcher window app_id.
const APP_ID: &str = "dev.tomakast.Tomakast";

/// Binds `bindings` in a new portal session and reports their presses and releases to `sink`.
fn register(bindings: Vec<HotkeyBinding>, sink: EventSink) -> HotkeyBackend {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let spawned = std::thread::Builder::new()
        .name("tomakast-hotkeys".into())
        .spawn(move || {
            async_io::block_on(async move {
                if let Err(reason) = run(bindings, &sink, &ready_tx).await {
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
    sink: &EventSink,
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
    let activated = portal
        .receive_activated()
        .await
        .map_err(|e| err("receive_activated failed", e))?
        .map(|ev| {
            (
                Signal::Activated,
                ev.shortcut_id().to_string(),
                ev.timestamp(),
            )
        });
    let deactivated = portal
        .receive_deactivated()
        .await
        .map_err(|e| err("receive_deactivated failed", e))?
        .map(|ev| {
            (
                Signal::Deactivated,
                ev.shortcut_id().to_string(),
                ev.timestamp(),
            )
        });
    // One loop for both. The two streams are polled in turn, so a press and its release that
    // arrive together may come out swapped: `Releases` drops such a late press by its timestamp.
    let mut events = futures_util::stream::select(activated, deactivated);
    let mut order = Releases::default();
    let _ = ready.send(Ok(()));
    while let Some((signal, id, at)) = events.next().await {
        if order.keep(signal, &id, at) {
            sink(event_of(signal, id));
        } else {
            tracing::debug!(id, "a press older than its release; dropped");
        }
    }
    // E.g. the portal service restarted; nothing re-binds the shortcuts.
    tracing::warn!("global shortcut stream ended; hotkeys are no longer active");
    drop(session);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tomakast_core::Hotkey;

    fn binding(id: &str, s: &str) -> HotkeyBinding {
        HotkeyBinding {
            id: id.into(),
            description: id.into(),
            hotkey: Hotkey::parse(s).unwrap(),
        }
    }

    #[test]
    fn deactivated_is_a_release() {
        assert_eq!(
            event_of(Signal::Activated, "leader".into()),
            HotkeyEvent::Activated("leader".into())
        );
        assert_eq!(
            event_of(Signal::Deactivated, "leader".into()),
            HotkeyEvent::Released("leader".into())
        );
    }

    #[test]
    fn a_press_older_than_a_delivered_release_is_dropped() {
        let ms = Duration::from_millis;
        let mut order = Releases::default();
        assert!(order.keep(Signal::Deactivated, "leader", ms(20)));
        assert!(!order.keep(Signal::Activated, "leader", ms(10)));
        // Other shortcuts are not affected.
        assert!(order.keep(Signal::Activated, "apps/kitty", ms(10)));
    }

    #[test]
    fn a_press_stamped_as_its_release_is_kept() {
        let ms = Duration::from_millis;
        let mut order = Releases::default();
        assert!(order.keep(Signal::Deactivated, "leader", ms(20)));
        assert!(order.keep(Signal::Activated, "leader", ms(20)));
    }

    #[test]
    fn a_portal_without_timestamps_loses_no_press() {
        let zero = Duration::ZERO;
        let mut order = Releases::default();
        for signal in [
            Signal::Activated,
            Signal::Deactivated,
            Signal::Activated,
            Signal::Deactivated,
        ] {
            assert!(order.keep(signal, "leader", zero));
        }
        // An unknown time is kept even after a stamped release.
        assert!(order.keep(Signal::Deactivated, "leader", Duration::from_millis(20)));
        assert!(order.keep(Signal::Activated, "leader", zero));
    }

    #[test]
    fn presses_and_releases_in_order_are_kept() {
        let ms = Duration::from_millis;
        let mut order = Releases::default();
        assert!(order.keep(Signal::Activated, "leader", ms(10)));
        assert!(order.keep(Signal::Deactivated, "leader", ms(20)));
        assert!(order.keep(Signal::Activated, "leader", ms(30)));
        // A release older than a delivered press stays.
        assert!(order.keep(Signal::Deactivated, "leader", ms(25)));
        assert!(order.keep(Signal::Deactivated, "leader", ms(40)));
    }

    #[test]
    fn a_modifier_alone_is_refused_after_the_bind_too() {
        let ctrl = HotkeyBinding {
            id: "leader".into(),
            description: "Leader".into(),
            hotkey: Hotkey::parse_leader("Ctrl").unwrap(),
        };
        assert_eq!(
            later_states(std::slice::from_ref(&ctrl), std::slice::from_ref(&ctrl)),
            [(
                "leader".to_string(),
                BindState::Refused {
                    reason: "a modifier alone is not available on this system".into()
                }
            )]
        );
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
