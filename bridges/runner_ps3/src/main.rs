//! `runner_ps3`: the standalone front end to the PS3 runner.
//!
//! The verbs live in the library's `verbs` module. This binary reads
//! the command line and the two variables, names the lease directory,
//! supplies the real console, sleep and clock, and prints the report. A
//! failure prints what the verb produced, then its message, and exits
//! with its class's code.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI binary: stdout carries the report, stderr the diagnostic"
)]

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode as ProcessExit;

use runner_ps3::cli;
use runner_ps3::env;
use runner_ps3::provenance;
use runner_ps3::run::WebmanConsole;
use runner_ps3::transport::Endpoint;
use runner_ps3::verbs::{self, Context, Failure, Report};
use runner_ps3::ExitCode;

/// The command every remedy this front end prints starts with.
const INVOCATION: &str = "runner_ps3";

fn main() -> ProcessExit {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut sleep = std::thread::sleep;
    let outcome = cli::parse(&args)
        .map_err(Box::<Failure>::from)
        .and_then(|command| {
            let context = Context {
                host_env: var(env::HOST),
                profile_env: var(env::PROFILE),
                lease_dir: std::env::temp_dir(),
                workspace_root: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
                invocation: INVOCATION.to_string(),
            };
            verbs::execute(
                &command,
                &context,
                |host| WebmanConsole::new(Endpoint::new(host)),
                &mut sleep,
                &mut provenance::now_rfc3339,
            )
        });
    match outcome {
        Ok(report) => {
            print(&report);
            ProcessExit::from(ExitCode::Ok.code() as u8)
        }
        Err(failure) => {
            if let Some(report) = &failure.report {
                print(report);
            }
            if let Some(lease) = &failure.lease {
                eprintln!("{INVOCATION}: lease: {lease}");
            }
            eprintln!("{INVOCATION}: {}", failure.error);
            if failure.error.exit_code() == ExitCode::Usage {
                eprintln!("{}", cli::USAGE);
            }
            ProcessExit::from(failure.error.exit_code().code() as u8)
        }
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var_os(name).and_then(|value| value.into_string().ok())
}

fn print(report: &Report) {
    for line in report.lines() {
        println!("{line}");
    }
}
