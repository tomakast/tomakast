use std::path::Path;
use std::time::{Duration, Instant};

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
            Err(_) => open_through_service().unwrap_or_else(run_daemon),
        },
        Some(Command::Daemon) => run_daemon(),
        Some(Command::Toggle) => client(Request::Toggle),
        Some(Command::Quit) => quit(),
        Some(Command::ApplyAutostart) => apply_autostart(),
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

/// How long `quit` and a start through the service wait for the daemon.
const WAIT: Duration = Duration::from_secs(10);

/// Asks the daemon to quit and waits until it is gone, so that a new one can start right
/// after. Nothing running is no error.
fn quit() -> i32 {
    if !ipc::alive() {
        eprintln!("tomakast is not running");
        return 0;
    }
    if let Err(e) = ipc::send(&Request::Quit) {
        eprintln!("tomakast: {e:#}");
        return 1;
    }
    let deadline = Instant::now() + WAIT;
    while ipc::alive() {
        if Instant::now() > deadline {
            eprintln!("tomakast: the daemon did not exit within {WAIT:?}");
            return 1;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    0
}

/// A bare `tomakast` with no daemon running (the app menu's entry): where the launcher runs as
/// a systemd service, starts the service and opens the launcher, so that the daemon stays under
/// systemd. `None` where it does not: the caller runs the daemon itself.
fn open_through_service() -> Option<i32> {
    if daemon::home_override() {
        return None;
    }
    match tomakast_platform::autostart::start_service() {
        Ok(true) => {}
        Ok(false) => return None,
        Err(e) => {
            eprintln!("tomakast: {e:#}; running the daemon here");
            return None;
        }
    }
    // The daemon takes its socket first; the toggle waits there until the window is ready.
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if ipc::send(&Request::Toggle).is_ok() {
            return Some(0);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("tomakast: the service started, but the launcher did not answer");
    Some(1)
}

fn apply_autostart() -> i32 {
    use tomakast_platform::autostart::Backend;
    match Paths::detect().and_then(|paths| daemon::apply_autostart(&paths)) {
        Ok(backend) => {
            println!(
                "{}",
                match backend {
                    Backend::Systemd => "systemd",
                    Backend::Xdg => "xdg",
                    Backend::Unsupported => "none",
                }
            );
            0
        }
        Err(e) => {
            eprintln!("tomakast apply-autostart: {e:#}");
            1
        }
    }
}

fn run_daemon() -> i32 {
    match Paths::detect().and_then(daemon::run) {
        Ok(()) => 0,
        Err(e) if e.is::<daemon::AlreadyRunning>() => {
            eprintln!("tomakast: {e:#}");
            3
        }
        Err(e) => {
            eprintln!("tomakast: {e:#}");
            1
        }
    }
}
