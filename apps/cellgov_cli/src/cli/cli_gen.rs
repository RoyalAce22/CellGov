//! `cellgov dev cli-gen` and `cellgov dev completions` -- the command
//! tree rendered as a reference document and as shell scripts.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::exit::die;
use super::exit_codes;
use super::parse::{CliGenArgs, CompletionShell, CompletionsArgs};
use super::reference::{command_tree, render_doc};

const DEFAULT_OUTPUT: &str = "docs/cli.md";

pub(crate) fn run(args: &CliGenArgs) {
    let output = resolve_output(args.output.as_deref());
    let body = render_doc(&command_tree());
    std::fs::write(&output, body)
        .unwrap_or_else(|e| die(&format!("cli-gen: write {}: {e}", output.display())));
    println!("cli-gen: wrote {}", output.display());
}

/// The document `--output` names, or the default.
fn resolve_output(flag: Option<&Path>) -> PathBuf {
    match flag {
        Some(p) if p.as_os_str().is_empty() => {
            die("cli-gen: --output is empty; name the document to write")
        }
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(DEFAULT_OUTPUT),
    }
}

pub(crate) fn completions(args: &CompletionsArgs) {
    // `clap_complete::generate` panics on a write failure, and a shell
    // redirects this command's stdout. Build the script in memory, where
    // no write fails, then report the pipe outcome.
    let mut script = Vec::new();
    clap_complete::generate(
        generator(args.shell),
        &mut command_tree(),
        "cellgov",
        &mut script,
    );
    write_stdout_or_exit(&script);
}

/// Write `body` to stdout.
///
/// When the reader already closed the pipe, the process exits
/// [`exit_codes::BROKEN_PIPE`].
fn write_stdout_or_exit(body: &[u8]) {
    let mut out = std::io::stdout().lock();
    let flushed = out.write_all(body).and_then(|()| out.flush());
    if let Err(e) = flushed {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(exit_codes::BROKEN_PIPE);
        }
        die(&format!("completions: stdout write: {e}"));
    }
}

pub(crate) fn generator(shell: CompletionShell) -> clap_complete::Shell {
    match shell {
        CompletionShell::Bash => clap_complete::Shell::Bash,
        CompletionShell::Zsh => clap_complete::Shell::Zsh,
        CompletionShell::Pwsh => clap_complete::Shell::PowerShell,
    }
}
