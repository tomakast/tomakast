//! The guest SDK (`crates/plugin-sdk`) is not a workspace member, so `cargo test --workspace`
//! does not run its tests. These checks keep what it copies from the host side in step,
//! reading its source with `include_str!`:
//! - its `API_VERSION` is the WIT package version, and one this host runs;
//! - the format characters it refuses in ids are exactly the ones the launcher refuses;
//! - its check of the plugin id in an item id is the manifest's check of `id`.

use tomakast_core::is_hidden_char;
use tomakast_plugin_host::SUPPORTED_API_VERSIONS;

const SDK_LIB: &str = include_str!("../../plugin-sdk/src/lib.rs");
const SDK_BUILDERS: &str = include_str!("../../plugin-sdk/src/builders.rs");
const WIT: &str = include_str!("../../plugin-api/wit/since_v0.1.0/plugin.wit");
const MANIFEST: &str = include_str!("../src/manifest.rs");

/// The text between `start` (which must occur once) and the next `end`.
fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    assert_eq!(text.matches(start).count(), 1, "{start:?} occurs once");
    let rest = &text[text.find(start).unwrap() + start.len()..];
    &rest[..rest.find(end).expect("the end marker")]
}

#[test]
fn sdk_api_version_is_the_wit_version_and_supported() {
    let sdk = between(SDK_LIB, "pub const API_VERSION: &str = \"", "\";");
    let wit = between(WIT, "package tomakast:plugin@", ";");
    assert_eq!(
        sdk, wit,
        "the SDK's API_VERSION and the WIT package version"
    );
    assert!(
        SUPPORTED_API_VERSIONS.contains(&sdk),
        "this host does not run the SDK's API {sdk}"
    );
}

#[test]
fn sdk_refuses_the_same_id_characters_as_the_launcher() {
    let table = between(
        SDK_BUILDERS,
        "const FORMAT_CHARS: &[(char, char)] = &[",
        "];",
    );
    let chars: Vec<char> = table
        .split("'\\u{")
        .skip(1)
        .map(|hex| {
            let hex = &hex[..hex.find('}').unwrap()];
            char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap()
        })
        .collect();
    assert!(
        !chars.is_empty() && chars.len().is_multiple_of(2),
        "parsed {chars:?}"
    );
    let ranges: Vec<(char, char)> = chars.chunks(2).map(|p| (p[0], p[1])).collect();
    // The SDK refuses a control character or one of its format characters.
    let sdk_refuses =
        |c: char| c.is_control() || ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c));
    let differ: Vec<char> = (0..=char::MAX as u32)
        .filter_map(char::from_u32)
        .filter(|&c| sdk_refuses(c) != is_hidden_char(c))
        .take(10)
        .collect();
    assert!(
        differ.is_empty(),
        "the SDK and the launcher disagree on {differ:?}"
    );
}

/// The body of a one-argument `fn <name>(<arg>: &str) -> bool` in `source`, without white
/// space, its argument renamed to `s`.
fn predicate_body(source: &str, name: &str, arg: &str) -> String {
    let body = between(source, &format!("fn {name}({arg}: &str) -> bool {{"), "\n}");
    let body: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    // `&id.len()` and `&&id.bytes()`.
    body.replace(&format!("&{arg}."), "&s.")
}

#[test]
fn sdk_checks_plugin_ids_as_the_manifest_does() {
    // The SDK refuses an item id whose plugin part `plugin.toml` would refuse as an `id`, and
    // nothing else: a drift would reject valid ids, or let through ones the host drops.
    let sdk = predicate_body(SDK_BUILDERS, "is_plugin_id", "id");
    let host = predicate_body(MANIFEST, "valid_ident", "s");
    assert!(sdk.contains("s.len()"), "parsed {sdk}");
    assert_eq!(
        sdk, host,
        "the SDK's is_plugin_id (crates/plugin-sdk/src/builders.rs) and the manifest's \
         valid_ident (crates/plugin-host/src/manifest.rs)"
    );
}
