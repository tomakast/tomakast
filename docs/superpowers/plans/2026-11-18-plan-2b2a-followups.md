# Plan 2b-2a: follow-ups

Items deliberately left out of plan 2b-2a by controller rulings. Each needs its own design before it is built.

## R-1. Reviewing denied content again (Ruling AF)

- **Today:** once the user denies a plugin, the gate (`crates/plugin-host/src/approval.rs`, `decide`/`check`) answers `Denied` for that exact content (same `plugin.toml ‖ 0x00 ‖ plugin.wasm` hash), and the user is not asked again.
- **The way out:** the plugin must change (a new hash asks again), or its folder must be removed and re-created (the manager deletes the grant on removal, Ruling AG).
- **Missing:** a way to change one's mind without touching the plugin, for example a "Reset plugin decision" command in the `plugins` module that calls `PluginDb::remove_grant` and re-gates the plugin. It belongs with the plugin manager feature in `docs/superpowers/backlog.md`.
