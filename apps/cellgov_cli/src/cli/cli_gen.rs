//! `cellgov dev cli-gen` and `cellgov dev completions` -- the command
//! tree rendered as a reference document and as shell scripts.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::exit::{CommandError, CommandExitCode};
use super::exit_codes;
use super::parse::{CliGenArgs, CompletionShell, CompletionsArgs};
use super::reference::{command_tree, render_doc};

const DEFAULT_OUTPUT: &str = "docs/cli.md";

pub(crate) fn run(args: &CliGenArgs) -> Result<(), CommandError> {
    let output = resolve_output(args.output.as_deref())?;
    let body = render_doc(&command_tree());
    std::fs::write(&output, body).map_err(|error| {
        CommandError::failed(format!("cli-gen: write {}: {error}", output.display()))
    })?;
    println!("cli-gen: wrote {}", output.display());
    Ok(())
}

fn resolve_output(flag: Option<&Path>) -> Result<PathBuf, CommandError> {
    match flag {
        Some(p) if p.as_os_str().is_empty() => Err(CommandError::failed(
            "cli-gen: --output is empty; name the document to write",
        )),
        Some(p) => Ok(p.to_path_buf()),
        None => Ok(PathBuf::from(DEFAULT_OUTPUT)),
    }
}

pub(crate) fn completions(args: &CompletionsArgs) -> Result<CommandExitCode, CommandError> {
    // This command generates in memory because `clap_complete::generate`
    // panics if the stdout write fails.
    let mut script = Vec::new();
    clap_complete::generate(
        generator(args.shell),
        &mut command_tree(),
        "cellgov",
        &mut script,
    );
    write_stdout(&script)
}

/// Preserves the closed-pipe status for a downstream reader.
///
/// # Errors
///
/// Returns an error if stdout fails for a reason other than a closed pipe.
fn write_stdout(body: &[u8]) -> Result<CommandExitCode, CommandError> {
    let mut out = std::io::stdout().lock();
    write_body(&mut out, body)
}

fn write_body(mut out: impl Write, body: &[u8]) -> Result<CommandExitCode, CommandError> {
    match out.write_all(body).and_then(|()| out.flush()) {
        Ok(()) => Ok(CommandExitCode::SUCCESS),
        // The shared contract treats a closed stdout pipe as an
        // outcome, not a diagnostic.
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
            Ok(CommandExitCode::new(exit_codes::BROKEN_PIPE))
        }
        Err(error) => Err(CommandError::failed(format!(
            "completions: stdout write: {error}"
        ))),
    }
}

pub(crate) fn generator(shell: CompletionShell) -> clap_complete::Shell {
    match shell {
        CompletionShell::Bash => clap_complete::Shell::Bash,
        CompletionShell::Zsh => clap_complete::Shell::Zsh,
        CompletionShell::Pwsh => clap_complete::Shell::PowerShell,
    }
}

#[cfg(test)]
#[path = "tests/cli_gen_tests.rs"]
mod tests;
