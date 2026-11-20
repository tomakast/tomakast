//! Command line of the `tomakast` binary.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tomakast_core::ItemId;

#[derive(Parser, Debug, PartialEq, Eq)]
#[command(name = "tomakast", version, about = "A fast keyboard-driven launcher")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Command {
    /// Run the daemon (exits with code 1 if one is already running)
    Daemon,
    /// Show or hide the launcher window of the running daemon
    Toggle,
    /// Run an item by its identifier, e.g. `apps/firefox.desktop`
    Run {
        #[arg(value_parser = parse_item_id)]
        id: ItemId,
    },
    /// Internal: compile a plugin component into the daemon's cache, then exit. The daemon runs
    /// this in a short-lived child process so that the compiler's memory never stays in it.
    #[command(hide = true)]
    CompilePlugin {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        sha256: String,
        #[arg(long)]
        output: PathBuf,
    },
}

fn parse_item_id(s: &str) -> Result<ItemId, String> {
    ItemId::parse(s).ok_or_else(|| format!("{s:?} is not of the form <provider>/<item>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_subcommands() {
        assert_eq!(Cli::try_parse_from(["tomakast"]).unwrap().command, None);
        assert_eq!(
            Cli::try_parse_from(["tomakast", "toggle"]).unwrap().command,
            Some(Command::Toggle)
        );
        assert_eq!(
            Cli::try_parse_from(["tomakast", "run", "apps/x.desktop"])
                .unwrap()
                .command,
            Some(Command::Run {
                id: ItemId::parse("apps/x.desktop").unwrap()
            })
        );
        assert!(Cli::try_parse_from(["tomakast", "run", "nosl"]).is_err());
    }

    #[test]
    fn parses_the_arguments_the_daemon_gives_the_compile_child() {
        use std::path::Path;
        use tomakast_plugin_host::compile::child_args;
        let args = child_args(
            Path::new("/c/.input-1.wasm"),
            &[0x5a; 32],
            Path::new("/c/e.cwasm"),
        );
        let cli = Cli::try_parse_from(std::iter::once("tomakast".into()).chain(args)).unwrap();
        assert_eq!(
            cli.command,
            Some(Command::CompilePlugin {
                input: "/c/.input-1.wasm".into(),
                sha256: "5a".repeat(32),
                output: "/c/e.cwasm".into(),
            })
        );
    }

    #[test]
    fn compile_plugin_is_hidden_from_help() {
        use clap::CommandFactory;
        let help = Cli::command().render_long_help().to_string();
        assert!(!help.contains("compile-plugin"), "{help}");
        assert!(help.contains("daemon"));
    }
}
