//! Command line of the `tomakast` binary.

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
}
