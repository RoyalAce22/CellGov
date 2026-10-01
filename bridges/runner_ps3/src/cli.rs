//! The command line: one verb, then its flags, each given at most once.
//!
//! A hand-rolled parser with no dependency: every flag is a word and,
//! for most, a value; a flag the verb does not take, a flag given twice,
//! and an argument that is not UTF-8 are each a usage error naming it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use cellgov_observation::console_profile::console_profiles_path;

use crate::console::OperatorFacts;
use crate::env;
use crate::error::RunnerPs3Error;

/// The wait between polls for the result when `--poll-ms` is absent.
pub const DEFAULT_POLL_MS: u64 = 500;

/// The usage text.
pub const USAGE: &str = "\
usage: runner_ps3 <verb> [flags]
  status   --host H --profile P --model M --cfw C --debugger none|attached [--profiles F]
           [--json]
  deploy   (status flags) --manifest F [--reclaim] [--wait-cool]
  run      (status flags) --manifest F [--poll-ms N] [--wait-cool]
  fetch    (status flags) --manifest F --out FILE [--wait-cool]
  cleanup  (status flags) --manifest F
  capture  (status flags) --manifest F --harness-revision SHA [--out DIR] [--poll-ms N]
           [--reclaim] [--keep-deployed] [--recapture --reason TEXT] [--wait-cool]
  convert  --frame FILE --manifest F --profile P [--profiles F] [--out FILE]
  unlock   --host H [--json]
--host defaults to CELLGOV_PS3_HOST and --profile to CELLGOV_PS3_PROFILE.
--json prints the report as JSON; convert always prints JSON.
deploy, run, fetch and capture refuse a hot console; --wait-cool waits for it to cool.";

/// A runner verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray)]
pub enum Verb {
    /// Read the console's identity and check it against the claim.
    Status,
    /// Copy the package to the console.
    Deploy,
    /// Start the deployed test and wait for its result.
    Run,
    /// Copy the result file to this machine.
    Fetch,
    /// Remove the package and the result from the console.
    Cleanup,
    /// The whole loop, writing a committed capture.
    Capture,
    /// Convert a frame offline.
    Convert,
    /// Remove a stale lease.
    Unlock,
}

impl Verb {
    /// The verb's word on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Deploy => "deploy",
            Self::Run => "run",
            Self::Fetch => "fetch",
            Self::Cleanup => "cleanup",
            Self::Capture => "capture",
            Self::Convert => "convert",
            Self::Unlock => "unlock",
        }
    }

    /// Whether the verb talks to the console.
    pub fn touches_console(self) -> bool {
        !matches!(self, Self::Convert | Self::Unlock)
    }

    /// Every flag the verb takes; the parser refuses any other, and
    /// [`USAGE`] names exactly these. A verb that talks to the console
    /// takes the six identity flags first.
    pub fn flags(self) -> &'static [&'static str] {
        match self {
            Self::Status => &[
                "--host",
                "--profile",
                "--profiles",
                "--model",
                "--cfw",
                "--debugger",
                "--json",
            ],
            Self::Deploy => &[
                "--host",
                "--profile",
                "--profiles",
                "--model",
                "--cfw",
                "--debugger",
                "--json",
                "--manifest",
                "--reclaim",
                "--wait-cool",
            ],
            Self::Run => &[
                "--host",
                "--profile",
                "--profiles",
                "--model",
                "--cfw",
                "--debugger",
                "--json",
                "--manifest",
                "--poll-ms",
                "--wait-cool",
            ],
            Self::Fetch => &[
                "--host",
                "--profile",
                "--profiles",
                "--model",
                "--cfw",
                "--debugger",
                "--json",
                "--manifest",
                "--out",
                "--wait-cool",
            ],
            Self::Cleanup => &[
                "--host",
                "--profile",
                "--profiles",
                "--model",
                "--cfw",
                "--debugger",
                "--json",
                "--manifest",
            ],
            Self::Capture => &[
                "--host",
                "--profile",
                "--profiles",
                "--model",
                "--cfw",
                "--debugger",
                "--json",
                "--manifest",
                "--harness-revision",
                "--out",
                "--poll-ms",
                "--reclaim",
                "--keep-deployed",
                "--recapture",
                "--reason",
                "--wait-cool",
            ],
            Self::Convert => &["--frame", "--manifest", "--profile", "--profiles", "--out"],
            Self::Unlock => &["--host", "--json"],
        }
    }
}

/// A parsed command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Command {
    /// The verb; `None` only in a value built by hand.
    pub verb: Option<Verb>,
    /// `--host`.
    pub host: Option<String>,
    /// `--profile`.
    pub profile: Option<String>,
    /// `--profiles`.
    pub profiles: Option<PathBuf>,
    /// `--model`.
    pub model: Option<String>,
    /// `--cfw`.
    pub cfw: Option<String>,
    /// `--debugger`: `Some(true)` for `attached`.
    pub debugger: Option<bool>,
    /// `--manifest`.
    pub manifest: Option<PathBuf>,
    /// `--out`.
    pub out: Option<PathBuf>,
    /// `--frame`.
    pub frame: Option<PathBuf>,
    /// `--harness-revision`.
    pub harness_revision: Option<String>,
    /// `--poll-ms`.
    pub poll_ms: Option<u64>,
    /// `--reclaim`.
    pub reclaim: bool,
    /// `--keep-deployed`.
    pub keep_deployed: bool,
    /// `--recapture`.
    pub recapture: bool,
    /// `--reason`.
    pub reason: Option<String>,
    /// `--json`.
    pub json: bool,
    /// `--wait-cool`.
    pub wait_cool: bool,
}

fn usage(message: String) -> RunnerPs3Error {
    RunnerPs3Error::Usage(message)
}

fn set_once<T>(slot: &mut Option<T>, flag: &str, value: T) -> Result<(), RunnerPs3Error> {
    if slot.is_some() {
        return Err(usage(format!("{flag} given more than once")));
    }
    *slot = Some(value);
    Ok(())
}

fn set_flag(slot: &mut bool, flag: &str) -> Result<(), RunnerPs3Error> {
    if *slot {
        return Err(usage(format!("{flag} given more than once")));
    }
    *slot = true;
    Ok(())
}

/// Parse `args`, the arguments after the program name.
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] naming the first argument that is not
/// UTF-8, an unknown verb or flag, a flag the verb does not take, a
/// flag given twice or with no value, or a value that does not parse.
pub fn parse(args: &[OsString]) -> Result<Command, RunnerPs3Error> {
    let mut words = Vec::with_capacity(args.len());
    for (position, arg) in args.iter().enumerate() {
        let word = arg.to_str().ok_or_else(|| {
            usage(format!(
                "argument {} is not UTF-8: {}",
                position + 1,
                arg.to_string_lossy()
            ))
        })?;
        words.push(word);
    }
    let mut words = words.into_iter();
    let verb_word = words
        .next()
        .ok_or_else(|| usage("no verb given".to_string()))?;
    let verb = <Verb as strum::VariantArray>::VARIANTS
        .iter()
        .copied()
        .find(|v| v.name() == verb_word)
        .ok_or_else(|| usage(format!("unknown verb {verb_word:?}")))?;
    let mut command = Command {
        verb: Some(verb),
        ..Command::default()
    };
    while let Some(flag) = words.next() {
        if !flag.starts_with("--") {
            return Err(usage(format!("unexpected argument {flag:?}")));
        }
        if !verb.flags().contains(&flag) {
            return Err(usage(format!("{} does not take {flag}", verb.name())));
        }
        match flag {
            "--reclaim" => set_flag(&mut command.reclaim, flag)?,
            "--keep-deployed" => set_flag(&mut command.keep_deployed, flag)?,
            "--recapture" => set_flag(&mut command.recapture, flag)?,
            "--json" => set_flag(&mut command.json, flag)?,
            "--wait-cool" => set_flag(&mut command.wait_cool, flag)?,
            _ => {
                let value = words
                    .next()
                    .ok_or_else(|| usage(format!("{flag} needs a value")))?;
                if value.starts_with("--") {
                    return Err(usage(format!("{flag} needs a value, not the flag {value}")));
                }
                let text = value.to_string();
                match flag {
                    "--host" => set_once(&mut command.host, flag, text)?,
                    "--profile" => set_once(&mut command.profile, flag, text)?,
                    "--profiles" => set_once(&mut command.profiles, flag, PathBuf::from(value))?,
                    "--model" => set_once(&mut command.model, flag, text)?,
                    "--cfw" => set_once(&mut command.cfw, flag, text)?,
                    "--debugger" => {
                        let attached = match value {
                            "none" => false,
                            "attached" => true,
                            other => {
                                return Err(usage(format!(
                                    "--debugger takes none or attached, not {other:?}"
                                )))
                            }
                        };
                        set_once(&mut command.debugger, flag, attached)?;
                    }
                    "--manifest" => set_once(&mut command.manifest, flag, PathBuf::from(value))?,
                    "--out" => set_once(&mut command.out, flag, PathBuf::from(value))?,
                    "--frame" => set_once(&mut command.frame, flag, PathBuf::from(value))?,
                    "--harness-revision" => set_once(&mut command.harness_revision, flag, text)?,
                    "--poll-ms" => {
                        let ms: u64 = value.parse().map_err(|_| {
                            usage(format!("--poll-ms {value:?} is not a millisecond count"))
                        })?;
                        if ms == 0 {
                            return Err(usage("--poll-ms must be greater than zero".to_string()));
                        }
                        set_once(&mut command.poll_ms, flag, ms)?;
                    }
                    "--reason" => set_once(&mut command.reason, flag, text)?,
                    _ => return Err(usage(format!("unknown flag {flag}"))),
                }
            }
        }
    }
    if command.recapture != command.reason.is_some() {
        return Err(usage(
            "--recapture and --reason go together: say why the capture is replaced".to_string(),
        ));
    }
    Ok(command)
}

impl Command {
    /// The verb.
    ///
    /// # Errors
    ///
    /// [`RunnerPs3Error::Usage`] for a value built with none.
    pub fn verb(&self) -> Result<Verb, RunnerPs3Error> {
        self.verb.ok_or_else(|| usage("no verb given".to_string()))
    }

    /// The console: `--host`, else `env` (the [`env::HOST`] variable).
    ///
    /// # Errors
    ///
    /// [`RunnerPs3Error::Usage`] naming both when neither is set.
    pub fn host(&self, env: Option<&str>) -> Result<String, RunnerPs3Error> {
        [self.host.as_deref(), env]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|v| !v.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                usage(format!(
                    "no console named; pass --host or set {}",
                    env::HOST
                ))
            })
    }

    /// `path`, or a usage error naming `flag`.
    fn required<'a>(path: Option<&'a Path>, flag: &str) -> Result<&'a Path, RunnerPs3Error> {
        path.ok_or_else(|| usage(format!("{flag} is required")))
    }

    /// `--manifest`.
    ///
    /// # Errors
    ///
    /// [`RunnerPs3Error::Usage`] when absent.
    pub fn manifest(&self) -> Result<&Path, RunnerPs3Error> {
        Self::required(self.manifest.as_deref(), "--manifest")
    }

    /// `--frame`.
    ///
    /// # Errors
    ///
    /// [`RunnerPs3Error::Usage`] when absent.
    pub fn frame(&self) -> Result<&Path, RunnerPs3Error> {
        Self::required(self.frame.as_deref(), "--frame")
    }

    /// `--profiles` as the operator typed it, or the tracked profiles
    /// file under `workspace_root`.
    pub fn profiles_path(&self, workspace_root: &Path) -> PathBuf {
        self.profiles
            .clone()
            .unwrap_or_else(|| console_profiles_path(workspace_root))
    }

    /// `--poll-ms`, or [`DEFAULT_POLL_MS`].
    pub fn poll_ms(&self) -> u64 {
        self.poll_ms.unwrap_or(DEFAULT_POLL_MS)
    }

    /// The facts the operator states.
    pub fn operator(&self) -> OperatorFacts {
        OperatorFacts {
            model: self.model.clone(),
            cfw: self.cfw.clone(),
            debugger_attached: self.debugger,
        }
    }
}

#[cfg(test)]
#[path = "tests/cli_tests.rs"]
mod tests;
