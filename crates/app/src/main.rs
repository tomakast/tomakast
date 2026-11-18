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
    };
    std::process::exit(code);
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
