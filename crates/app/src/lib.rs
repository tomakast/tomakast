//! Application glue of tomakast: the daemon, single-instance IPC and the CLI.

pub mod cli;
mod config_watch;
pub mod daemon;
mod hotkeys;
pub mod ipc;
mod leader;
