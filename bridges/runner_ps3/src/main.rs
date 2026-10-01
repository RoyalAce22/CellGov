//! `runner_ps3`: the command-line entry to the PS3 runner.
//!
//! The verbs live in the library; this binary reads the command line
//! and the two variables, names the lease directory, supplies the real
//! console, clock and sleep, and prints. A failure prints its message
//! and exits with its class's code.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI binary: stdout carries the report, stderr the diagnostic"
)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode as ProcessExit;

use cellgov_compare::console_profile::ConsoleProfiles;
use cellgov_compare::manifest::{self, ConsoleManifest};
use runner_ps3::capture::{self, CapturePlan};
use runner_ps3::cli::{self, Command, Verb, HOST_ENV};
use runner_ps3::console::{self, PROFILE_ENV, STATUS_PATH};
use runner_ps3::deploy::{self, Package};
use runner_ps3::lease::{self, Lease};
use runner_ps3::provenance;
use runner_ps3::run::{self, ConsoleOps, Target, WebmanConsole};
use runner_ps3::transcript::Transcript;
use runner_ps3::transport::{Endpoint, TransportError};
use runner_ps3::{ExitCode, RunnerPs3Error};

fn main() -> ProcessExit {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match cli::parse(&args).and_then(|command| execute(&command)) {
        Ok(()) => ProcessExit::from(ExitCode::Ok.code() as u8),
        Err(error) => {
            eprintln!("runner_ps3: {error}");
            if error.exit_code() == ExitCode::Usage {
                eprintln!("{}", cli::USAGE);
            }
            ProcessExit::from(error.exit_code().code() as u8)
        }
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var_os(name).and_then(|value| value.into_string().ok())
}

/// The directory every runner on this machine keeps its leases in.
fn lease_dir() -> PathBuf {
    std::env::temp_dir()
}

fn execute(command: &Command) -> Result<(), RunnerPs3Error> {
    let verb = command.verb()?;
    match verb {
        Verb::Unlock => {
            let host = command.host(env(HOST_ENV).as_deref())?;
            let removed = lease::unlock(&lease_dir(), &host)?;
            println!(
                "{}",
                if removed {
                    format!("removed the lease on {host}")
                } else {
                    format!("no lease on {host}")
                }
            );
            Ok(())
        }
        Verb::Convert => convert(command),
        _ => on_console(verb, command),
    }
}

fn load_profiles(command: &Command) -> Result<ConsoleProfiles, RunnerPs3Error> {
    Ok(ConsoleProfiles::load(&command.profiles_path())?)
}

fn convert(command: &Command) -> Result<(), RunnerPs3Error> {
    let claimed =
        console::claimed_profile(command.profile.as_deref(), env(PROFILE_ENV).as_deref())?;
    let observation = capture::convert(
        command.frame()?,
        command.manifest()?,
        &load_profiles(command)?,
        &claimed,
    )?;
    let json = capture::observation_json(&observation)?;
    match &command.out {
        Some(path) => write(path, json.as_bytes()),
        None => {
            println!("{json}");
            Ok(())
        }
    }
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), RunnerPs3Error> {
    std::fs::write(path, bytes).map_err(|source| RunnerPs3Error::LocalWrite {
        path: path.to_path_buf(),
        source,
    })
}

/// What a verb that changes the console does, with the inputs only it
/// needs.
enum Action {
    Deploy,
    Run,
    Fetch { out: PathBuf },
    Cleanup,
    Capture { harness_revision: String },
}

/// A verb that changes the console, with every local input it needs.
/// The runner resolves these before it sends a request, so a missing
/// or unreadable input is a usage error whatever the console's state.
struct Job {
    action: Action,
    manifest_path: PathBuf,
    manifest: ConsoleManifest,
}

impl Job {
    /// The job for `verb`, or `None` for a verb that changes nothing on
    /// the console.
    fn of(verb: Verb, command: &Command) -> Result<Option<Self>, RunnerPs3Error> {
        let action = match verb {
            Verb::Status | Verb::Convert | Verb::Unlock => return Ok(None),
            Verb::Deploy => Action::Deploy,
            Verb::Run => Action::Run,
            Verb::Fetch => Action::Fetch {
                out: command
                    .out
                    .clone()
                    .ok_or_else(|| RunnerPs3Error::Usage("--out is required".to_string()))?,
            },
            Verb::Cleanup => Action::Cleanup,
            Verb::Capture => Action::Capture {
                harness_revision: command.harness_revision.clone().ok_or_else(|| {
                    RunnerPs3Error::Usage(
                        "--harness-revision is required: the runner cannot read git".to_string(),
                    )
                })?,
            },
        };
        let manifest_path = command.manifest()?.to_path_buf();
        let manifest = manifest::load_console(&manifest_path)?;
        Ok(Some(Self {
            action,
            manifest_path,
            manifest,
        }))
    }
}

/// `word`, quoted when a shell would split it.
fn shell_word(word: &str) -> String {
    if word.is_empty() || word.contains(|c: char| c.is_whitespace() || c == '"') {
        format!("{word:?}")
    } else {
        word.to_string()
    }
}

/// The `cleanup` line that clears a refusal, with every identity flag
/// that verb requires.
fn cleanup_line(host: &str, claimed: &str, command: &Command, manifest_path: &Path) -> String {
    let mut words = vec![
        "runner_ps3".to_string(),
        "cleanup".to_string(),
        "--host".to_string(),
        host.to_string(),
        "--profile".to_string(),
        claimed.to_string(),
    ];
    if let Some(profiles) = &command.profiles {
        words.push("--profiles".to_string());
        words.push(profiles.display().to_string());
    }
    if let Some(model) = &command.model {
        words.push("--model".to_string());
        words.push(model.clone());
    }
    if let Some(cfw) = &command.cfw {
        words.push("--cfw".to_string());
        words.push(cfw.clone());
    }
    if let Some(attached) = command.debugger {
        words.push("--debugger".to_string());
        words.push(if attached { "attached" } else { "none" }.to_string());
    }
    words.push("--manifest".to_string());
    words.push(manifest_path.display().to_string());
    words
        .iter()
        .map(String::as_str)
        .map(shell_word)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every verb that talks to the console: identify it and check the
/// claim first, then take the lease for any verb that changes it.
fn on_console(verb: Verb, command: &Command) -> Result<(), RunnerPs3Error> {
    let host = command.host(env(HOST_ENV).as_deref())?;
    let claimed =
        console::claimed_profile(command.profile.as_deref(), env(PROFILE_ENV).as_deref())?;
    let profiles = load_profiles(command)?;
    let job = Job::of(verb, command)?;
    let mut webman = WebmanConsole::new(Endpoint::new(host.clone()));
    let mut transcript = Transcript::new();
    let page = webman.fetch(STATUS_PATH, &mut transcript)?.ok_or_else(|| {
        TransportError::UnexpectedStatus {
            path: STATUS_PATH.to_string(),
            status: 404,
        }
    })?;
    let html = String::from_utf8_lossy(&page);
    let Some(job) = job else {
        let facts = console::identify(
            &console::parse_status_page(&html),
            &command.operator(),
            &claimed,
        )?;
        let (lines, verdict) = console::status_report(&facts, &profiles, &claimed)?;
        for line in lines {
            println!("{line}");
        }
        return Ok(verdict?);
    };
    let facts = console::establish(
        &html,
        &command.operator(),
        &profiles,
        &claimed,
        &mut transcript,
    )?;
    let Job {
        action,
        manifest_path,
        manifest,
    } = job;
    let manifest_path = manifest_path.as_path();
    let lease = Lease::acquire(&lease_dir(), &host, &manifest.test.name)?;
    let target = Target::new(&manifest.ps3.appid, &manifest.result_file_name());
    let clear_with = cleanup_line(&host, &claimed, command, manifest_path);
    let mut sleep = std::thread::sleep;
    let result = match action {
        Action::Deploy => run::preflight(
            &mut webman,
            &target,
            command.reclaim,
            &clear_with,
            &mut transcript,
        )
        .and_then(|()| {
            deploy::deploy(
                &mut webman,
                &target,
                &Package::of(manifest_path, &manifest),
                &mut transcript,
            )
        }),
        Action::Run => run::clear_stale_result(&mut webman, &target, &clear_with, &mut transcript)
            .and_then(|()| run::start(&mut webman, &target, &mut transcript))
            .and_then(|()| {
                run::wait_for_result(
                    &mut webman,
                    &target,
                    manifest.ps3.timeout_ms,
                    command.poll_ms(),
                    &mut sleep,
                    &mut transcript,
                )
            }),
        Action::Fetch { out } => run::fetch_result(
            &mut webman,
            &target,
            manifest.ps3.timeout_ms,
            &mut transcript,
        )
        .and_then(|bytes| write(&out, &bytes)),
        Action::Cleanup => run::cleanup(&mut webman, &target, &mut transcript),
        Action::Capture { harness_revision } => {
            let out = command.out.clone().unwrap_or_else(|| {
                manifest_path
                    .parent()
                    .map_or_else(PathBuf::new, Path::to_path_buf)
                    .join(cellgov_compare::hardware_capture::CAPTURE_DIR)
                    .join(&claimed)
            });
            let plan = CapturePlan {
                manifest_path: manifest_path.to_path_buf(),
                manifest: manifest.clone(),
                out,
                harness_revision,
                poll_ms: command.poll_ms(),
                reclaim: command.reclaim,
                keep_deployed: command.keep_deployed,
                recapture_reason: command.reason.clone(),
                clear_with,
            };
            capture::capture(
                &mut webman,
                &plan,
                &facts,
                &mut sleep,
                &mut provenance::now_rfc3339,
                &mut transcript,
            )
            .map(|record| println!("{} -> {}", record.capture_id, plan.out.display()))
        }
    };
    for line in transcript.lines() {
        println!("{line}");
    }
    let released = lease.release();
    if let (Err(_), Err(lease_error)) = (&result, &released) {
        // The verb's failure is the exit class; the lease failure still
        // reaches the operator, who clears it with `unlock`.
        eprintln!("runner_ps3: lease: {lease_error}");
    }
    result?;
    Ok(released?)
}
