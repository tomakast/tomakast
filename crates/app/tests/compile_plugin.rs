//! The hidden `tomakast compile-plugin` subcommand, run as the daemon runs it.

use std::process::Command;

use sha2::{Digest, Sha256};
use tomakast_plugin_host::compile::child_args;

/// The smallest valid component: the preamble with component-model version 0x0d, layer 1.
const EMPTY_COMPONENT: &[u8] = b"\0asm\x0d\x00\x01\x00";

#[test]
fn compile_plugin_checks_the_hash_and_writes_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.wasm");
    std::fs::write(&input, EMPTY_COMPONENT).unwrap();
    let out = dir.path().join("entry.cwasm");
    let hash: [u8; 32] = Sha256::digest(EMPTY_COMPONENT).into();
    let mut wrong = hash;
    wrong[31] ^= 0xff;

    let refused = Command::new(env!("CARGO_BIN_EXE_tomakast"))
        .args(child_args(&input, &wrong, &out))
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("sha256"));
    assert!(!out.exists());
    assert!(no_temp_files(dir.path()));

    let ok = Command::new(env!("CARGO_BIN_EXE_tomakast"))
        .args(child_args(&input, &hash, &out))
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(std::fs::metadata(&out).unwrap().len() > 0);
    assert!(no_temp_files(dir.path()));
}

fn no_temp_files(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .unwrap()
        .all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp"))
}
