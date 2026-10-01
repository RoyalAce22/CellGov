//! `cellgov ps3`: the PS3 runner's verbs, driven through its library.
//!
//! This module maps the clap arguments onto the runner's command,
//! prints the report the library returns, and maps the runner's failure
//! classes onto `cellgov`'s exit contract. Every verb's behaviour lives
//! in `runner_ps3::verbs`.

use cellgov_compare::console_profile::ConsoleProfileError;
use runner_ps3::cli::{Command as RunnerCommand, Verb};
use runner_ps3::console::ConsoleError;
use runner_ps3::lease::LeaseError;
use runner_ps3::run::{Reclaim, WebmanConsole};
use runner_ps3::transport::Endpoint;
use runner_ps3::verbs::{self, Context};
use runner_ps3::RunnerPs3Error;

use super::confirm::{self, Answers};
use super::exit::{CommandError, CommandExitCode};
use super::exit_codes;
use super::parse::{OutputFormat, Ps3Command, Ps3Debugger, Ps3Identity, Ps3ManifestArgs};

/// The command every remedy the runner prints starts with.
pub(crate) const INVOCATION: &str = "cellgov ps3";

/// The runner refused before changing the console.
pub(crate) const EXIT_REFUSED: i32 = exit_codes::command_specific(50);
/// The console did not answer as the protocol requires.
pub(crate) const EXIT_TRANSPORT: i32 = exit_codes::command_specific(51);
/// The test left no result within the manifest's budget.
pub(crate) const EXIT_TIMEOUT: i32 = exit_codes::command_specific(52);
/// The fetched bytes are not one whole CGOV frame.
pub(crate) const EXIT_FRAME: i32 = exit_codes::command_specific(53);
/// Cleanup after a capture left something on the console.
pub(crate) const EXIT_CLEANUP: i32 = exit_codes::command_specific(54);

/// Run `command` against the console its flags or the environment name,
/// and print its report: as JSON under `--format json`. `--reclaim`
/// asks before it empties an occupied game directory, and `answers`
/// (`--yes`, `--no-input`) answers for a run that cannot ask.
///
/// # Errors
///
/// The runner's failure, with its status from [`exit_code`].
pub(crate) fn run(
    command: &Ps3Command,
    format: OutputFormat,
    answers: Answers,
) -> Result<CommandExitCode, CommandError> {
    let json = format == OutputFormat::Json;
    let context = Context {
        host_env: var(runner_ps3::env::HOST),
        profile_env: var(runner_ps3::env::PROFILE),
        lease_dir: runner_ps3::lease::default_dir(),
        workspace_root: crate::paths::workspace_root(),
        invocation: INVOCATION.to_string(),
    };
    let mut sleep = std::thread::sleep;
    let outcome = verbs::execute(
        &runner_command(command, json),
        &context,
        |host| WebmanConsole::new(Endpoint::new(host)),
        &mut sleep,
        &mut runner_ps3::provenance::now_rfc3339,
        &mut may_reclaim(answers),
    );
    match outcome {
        Ok(report) => {
            let lines = report
                .render(json)
                .map_err(|error| CommandError::status(exit_code(&error), error.to_string()))?;
            print(&lines);
            Ok(CommandExitCode::SUCCESS)
        }
        Err(failure) => {
            if let Some(Ok(lines)) = failure.report.as_ref().map(|r| r.render(json)) {
                print(&lines);
            }
            if let Some(lease) = &failure.lease {
                eprintln!("{INVOCATION}: lease: {lease}");
            }
            Err(CommandError::status(
                exit_code(&failure.error),
                format!("{INVOCATION}: {}", failure.error),
            ))
        }
    }
}

/// The way out of the reclaim prompt other than `--yes`.
const WITHOUT_RECLAIM: &str = "rerun without --reclaim to leave the directory";

/// The reclaim question, naming the directory and every path in it.
pub(crate) fn reclaim_question(reclaim: &Reclaim) -> String {
    let mut question = format!(
        "--reclaim empties {} on the console, removing:",
        reclaim.game_dir
    );
    for path in &reclaim.contents {
        question.push_str(
            "
  ",
        );
        question.push_str(path);
    }
    question.push_str(
        "
empty it?",
    );
    question
}

/// The answer to a reclaim question: a prompt, `--yes`, or, for a run
/// that cannot ask, a usage error.
pub(crate) fn may_reclaim(
    answers: Answers,
) -> impl FnMut(&Reclaim) -> Result<bool, RunnerPs3Error> {
    move |reclaim| {
        confirm::decide(&reclaim_question(reclaim), answers, WITHOUT_RECLAIM)
            .map_err(|cannot| RunnerPs3Error::Usage(cannot.to_string()))
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var_os(name).and_then(|value| value.into_string().ok())
}

fn print(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

/// The runner's command for `command`; `json` is the global `--format
/// json`, the runner's `--json`.
pub(crate) fn runner_command(command: &Ps3Command, json: bool) -> RunnerCommand {
    let mut out = RunnerCommand {
        json,
        ..RunnerCommand::default()
    };
    match command {
        Ps3Command::Status(identity) => {
            out.verb = Some(Verb::Status);
            set_identity(&mut out, identity);
        }
        Ps3Command::Deploy(args) => {
            out.verb = Some(Verb::Deploy);
            set_target(&mut out, &args.target);
            out.reclaim = args.reclaim;
        }
        Ps3Command::Run(args) => {
            out.verb = Some(Verb::Run);
            set_target(&mut out, &args.target);
            out.poll_ms = args.poll_ms;
        }
        Ps3Command::Fetch(args) => {
            out.verb = Some(Verb::Fetch);
            set_target(&mut out, &args.target);
            out.out = Some(args.out.clone());
        }
        Ps3Command::Cleanup(args) => {
            out.verb = Some(Verb::Cleanup);
            set_target(&mut out, args);
        }
        Ps3Command::Capture(args) => {
            out.verb = Some(Verb::Capture);
            set_target(&mut out, &args.target);
            out.harness_revision = Some(args.harness_revision.clone());
            out.out = args.out.clone();
            out.poll_ms = args.poll_ms;
            out.reclaim = args.reclaim;
            out.keep_deployed = args.keep_deployed;
            out.recapture = args.recapture;
            out.reason = args.reason.clone();
        }
        Ps3Command::Convert(args) => {
            out.verb = Some(Verb::Convert);
            out.frame = Some(args.frame.clone());
            out.manifest = Some(args.manifest.clone());
            out.profile = args.profile.clone();
            out.profiles = args.profiles.clone();
            out.out = args.out.clone();
        }
        Ps3Command::Unlock(args) => {
            out.verb = Some(Verb::Unlock);
            out.host = args.host.clone();
        }
    }
    out
}

fn set_identity(out: &mut RunnerCommand, identity: &Ps3Identity) {
    out.host = identity.host.clone();
    out.profile = identity.profile.clone();
    out.profiles = identity.profiles.clone();
    out.model = identity.model.clone();
    out.cfw = identity.cfw.clone();
    out.debugger = identity.debugger.map(|d| d == Ps3Debugger::Attached);
}

fn set_target(out: &mut RunnerCommand, target: &Ps3ManifestArgs) {
    set_identity(out, &target.identity);
    out.manifest = Some(target.manifest.clone());
}

/// `cellgov`'s status for a runner failure.
///
/// A bad or missing flag is the shared usage status. An input file that
/// does not load, and a local write or clock failure, are the shared
/// failed status, as `cellgov`'s other commands give them. The runner's
/// own outcomes take the 50-54 band.
pub(crate) fn exit_code(error: &RunnerPs3Error) -> i32 {
    match error {
        RunnerPs3Error::Usage(_)
        | RunnerPs3Error::Console(ConsoleError::OperatorMissing { .. }) => exit_codes::USAGE,
        RunnerPs3Error::LocalRead { .. }
        | RunnerPs3Error::Manifest(_)
        | RunnerPs3Error::Capture(_)
        | RunnerPs3Error::Profile(
            ConsoleProfileError::Io { .. }
            | ConsoleProfileError::Parse(_)
            | ConsoleProfileError::UnknownReference(_)
            | ConsoleProfileError::NoModels(_),
        )
        | RunnerPs3Error::LocalWrite { .. }
        | RunnerPs3Error::HostClock
        | RunnerPs3Error::Serialize(_)
        | RunnerPs3Error::Lease(LeaseError::Io { .. }) => exit_codes::FAILED,
        RunnerPs3Error::Refused { .. }
        | RunnerPs3Error::Lease(LeaseError::Held { .. })
        | RunnerPs3Error::Console(ConsoleError::Contradiction { .. })
        | RunnerPs3Error::Profile(
            ConsoleProfileError::Mismatch { .. } | ConsoleProfileError::UnknownProfile { .. },
        ) => EXIT_REFUSED,
        RunnerPs3Error::Transport(_)
        | RunnerPs3Error::Console(ConsoleError::PageMissing { .. }) => EXIT_TRANSPORT,
        RunnerPs3Error::Timeout { .. } => EXIT_TIMEOUT,
        RunnerPs3Error::Frame(_) => EXIT_FRAME,
        RunnerPs3Error::Cleanup { .. } => EXIT_CLEANUP,
    }
}

#[cfg(test)]
#[path = "tests/ps3_tests.rs"]
mod tests;
