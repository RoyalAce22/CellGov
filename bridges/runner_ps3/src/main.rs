//! `runner_ps3`: the command-line entry to the PS3 runner.
//!
//! The verbs arrive with the console transport and the capture loop;
//! until then the binary answers every invocation with its usage and
//! the [`ExitCode::Usage`] status, so a script that wires it in early
//! sees the contract it will keep.

#![allow(
    clippy::print_stderr,
    reason = "CLI binary: stderr is the user-facing diagnostic channel"
)]

use std::process::ExitCode as ProcessExit;

use runner_ps3::{ExitCode, RunnerPs3Error};

const USAGE: &str = "usage: runner_ps3 <verb> [--host <console>] ...\n\
                     verbs: status, deploy, run, fetch, cleanup, capture, convert, unlock";

fn main() -> ProcessExit {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Ok(()) => ProcessExit::from(ExitCode::Ok.code() as u8),
        Err(error) => {
            eprintln!("runner_ps3: {error}");
            eprintln!("{USAGE}");
            ProcessExit::from(error.exit_code().code() as u8)
        }
    }
}

/// The verb parser. This build implements no verb, so every invocation
/// returns a usage error that names the verb it asked for.
fn parse(args: &[String]) -> Result<(), RunnerPs3Error> {
    match args.first().map(String::as_str) {
        None => Err(RunnerPs3Error::Usage("no verb given".to_string())),
        Some(verb) => Err(RunnerPs3Error::Usage(format!(
            "verb {verb:?} is not available in this build"
        ))),
    }
}
