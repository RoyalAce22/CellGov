//! Every verb, for any front end: the standalone `runner_ps3` binary,
//! or another command line that links this library.
//!
//! [`execute`] takes the parsed command, the [`Context`] its front end
//! resolved, and the console, sleep and clock to use. It returns a
//! [`Report`] and prints nothing. Every remedy the runner names starts
//! with [`Context::invocation`], so a remedy names the front end that
//! ran the verb.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cellgov_observation::console_profile::ConsoleProfiles;
use cellgov_observation::hardware_capture::CAPTURE_DIR;
use cellgov_observation::manifest::{self, ConsoleManifest};
use serde::{Deserialize, Serialize};

use crate::capture::{self, CapturePlan};
use crate::cli::{Command, Verb};
use crate::console::{self, StatusReport};
use crate::deploy::{self, Package};
use crate::error::RunnerPs3Error;
use crate::lease::{self, Lease, LeaseError};
use crate::load::{self, Interlock};
use crate::run::{self, ConsoleOps, Reclaim, Target};
use crate::transcript::Transcript;

/// What a front end resolves before a verb runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    /// The value of [`crate::env::HOST`], when set.
    pub host_env: Option<String>,
    /// The value of [`crate::env::PROFILE`], when set.
    pub profile_env: Option<String>,
    /// The directory every runner on this machine keeps its hot markers
    /// in.
    pub marker_dir: PathBuf,
    /// The user and machine running the front end, as `user@machine`; a
    /// lease names them to every other runner.
    pub who: String,
    /// The workspace root; the tracked profiles file under it is the
    /// default when `--profiles` is absent.
    pub workspace_root: PathBuf,
    /// The words that run this front end, such as `runner_ps3`. Every
    /// remedy the runner names starts with them.
    pub invocation: String,
}

/// A capture the runner wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureReport {
    /// The capture's identifier.
    pub capture_id: String,
    /// The directory it went to.
    pub out: PathBuf,
    /// Why it replaced a committed capture; `None` for a first capture.
    pub replaced: Option<String>,
}

/// What a verb produced. Its JSON form ([`Report::json`]) carries each
/// variant's fields with no tag: `status` its facts, claim and verdict,
/// `capture` its capture and transcript, `unlock` its host and whether a
/// it removed a lease, and the other console verbs their transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Report {
    /// `unlock`: whether a lease on `host` was there to remove.
    Unlock {
        /// The console.
        host: String,
        /// Whether `unlock` found a lease file and removed it.
        removed: bool,
    },
    /// `convert`: the observation, and the file it went to.
    Convert {
        /// The observation as JSON.
        json: String,
        /// `--out`; `None` when the JSON is the report.
        out: Option<PathBuf>,
    },
    /// `status`: the facts, the claim and its verdict, and every other
    /// profile the console satisfies.
    Status(Box<StatusReport>),
    /// A verb that changes the console.
    Console {
        /// The capture, for a `capture` that wrote one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture: Option<CaptureReport>,
        /// Every request, reply and decision, redacted.
        transcript: Vec<String>,
    },
}

impl Report {
    /// The report as lines of text, in order.
    pub fn lines(&self) -> Vec<String> {
        match self {
            Self::Unlock {
                host,
                removed: true,
            } => vec![format!("removed the lease on {host}")],
            Self::Unlock {
                host,
                removed: false,
            } => vec![format!("no lease on {host}")],
            Self::Convert { json, out: None } => vec![json.clone()],
            Self::Convert { out: Some(_), .. } => Vec::new(),
            Self::Status(report) => report.lines(),
            Self::Console {
                capture,
                transcript,
            } => capture
                .iter()
                .map(|c| format!("{} -> {}", c.capture_id, c.out.display()))
                .chain(transcript.iter().cloned())
                .collect(),
        }
    }

    /// The report as a front end prints it: its lines, or under `json`
    /// its JSON as one block.
    ///
    /// # Errors
    ///
    /// [`RunnerPs3Error::Serialize`] when the JSON form does not
    /// serialize.
    pub fn render(&self, json: bool) -> Result<Vec<String>, RunnerPs3Error> {
        if json {
            Ok(vec![self.json()?])
        } else {
            Ok(self.lines())
        }
    }

    /// The report as JSON: `convert`'s observation as it stands, and
    /// every other report as its fields.
    ///
    /// # Errors
    ///
    /// [`RunnerPs3Error::Serialize`] when the report does not serialize.
    pub fn json(&self) -> Result<String, RunnerPs3Error> {
        match self {
            Self::Convert { json, .. } => Ok(json.clone()),
            report => Ok(serde_json::to_string_pretty(report)?),
        }
    }
}

/// A verb that failed, with what it produced before the failure.
#[derive(Debug)]
pub struct Failure {
    /// Why the verb failed; its class is the exit code.
    pub error: RunnerPs3Error,
    /// `status`'s lines before a claim the console fails, or the
    /// transcript of a verb that changes the console.
    pub report: Option<Report>,
    /// The lease, when its release also failed after `error`. The
    /// operator clears it with `unlock`.
    pub lease: Option<LeaseError>,
}

impl From<RunnerPs3Error> for Box<Failure> {
    fn from(error: RunnerPs3Error) -> Self {
        Box::new(Failure {
            error,
            report: None,
            lease: None,
        })
    }
}

/// Run `command`'s verb. `connect` opens the console a host names, and
/// runs at most once; `sleep` waits between polls; `now` stamps a
/// capture; `may_reclaim` answers whether `--reclaim` may empty an
/// occupied game directory, shown the directory and what it holds.
///
/// # Errors
///
/// A [`Failure`] carrying the verb's [`RunnerPs3Error`], and whatever
/// the verb produced before it.
pub fn execute<C: ConsoleOps>(
    command: &Command,
    context: &Context,
    connect: impl FnOnce(&str) -> C,
    sleep: &mut dyn FnMut(Duration),
    now: &mut dyn FnMut() -> Result<String, RunnerPs3Error>,
    may_reclaim: &mut dyn FnMut(&Reclaim) -> Result<bool, RunnerPs3Error>,
) -> Result<Report, Box<Failure>> {
    match command.verb()? {
        Verb::Unlock => {
            let host = command.host(context.host_env.as_deref())?;
            let mut console = connect(&host);
            let removed = lease::unlock(&mut console, &mut Transcript::new())
                .map_err(RunnerPs3Error::from)?;
            Ok(Report::Unlock { host, removed })
        }
        Verb::Convert => Ok(convert(command, context)?),
        verb => on_console(verb, command, context, connect, sleep, now, may_reclaim),
    }
}

fn claimed_profile(command: &Command, context: &Context) -> Result<String, RunnerPs3Error> {
    console::claimed_profile(command.profile.as_deref(), context.profile_env.as_deref())
}

fn load_profiles(command: &Command, context: &Context) -> Result<ConsoleProfiles, RunnerPs3Error> {
    Ok(ConsoleProfiles::load(
        &command.profiles_path(&context.workspace_root),
    )?)
}

fn convert(command: &Command, context: &Context) -> Result<Report, RunnerPs3Error> {
    let claimed = claimed_profile(command, context)?;
    let observation = capture::convert(
        command.frame()?,
        command.manifest()?,
        &load_profiles(command, context)?,
        &claimed,
    )?;
    let json = capture::observation_json(&observation)?;
    if let Some(path) = &command.out {
        write(path, json.as_bytes())?;
    }
    Ok(Report::Convert {
        json,
        out: command.out.clone(),
    })
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

/// The `verb` line a refusal names, with every identity flag the verb
/// requires and the manifest when it takes one, run through
/// `invocation`.
fn verb_line(
    invocation: &str,
    verb: Verb,
    host: &str,
    claimed: &str,
    command: &Command,
    manifest_path: Option<&Path>,
) -> String {
    let mut words = vec![
        verb.name().to_string(),
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
    if let Some(manifest_path) = manifest_path {
        words.push("--manifest".to_string());
        words.push(manifest_path.display().to_string());
    }
    std::iter::once(invocation.to_string())
        .chain(words.iter().map(|word| shell_word(word)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every verb that talks to the console: identify it and check the
/// claim first, hold any verb but `cleanup` to the load interlock, then
/// take the lease for any verb that changes the console.
fn on_console<C: ConsoleOps>(
    verb: Verb,
    command: &Command,
    context: &Context,
    connect: impl FnOnce(&str) -> C,
    sleep: &mut dyn FnMut(Duration),
    now: &mut dyn FnMut() -> Result<String, RunnerPs3Error>,
    may_reclaim: &mut dyn FnMut(&Reclaim) -> Result<bool, RunnerPs3Error>,
) -> Result<Report, Box<Failure>> {
    let host = command.host(context.host_env.as_deref())?;
    let claimed = claimed_profile(command, context)?;
    let profiles = load_profiles(command, context)?;
    let job = Job::of(verb, command)?;
    let mut webman = connect(&host);
    let mut transcript = Transcript::new();
    let html = load::status_page(&mut webman, &mut transcript)?;
    let Some(job) = job else {
        let facts = console::identify(
            &console::parse_status_page(&html),
            &command.operator(),
            &claimed,
        )
        .map_err(RunnerPs3Error::from)?;
        let load = load::parse_load(&html);
        let thermal = match load.reading() {
            Ok(reading) => Some(load::assess(
                reading,
                &profiles.load,
                &context.marker_dir,
                &host,
            )?),
            Err(_) => None,
        };
        let (status, verdict) = console::status_report(&facts, &profiles, &claimed, load, thermal)
            .map_err(RunnerPs3Error::from)?;
        let report = Report::Status(Box::new(status));
        return match verdict {
            Ok(()) => Ok(report),
            Err(mismatch) => Err(Box::new(Failure {
                error: mismatch.into(),
                report: Some(report),
                lease: None,
            })),
        };
    };
    let mut facts = console::establish(
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
    if !matches!(action, Action::Cleanup) {
        let interlock = Interlock {
            limits: profiles.load,
            marker_dir: &context.marker_dir,
            host: &host,
            wait_cool: command.wait_cool,
            needs_space: matches!(action, Action::Deploy | Action::Capture { .. }),
            poll_with: verb_line(
                &context.invocation,
                Verb::Status,
                &host,
                &claimed,
                command,
                None,
            ),
        };
        let reading = load::admit(&mut webman, &html, &interlock, sleep, &mut transcript)?;
        if matches!(action, Action::Capture { .. }) {
            facts.load_at_start = Some(reading);
        }
    }
    let lease = Lease::acquire(
        &mut webman,
        &host,
        &lease::holder_text(&context.who, &manifest.test.name, &context.invocation),
        &context.invocation,
        &mut transcript,
    )
    .map_err(RunnerPs3Error::from)?;
    let target = Target::new(&manifest.ps3.appid, &manifest.result_file_name());
    let clear_with = verb_line(
        &context.invocation,
        Verb::Cleanup,
        &host,
        &claimed,
        command,
        Some(manifest_path),
    );
    let result = match action {
        Action::Deploy => run::preflight(
            &mut webman,
            &target,
            command.reclaim,
            may_reclaim,
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
        })
        .map(|()| None),
        Action::Run => run::clear_stale_result(&mut webman, &target, &clear_with, &mut transcript)
            .and_then(|()| run::start(&mut webman, &target, &mut transcript))
            .and_then(|()| {
                run::wait_for_result(
                    &mut webman,
                    &target,
                    manifest.ps3.timeout_ms,
                    command.poll_ms(),
                    sleep,
                    &mut transcript,
                )
            })
            .map(|()| None),
        Action::Fetch { out } => run::fetch_result(
            &mut webman,
            &target,
            manifest.ps3.timeout_ms,
            &mut transcript,
        )
        .and_then(|bytes| write(&out, &bytes))
        .map(|()| None),
        Action::Cleanup => run::cleanup(&mut webman, &target, &mut transcript).map(|()| None),
        Action::Capture { harness_revision } => {
            let out = command.out.clone().unwrap_or_else(|| {
                manifest_path
                    .parent()
                    .map_or_else(PathBuf::new, Path::to_path_buf)
                    .join(CAPTURE_DIR)
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
                sleep,
                now,
                may_reclaim,
                &mut transcript,
            )
            .map(|record| {
                Some(CaptureReport {
                    capture_id: record.capture_id,
                    out: plan.out.clone(),
                    replaced: plan.recapture_reason.clone(),
                })
            })
        }
    };
    let (capture, failed) = match result {
        Ok(capture) => (capture, None),
        Err(error) => (None, Some(error)),
    };
    let released = lease.release(&mut webman, &mut transcript);
    let report = Report::Console {
        capture,
        transcript: transcript.lines().to_vec(),
    };
    let (error, lease) = match (failed, released) {
        (None, Ok(())) => return Ok(report),
        (None, Err(lease_error)) => (lease_error.into(), None),
        (Some(error), Ok(())) => (error, None),
        // The verb's failure is the exit class; the lease failure still
        // reaches the operator, who clears it with `unlock`.
        (Some(error), Err(lease_error)) => (error, Some(lease_error)),
    };
    Err(Box::new(Failure {
        error,
        report: Some(report),
        lease,
    }))
}

#[cfg(test)]
#[path = "tests/verbs_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/report_json_tests.rs"]
mod report_json_tests;
