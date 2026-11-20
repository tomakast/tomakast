//! The WIT 0.1 contract and the manifest format are frozen.
//!
//! - `plugin.wit` must hash to the sha256 in `wit/since_v0.1.0/FROZEN`, and the folder holds
//!   nothing else but `FROZEN.md`, the freeze policy and the record of re-hashes. Relaxing a
//!   limit is a deliberate re-hash recorded there; tightening one, or any change of shape,
//!   goes into a new version folder, `wit/since_v0.2.0/` (the widgets edit of 2026-11-19 is
//!   the recorded exception).
//! - The keys `Manifest::parse` accepts (top level, `[[commands]]`, `[[preferences]]`,
//!   `[[widgets]]` and `[permissions]`) must equal the keys `docs/plugins/manifest.md`
//!   documents. The parser's
//!   keys are read from the parser itself: every table refuses unknown keys, and its refusal
//!   lists the keys it expects.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tomakast_plugin_host::{Manifest, ManifestError};

fn crate_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn wit_dir() -> PathBuf {
    crate_dir().join("wit/since_v0.1.0")
}

/// What a failure says: where the policy is.
const FROZEN_MESSAGE: &str = "WIT 0.1 is frozen; read crates/plugin-api/wit/since_v0.1.0/FROZEN.md. \
     Relaxing a limit is a deliberate re-hash of FROZEN, recorded in FROZEN.md with its reason; \
     tightening a limit or any change of shape needs a new `since_v0.2.0` folder \
     (the widgets edit of 2026-11-19 is the recorded exception)";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn wit_0_1_is_frozen() {
    let wit = std::fs::read(wit_dir().join("plugin.wit")).expect("plugin.wit");
    let actual = hex(&Sha256::digest(&wit));
    let frozen = std::fs::read_to_string(wit_dir().join("FROZEN")).unwrap_or_default();
    // `<sha256>  plugin.wit` on the first line, as `sha256sum` prints it.
    let pinned = frozen.split_whitespace().next().unwrap_or_default();
    assert_eq!(actual, pinned, "{FROZEN_MESSAGE}");
}

/// The frozen folder holds the contract, its hash and the freeze policy, nothing else: a
/// second `.wit` file would join the package without changing `plugin.wit`'s hash.
#[test]
fn wit_0_1_folder_holds_only_the_frozen_files() {
    let mut names: Vec<String> = std::fs::read_dir(wit_dir())
        .expect("wit/since_v0.1.0")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["FROZEN", "FROZEN.md", "plugin.wit"],
        "{FROZEN_MESSAGE}"
    );
}

/// A minimal valid manifest, with `extra` appended.
fn manifest(extra: &str) -> String {
    format!("id = \"probe\"\nname = \"Probe\"\nversion = \"1\"\napi = \"0.1.0\"\n{extra}")
}

/// The keys the table probed by `text` accepts, from the refusal of its unknown key
/// `zz-unknown`: "unknown field `zz-unknown`, expected one of `a`, `b`" (or "expected `a`").
fn accepted_keys(table: &str, text: &str) -> BTreeSet<String> {
    let err = Manifest::parse(text, "probe").expect_err("an unknown key is refused");
    let ManifestError::Syntax(message) = &err else {
        panic!("{table}: expected a syntax error, got {err:?}");
    };
    let (_, expected) = message
        .split_once("unknown field `zz-unknown`, expected ")
        .unwrap_or_else(|| panic!("{table}: no list of expected keys in {message:?}"));
    let expected = expected.lines().next().unwrap_or_default();
    let keys: BTreeSet<String> = expected
        .split('`')
        .skip(1)
        .step_by(2)
        .map(|key| format!("{table}{key}"))
        .collect();
    assert!(!keys.is_empty(), "{table}: no keys in {message:?}");
    keys
}

/// Every key `Manifest::parse` accepts, as `<table>.<key>` (`<key>` at the top level).
fn parser_keys() -> BTreeSet<String> {
    let mut keys = accepted_keys("", &manifest("zz-unknown = 1\n"));
    keys.extend(accepted_keys(
        "commands.",
        &manifest("[[commands]]\nid = \"c\"\ntitle = \"C\"\nzz-unknown = 1\n"),
    ));
    keys.extend(accepted_keys(
        "preferences.",
        &manifest("[[preferences]]\nkey = \"p\"\ntitle = \"P\"\nkind = \"text\"\nzz-unknown = 1\n"),
    ));
    keys.extend(accepted_keys(
        "widgets.",
        &manifest("[[widgets]]\nid = \"w\"\ntitle = \"W\"\nsize = \"single\"\nzz-unknown = 1\n"),
    ));
    keys.extend(accepted_keys(
        "permissions.",
        &manifest("[permissions]\nzz-unknown = 1\n"),
    ));
    keys
}

/// The keys `docs/plugins/manifest.md` documents: the first cell (`` `key` ``) of each table
/// row under the headings "Top-level keys", "`[[commands]]`", "`[[preferences]]`",
/// "`[[widgets]]`" and "`[permissions]`".
fn documented_keys() -> BTreeSet<String> {
    let path = crate_dir().join("../../docs/plugins/manifest.md");
    let doc = std::fs::read_to_string(&path).expect("docs/plugins/manifest.md");
    let mut table: Option<&str> = None;
    let mut keys = BTreeSet::new();
    for line in doc.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            table = match heading.trim() {
                "Top-level keys" => Some(""),
                "`[[commands]]`" => Some("commands."),
                "`[[preferences]]`" => Some("preferences."),
                "`[[widgets]]`" => Some("widgets."),
                "`[permissions]`" => Some("permissions."),
                _ => None,
            };
            continue;
        }
        let (Some(table), Some(row)) = (table, line.strip_prefix("| `")) else {
            continue;
        };
        let key = row.split('`').next().unwrap_or_default();
        assert!(
            keys.insert(format!("{table}{key}")),
            "`{table}{key}` is documented twice"
        );
    }
    keys
}

#[test]
fn manifest_schema_is_frozen() {
    let parser = parser_keys();
    let documented = documented_keys();
    let undocumented: Vec<&String> = parser.difference(&documented).collect();
    let unknown: Vec<&String> = documented.difference(&parser).collect();
    assert!(
        undocumented.is_empty() && unknown.is_empty(),
        "the manifest format of API 0.1 is frozen: Manifest::parse and docs/plugins/manifest.md \
         must agree. Accepted but not documented: {undocumented:?}; documented but not \
         accepted: {unknown:?}"
    );
}
