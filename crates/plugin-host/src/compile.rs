//! Compiling a component in a short-lived child process, so that Cranelift's memory never
//! stays in the daemon.
//!
//! The daemon writes the wasm bytes it read (and hashed) to a temp file in the cache dir and
//! runs `<exe> compile-plugin --input <tmp> --sha256 <hex> --output <entry>`. The child
//! re-hashes its input and refuses a mismatch, precompiles with [`engine_config`], writes
//! `<entry>.<child pid>.tmp` and renames it to `<entry>`. The temp name is unique per child,
//! so compiles of the same entry can run at once. The child is only ever given paths the
//! daemon wrote.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail, ensure};
use sha2::{Digest, Sha256};
use wasmtime::Engine;

use crate::cache::{INPUT_PREFIX, hex, parse_hex32};
use crate::engine::engine_config;
use crate::package::MAX_WASM_BYTES;

/// The subcommand that runs the compile child.
pub const SUBCOMMAND: &str = "compile-plugin";

/// How long the compile child may run before it is killed.
pub const COMPILE_TIMEOUT: Duration = Duration::from_secs(60);

/// The arguments [`compile_in_child`] passes to the executable.
pub fn child_args(input: &Path, wasm_hash: &[u8; 32], output: &Path) -> Vec<OsString> {
    vec![
        SUBCOMMAND.into(),
        "--input".into(),
        input.into(),
        "--sha256".into(),
        hex(wasm_hash).into(),
        "--output".into(),
        output.into(),
    ]
}

/// `<output>.<pid>.tmp`, where the child with process id `pid` writes before the rename.
pub fn temp_output(output: &Path, pid: u32) -> PathBuf {
    let mut name = output.as_os_str().to_owned();
    name.push(format!(".{pid}.tmp"));
    PathBuf::from(name)
}

/// Compiles `wasm` into the cache entry `out` by running `exe compile-plugin`.
pub fn compile_in_child(
    exe: &Path,
    wasm: &[u8],
    wasm_hash: [u8; 32],
    out: &Path,
) -> anyhow::Result<()> {
    compile_with_timeout(exe, wasm, wasm_hash, out, COMPILE_TIMEOUT)
}

fn compile_with_timeout(
    exe: &Path,
    wasm: &[u8],
    wasm_hash: [u8; 32],
    out: &Path,
    timeout: Duration,
) -> anyhow::Result<()> {
    let dir = out.parent().context("cache entry has no parent folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut input = tempfile::Builder::new()
        .prefix(INPUT_PREFIX)
        .suffix(".wasm")
        .tempfile_in(dir)
        .context("creating the compile input")?;
    input.write_all(wasm).context("writing the compile input")?;
    // The child opens the file by path; keep only the path (deleted on drop).
    let input = input.into_temp_path();

    let mut child = Command::new(exe)
        .args(child_args(&input, &wasm_hash, out))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting the compile child {}", exe.display()))?;
    let tmp = temp_output(out, child.id());
    let stderr = child.stderr.take().expect("stderr is piped");
    let reader = std::thread::spawn(move || read_capped_lossy(stderr, 4096));

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().context("waiting for the compile child")? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&tmp);
            bail!("compiling the plugin took longer than {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let message = reader.join().unwrap_or_default();
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("compile child failed ({status}): {}", message.trim());
    }
    let meta = std::fs::symlink_metadata(out).context("compile child wrote no cache entry")?;
    ensure!(meta.is_file(), "cache entry is not a regular file");
    Ok(())
}

/// Reads all of `r`, keeping the first `max` bytes as text.
fn read_capped_lossy(mut r: impl Read, max: usize) -> String {
    let mut kept = Vec::new();
    let _ = r.by_ref().take(max as u64).read_to_end(&mut kept);
    let _ = std::io::copy(&mut r, &mut std::io::sink());
    String::from_utf8_lossy(&kept).into_owned()
}

/// The child's side: `compile-plugin --input <path> --sha256 <hex> --output <path>`. Refuses an
/// input whose sha256 is not `sha256_hex`; writes `<output>.<pid>.tmp`, then renames it. The
/// temp file is created fresh (`create_new`, after removing whatever has its name), so a
/// planted symlink there is never written through.
pub fn child_main(input: &Path, sha256_hex: &str, output: &Path) -> anyhow::Result<()> {
    let expected = parse_hex32(sha256_hex).context("--sha256 is not 64 hex digits")?;
    let mut wasm = Vec::new();
    File::open(input)
        .with_context(|| format!("opening {}", input.display()))?
        .take(MAX_WASM_BYTES + 1)
        .read_to_end(&mut wasm)
        .with_context(|| format!("reading {}", input.display()))?;
    ensure!(
        wasm.len() as u64 <= MAX_WASM_BYTES,
        "input is larger than {MAX_WASM_BYTES} bytes"
    );
    let actual: [u8; 32] = Sha256::digest(&wasm).into();
    ensure!(
        actual == expected,
        "input sha256 {} does not match --sha256 {}",
        hex(&actual),
        hex(&expected)
    );

    let engine = Engine::new(&engine_config())?;
    let compiled = engine.precompile_component(&wasm)?;

    let tmp = temp_output(output, std::process::id());
    let written = (|| {
        match std::fs::remove_file(&tmp) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(&compiled)?;
        file.sync_all()?;
        std::fs::rename(&tmp, output)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("writing {}", output.display()));
    }
    Ok(())
}

/// Parses exactly the arguments [`child_args`] builds (after the program name) and runs
/// [`child_main`]. For the test helper binary; the app parses them with clap.
pub fn child_main_from_args(args: &[OsString]) -> anyhow::Result<()> {
    let [sub, i, input, s, sha, o, output] = args else {
        bail!("usage: {SUBCOMMAND} --input <path> --sha256 <hex> --output <path>");
    };
    if sub != SUBCOMMAND || i != "--input" || s != "--sha256" || o != "--output" {
        bail!("usage: {SUBCOMMAND} --input <path> --sha256 <hex> --output <path>");
    }
    let sha = sha.to_str().context("--sha256 is not UTF-8")?;
    child_main(Path::new(input), sha, Path::new(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_args_roundtrip_through_the_helper_parser() {
        let args = child_args(
            Path::new("/c/in.wasm"),
            &[0xab; 32],
            Path::new("/c/out.cwasm"),
        );
        assert_eq!(args[0], SUBCOMMAND);
        assert_eq!(args[4], "ab".repeat(32).as_str());
        // Wrong order or a missing flag is refused before anything is read.
        let mut swapped = args.clone();
        swapped.swap(1, 3);
        assert!(child_main_from_args(&swapped).is_err());
        assert!(child_main_from_args(&args[..6]).is_err());
    }

    #[test]
    fn temp_output_appends_pid_and_tmp() {
        assert_eq!(
            temp_output(Path::new("/c/x.cwasm"), 42),
            PathBuf::from("/c/x.cwasm.42.tmp")
        );
    }

    /// The smallest valid component, so the child can run in this process.
    const EMPTY_COMPONENT: &[u8] = b"\0asm\x0d\x00\x01\x00";

    #[cfg(unix)]
    #[test]
    fn child_does_not_write_through_a_planted_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.wasm");
        std::fs::write(&input, EMPTY_COMPONENT).unwrap();
        let out = dir.path().join("entry.cwasm");
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"victim").unwrap();
        std::os::unix::fs::symlink(&victim, temp_output(&out, std::process::id())).unwrap();

        let sha = hex(&Sha256::digest(EMPTY_COMPONENT));
        child_main(&input, &sha, &out).unwrap();
        let victim_now = std::fs::read(&victim).unwrap();
        assert!(
            victim_now == b"victim",
            "the symlink target was overwritten"
        );
        assert!(std::fs::symlink_metadata(&out).unwrap().is_file());
        assert!(!temp_output(&out, std::process::id()).exists());
    }

    #[cfg(unix)]
    #[test]
    fn slow_child_is_killed_at_the_timeout() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("slow-child");
        std::fs::write(&exe, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = dir.path().join("cache/entry.cwasm");

        let start = Instant::now();
        let err = compile_with_timeout(&exe, b"x", [0; 32], &out, Duration::from_millis(200))
            .unwrap_err();
        assert!(err.to_string().contains("longer than"), "{err:#}");
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(!out.exists());
        // The temp input is gone too.
        let left: Vec<_> = std::fs::read_dir(dir.path().join("cache"))
            .unwrap()
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }
}
