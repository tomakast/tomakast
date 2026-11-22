//! No global hotkeys in this run: every change is accepted at once (so the file is written)
//! and nothing is bound.

use super::{BindState, EventSink, HotkeyBinding, HotkeyEvent, Registrar, SetKind};

pub(super) struct NullBackend {
    sink: EventSink,
}

impl NullBackend {
    /// Reports `Unavailable { reason }` at once.
    pub(super) fn new(reason: String, sink: EventSink) -> NullBackend {
        sink(HotkeyEvent::Unavailable { reason });
        NullBackend { sink }
    }
}

impl Registrar for NullBackend {
    fn set(&mut self, set: SetKind, generation: u64, bindings: Vec<HotkeyBinding>) {
        let states = bindings
            .into_iter()
            .map(|b| (b.id, BindState::Active { trigger: None }))
            .collect();
        (self.sink)(HotkeyEvent::Applied {
            set,
            generation,
            outcome: Ok(states),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tomakast_core::Hotkey;

    #[test]
    fn null_backend_accepts_everything() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let mut backend = NullBackend::new(
            "no session".into(),
            Arc::new(move |e| sink.lock().unwrap().push(e)),
        );
        backend.set(
            SetKind::Items,
            7,
            vec![HotkeyBinding {
                id: "a".into(),
                description: "A".into(),
                hotkey: Hotkey::parse("ctrl+alt+a").unwrap(),
            }],
        );
        assert_eq!(
            *events.lock().unwrap(),
            [
                HotkeyEvent::Unavailable {
                    reason: "no session".into()
                },
                HotkeyEvent::Applied {
                    set: SetKind::Items,
                    generation: 7,
                    outcome: Ok(vec![("a".into(), BindState::Active { trigger: None })]),
                },
            ]
        );
    }
}
