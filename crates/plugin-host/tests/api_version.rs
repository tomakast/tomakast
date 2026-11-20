//! The API version check: the version a component was built for, read from the `@x.y.z` of
//! its `tomakast:plugin/*` imports, must be one the host runs and the `api` of its
//! `plugin.toml`; otherwise the plugin is refused before it is instantiated.

mod support;

use std::time::Duration;

use support::harness::*;
use tomakast_core::{Effect, ToastKind};
use tomakast_plugin_host::{PluginEvents, SUPPORTED_API_VERSIONS, api_version};
use wasmtime::component::Component;

/// The plugin was refused for `reason`: one error toast that names the plugin and says what
/// it was built for, one unload request, no `init` (its commands are never indexed). Returns
/// the toast's message.
fn assert_refused(h: &mut Harness, reason: &str) -> String {
    let toast = h.toast();
    assert_eq!(toast.kind, ToastKind::Error, "{toast:?}");
    assert!(toast.title.contains(NAME), "{toast:?}");
    let message = toast.message.clone().unwrap_or_default();
    assert!(message.contains(reason), "{toast:?}");
    match h.broken_event() {
        PluginEvents::Broken { id, reason: r, .. } => {
            assert_eq!(id, ID);
            assert!(r.contains(reason), "{r}");
        }
        other => panic!("unexpected {other:?}"),
    }
    h.settle(Duration::from_millis(100));
    assert!(h.take_toast().is_none(), "a second toast");
    assert!(
        h.indexed("Say hello").is_none(),
        "a refused plugin runs nothing"
    );
    assert!(
        !h.unsolicited
            .iter()
            .any(|e| matches!(e, Effect::ReplaceItems(_))),
        "init ran: {:?}",
        h.unsolicited
    );
    message
}

/// The kitchen-sink component with every `tomakast:plugin/<interface>@0.1.0` import renamed to
/// `@0.2.0`: what the same plugin looks like built against a 0.2.0 WIT. The names are patched
/// in place (same length), in the component's imports and in its core module's.
fn built_for_0_2_0() -> Component {
    let wasm = std::fs::read(loaded().2.join("plugin.wasm")).unwrap();
    let mut patched = wasm.clone();
    let mut renamed = 0;
    let prefix = b"tomakast:plugin/";
    let mut at = 0;
    while let Some(start) = find(&patched[at..], prefix).map(|i| i + at) {
        let name_end = start + prefix.len();
        // The interface name (`storage`, `types`, …: a-z and '-', well under 64 bytes) runs
        // up to the `@` of its version; nothing further on is touched.
        let interface = patched[name_end..]
            .iter()
            .take(64)
            .take_while(|b| b.is_ascii_lowercase() || **b == b'-')
            .count();
        let v = name_end + interface + 1;
        if interface > 0 && patched.get(v - 1) == Some(&b'@') && patched[v..].starts_with(b"0.1.0")
        {
            patched[v..v + 5].copy_from_slice(b"0.2.0");
            renamed += 1;
        }
        at = name_end;
    }
    assert!(renamed > 0, "no tomakast import to rename");
    Component::new(loaded().0.engine(), &patched).expect("the patched component compiles")
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn matching_version_loads() {
    let (host, component, _) = loaded();
    // The kitchen-sink is built through the SDK, which records the WIT 0.1.0 it wraps.
    assert_eq!(
        api_version(component, host.engine()).unwrap().as_deref(),
        Some("0.1.0")
    );
    assert!(SUPPORTED_API_VERSIONS.contains(&"0.1.0"));
    let mut h = harness();
    let effects = h.run("kitchen-sink/hello", "run").unwrap();
    assert!(
        matches!(&effects[..], [Effect::Toast(t)] if t.title == "Kitchen Sink: hello"),
        "{effects:?}"
    );
    h.settle(Duration::from_millis(50));
    assert!(h.take_toast().is_none(), "no refusal toast");
}

/// The component is built for 0.1.0 but `plugin.toml` says another version. (`Manifest::parse`
/// refuses an api this host does not run, so the manifest is edited after parsing: what a
/// manifest for a future host that runs both versions would say.)
#[test]
fn manifest_and_component_disagree_is_refused() {
    let (_, component, _) = loaded();
    let mut h = started(component, |package| package.manifest.api = "0.2.0".into());
    let message = assert_refused(&mut h, "built for API 0.1.0");
    assert!(message.contains(r#"api = "0.2.0""#), "{message}");
}

#[test]
fn unsupported_version_is_refused() {
    let component = built_for_0_2_0();
    let host = &loaded().0;
    assert_eq!(
        api_version(&component, host.engine()).unwrap().as_deref(),
        Some("0.2.0")
    );
    let mut h = started(&component, |_| {});
    let message = assert_refused(&mut h, "built for API 0.2.0");
    assert!(
        message.contains("0.1.0"),
        "names what this host runs: {message}"
    );
}
