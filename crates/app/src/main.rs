use std::path::Path;

use clap::Parser;
use tomakast_app::cli::{Cli, Command};
use tomakast_app::daemon;
use tomakast_app::ipc::{self, Request};
use tomakast_core::Paths;

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        None => match ipc::send(&Request::Toggle) {
            Ok(()) => 0,
            Err(_) => run_daemon(),
        },
        Some(Command::Daemon) => run_daemon(),
        Some(Command::Toggle) => client(Request::Toggle),
        Some(Command::Run { id }) => client(Request::Run(id)),
        Some(Command::Settings { page }) => client(Request::Settings(page)),
        Some(Command::ReleaseHotkeys) => match tomakast_platform::release_hotkeys() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("tomakast release-hotkeys: {e}");
                1
            }
        },
        Some(Command::CompilePlugin {
            input,
            sha256,
            output,
        }) => compile_plugin(&input, &sha256, &output),
    };
    std::process::exit(code);
}

/// The compile child does nothing but compile: no IPC, no paths, no logging.
fn compile_plugin(input: &Path, sha256: &str, output: &Path) -> i32 {
    match tomakast_plugin_host::compile::child_main(input, sha256, output) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("tomakast compile-plugin: {e:#}");
            1
        }
    }
}

fn client(req: Request) -> i32 {
    match ipc::send(&req) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("tomakast: {e:#}");
            1
        }
    }
}

fn run_daemon() -> i32 {
    match Paths::detect().and_then(daemon::run) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("tomakast: {e:#}");
            1
        }
    }
}
