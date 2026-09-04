//! The help layout, and the walk that attaches each command's examples.

use clap::CommandFactory;

use super::examples;

/// Help layout for a command that leads with examples.
const HELP_TEMPLATE: &str = "\
{about-with-newline}
{usage-heading} {usage}

{before-help}{all-args}{after-help}";

/// The command tree, with each documented command's examples attached.
pub(crate) fn command_tree() -> clap::Command {
    decorate(crate::cli::parse::Cli::command(), "")
}

/// Attach `path`'s example block, then recurse into its subcommands.
///
/// `path` is the space-separated path below `cellgov`, and the key
/// [`examples::lines_for`] answers to. The walk covers every subcommand
/// the tree declares; clap adds its own `help` subcommand later, during
/// the build.
fn decorate(cmd: clap::Command, path: &str) -> clap::Command {
    let mut cmd = match examples::lines_for(path) {
        Some(lines) => cmd
            .before_help(examples::block(lines))
            .help_template(HELP_TEMPLATE),
        None => cmd,
    };
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|sub| sub.get_name().to_string())
        .collect();
    for name in names {
        let child = match path {
            "" => name.clone(),
            _ => format!("{path} {name}"),
        };
        cmd = cmd.mut_subcommand(&name, |sub| decorate(sub, &child));
    }
    cmd
}
