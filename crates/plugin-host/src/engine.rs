//! The daemon's wasmtime engine and component loading through the cache.

use std::path::Path;

use anyhow::{Context, bail, ensure};
use sha2::{Digest, Sha256};
use wasmtime::component::Component;
use wasmtime::{Config, Engine};

use crate::cache::{Cache, Entry, read_entry};
use crate::compile::compile_in_child;
use crate::epoch::Epoch;
use crate::manifest::SUPPORTED_API_VERSIONS;

/// The engine config shared by the daemon and the compile child, so that the child's output
/// deserializes in the daemon: epoch interruption on, wasm GC off.
pub fn engine_config() -> Config {
    let mut config = Config::new();
    config.epoch_interruption(true);
    config.wasm_gc(false);
    config
}

pub struct HostEngine {
    engine: Engine,
}

impl HostEngine {
    /// The engine, registered with [`Epoch::global`].
    pub fn new() -> anyhow::Result<HostEngine> {
        let engine = Engine::new(&engine_config())?;
        Epoch::global().register(&engine);
        Ok(HostEngine { engine })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The component for `wasm`, deserialized from its entry in `cache`. On a miss the entry
    /// is compiled first by `exe compile-plugin`; a corrupt entry is recompiled once. Nothing
    /// outside `cache.dir()` is ever deserialized: the entry is read into memory without
    /// following a symlink, and an entry that is not a regular file is replaced.
    ///
    /// Takes no lock: concurrent compiles of one entry each write their own temp file and
    /// rename it into place.
    pub fn load(
        &self,
        wasm_hash: [u8; 32],
        wasm: &[u8],
        cache: &Cache,
        exe: &Path,
    ) -> anyhow::Result<Component> {
        // The entry is keyed by the hash: a hit must not serve a component for other bytes.
        ensure!(
            <[u8; 32]>::from(Sha256::digest(wasm)) == wasm_hash,
            "plugin.wasm does not match its hash"
        );
        let entry = cache.path_for(&wasm_hash, &self.engine);
        if let Some(component) = self.cached(&entry)? {
            return Ok(component);
        }
        compile_in_child(exe, wasm, wasm_hash, &entry)?;
        match read_entry(&entry) {
            Entry::Bytes(bytes) => self.deserialize(&bytes, &entry),
            Entry::Missing => bail!("the compile child left no {}", entry.display()),
            Entry::Unusable(problem) => bail!("{}: {problem}", entry.display()),
        }
    }

    /// The component in `entry`, or `None` when there is no usable entry. An unusable entry
    /// is removed so the compile child can write a fresh one.
    fn cached(&self, entry: &Path) -> anyhow::Result<Option<Component>> {
        let problem = match read_entry(entry) {
            Entry::Missing => return Ok(None),
            Entry::Bytes(bytes) => match self.deserialize(&bytes, entry) {
                Ok(component) => return Ok(Some(component)),
                Err(e) => format!("{e:#}"),
            },
            Entry::Unusable(problem) => problem,
        };
        tracing::debug!(
            target: "plugin",
            entry = %entry.display(),
            "discarding cache entry, recompiling: {problem}"
        );
        // A directory here was not made by the cache; refuse rather than remove a tree.
        let is_dir = std::fs::symlink_metadata(entry).is_ok_and(|m| m.is_dir());
        ensure!(!is_dir, "{} is a directory", entry.display());
        match std::fs::remove_file(entry) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| format!("removing stale cache entry {}", entry.display()))
            }
            _ => Ok(None),
        }
    }

    fn deserialize(&self, bytes: &[u8], entry: &Path) -> anyhow::Result<Component> {
        // SAFETY: `bytes` were read, without following symlinks, from a regular file in the
        // daemon's own cache dir, which only the compile child writes, from bytes the daemon
        // read and hashed. wasmtime also checks that a compatible engine produced them.
        let component = unsafe { Component::deserialize(&self.engine, bytes) }
            .map_err(anyhow::Error::from)
            .with_context(|| format!("loading {}", entry.display()))?;
        Ok(component)
    }
}

/// The package prefix of the contract's interfaces, as they appear in import names
/// (`tomakast:plugin/storage@0.1.0`).
const API_PACKAGE: &str = "tomakast:plugin/";

/// The API version `component` was built for: the `@x.y.z` of its `tomakast:plugin/*`
/// imports. `Ok(None)` when no import carries one (the component then gets the manifest's
/// word); an error when the imports name different versions.
///
/// Every component built from the WIT imports `tomakast:plugin/types@<version>`, because its
/// exports use those types; interfaces it never calls are left out of its imports.
pub fn api_version(component: &Component, engine: &Engine) -> Result<Option<String>, String> {
    let ty = component.component_type();
    versions_of(ty.imports(engine).map(|(name, _)| name))
}

/// [`api_version`] over the import names.
fn versions_of<'a>(names: impl Iterator<Item = &'a str>) -> Result<Option<String>, String> {
    let mut found: Option<&str> = None;
    for name in names {
        let Some((_, version)) = name
            .strip_prefix(API_PACKAGE)
            .and_then(|rest| rest.split_once('@'))
        else {
            continue;
        };
        match found {
            Some(seen) if seen != version => {
                return Err(format!("built for API {seen} and {version} at once"));
            }
            _ => found = Some(version),
        }
    }
    Ok(found.map(str::to_owned))
}

/// Whether a component built for API `built_for` ([`api_version`]) may run with a
/// `plugin.toml` that says `api = manifest_api`: the version must be one this host runs and
/// the manifest's. A component that records no version runs on the manifest's word.
pub fn check_api_version(built_for: Option<&str>, manifest_api: &str) -> Result<(), String> {
    match built_for {
        None => Ok(()),
        Some(version) if !SUPPORTED_API_VERSIONS.contains(&version) => Err(format!(
            "built for API {version}, which this launcher does not run (it runs {})",
            SUPPORTED_API_VERSIONS.join(", ")
        )),
        Some(version) if version != manifest_api => Err(format!(
            "built for API {version}, but its plugin.toml says api = \"{manifest_api}\""
        )),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comes_from_the_tomakast_imports_only() {
        let names = [
            "wasi:io/poll@0.2.9",
            "tomakast:plugin/types@0.1.0",
            "tomakast:plugin/storage@0.1.0",
            "wasi:cli/stdout@0.2.9",
        ];
        assert_eq!(versions_of(names.into_iter()), Ok(Some("0.1.0".into())));
        // No marker: only other packages, or an unversioned name.
        let none = [
            "wasi:io/poll@0.2.9",
            "tomakast:plugin/types",
            "tomakast:other/x@9.9.9",
        ];
        assert_eq!(versions_of(none.into_iter()), Ok(None));
        assert_eq!(versions_of(std::iter::empty()), Ok(None));
    }

    #[test]
    fn mixed_versions_are_an_error() {
        let names = ["tomakast:plugin/types@0.1.0", "tomakast:plugin/log@0.2.0"];
        let err = versions_of(names.into_iter()).unwrap_err();
        assert!(err.contains("0.1.0") && err.contains("0.2.0"), "{err}");
    }

    #[test]
    fn check_needs_a_supported_version_that_the_manifest_states() {
        assert_eq!(check_api_version(Some("0.1.0"), "0.1.0"), Ok(()));
        assert_eq!(check_api_version(None, "0.1.0"), Ok(()), "no marker");
        let unsupported = check_api_version(Some("0.2.0"), "0.2.0").unwrap_err();
        assert!(
            unsupported.starts_with("built for API 0.2.0,"),
            "{unsupported}"
        );
        assert!(unsupported.contains("it runs 0.1.0"), "{unsupported}");
        let disagree = check_api_version(Some("0.1.0"), "0.2.0").unwrap_err();
        assert_eq!(
            disagree,
            r#"built for API 0.1.0, but its plugin.toml says api = "0.2.0""#
        );
    }
}
