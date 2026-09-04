//! The parse entry points, and the exits they take in place of a return.

use clap::FromArgMatches;

use super::globals::global_refusal;
use super::tree::Cli;

/// Print `msg` to stderr and exit with the usage status.
pub(crate) fn die_usage(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(crate::cli::exit_codes::USAGE)
}

/// Parse `argv` into the command tree.
///
/// This function does not always return:
///
/// - A usage error exits [`crate::cli::exit_codes::USAGE`].
/// - `--help` and `--version` print their text and exit 0.
pub(crate) fn parse_or_exit(argv: &[String]) -> Cli {
    let cli = match try_parse(argv) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };
    if let Some(refusal) = global_refusal(&cli) {
        die_usage(&refusal);
    }
    cli
}

/// Parse `argv` against the tree that carries each command's examples, so
/// `--help` and `docs/cli.md` show the same invocations.
pub(crate) fn try_parse(argv: &[String]) -> Result<Cli, clap::Error> {
    let matches = crate::cli::reference::command_tree().try_get_matches_from(argv)?;
    // `Cli::from_arg_matches` raises an unformatted error: no usage
    // line, no "try --help". `Parser::try_parse_from` formats every such
    // error through the command, so this path formats it here instead.
    Cli::from_arg_matches(&matches)
        .map_err(|e| e.format(&mut crate::cli::reference::command_tree()))
}
