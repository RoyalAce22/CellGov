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
use crate::peek::{PeekPlan, Space, DEFAULT_READS};

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
  peek     --host H --space lv1|lv2 --width 1|2|4|8 --address A[,A...] [--reads N]
           [--out FILE] [--json]
--host defaults to CELLGOV_PS3_HOST and --profile to CELLGOV_PS3_PROFILE.
--json prints the report as JSON; convert always prints JSON.
Every verb but convert waits its turn in the host-wide console queue.
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
    /// Read words of console memory through webMAN's peek pages.
    Peek,
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
            Self::Peek => "peek",
        }
    }

    /// Whether the verb talks to the console: every verb but `convert`,
    /// since the lease `unlock` clears lives on the console.
    pub fn touches_console(self) -> bool {
        !matches!(self, Self::Convert)
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
            Self::Peek => &[
                "--host",
                "--json",
                "--space",
                "--width",
                "--address",
                "--reads",
                "--out",
            ],
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
    /// `--space`.
    pub space: Option<Space>,
    /// `--width`.
    pub width: Option<u8>,
    /// `--address`, one or more addresses.
    pub addresses: Option<Vec<u64>>,
    /// `--reads`.
    pub reads: Option<u32>,
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
    let mut words = utf8_words(args)?.into_iter();
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
                set_valued_flag(&mut command, flag, value)?;
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

/// Borrow each argument as UTF-8, refusing the first that is not.
fn utf8_words(args: &[OsString]) -> Result<Vec<&str>, RunnerPs3Error> {
    args.iter()
        .enumerate()
        .map(|(position, arg)| {
            arg.to_str().ok_or_else(|| {
                usage(format!(
                    "argument {} is not UTF-8: {}",
                    position + 1,
                    arg.to_string_lossy()
                ))
            })
        })
        .collect()
}

/// Store `value` for the flag that takes one.
///
/// A value that does not parse refuses before a repeated flag does.
fn set_valued_flag(command: &mut Command, flag: &str, value: &str) -> Result<(), RunnerPs3Error> {
    let text = value.to_string();
    match flag {
        "--host" => set_once(&mut command.host, flag, text),
        "--profile" => set_once(&mut command.profile, flag, text),
        "--profiles" => set_once(&mut command.profiles, flag, PathBuf::from(value)),
        "--model" => set_once(&mut command.model, flag, text),
        "--cfw" => set_once(&mut command.cfw, flag, text),
        "--debugger" => set_once(&mut command.debugger, flag, parse_debugger(value)?),
        "--manifest" => set_once(&mut command.manifest, flag, PathBuf::from(value)),
        "--out" => set_once(&mut command.out, flag, PathBuf::from(value)),
        "--frame" => set_once(&mut command.frame, flag, PathBuf::from(value)),
        "--harness-revision" => set_once(&mut command.harness_revision, flag, text),
        "--poll-ms" => set_once(&mut command.poll_ms, flag, parse_poll_ms(value)?),
        "--reason" => set_once(&mut command.reason, flag, text),
        "--space" => set_once(&mut command.space, flag, parse_space(value)?),
        "--width" => set_once(&mut command.width, flag, parse_width(value)?),
        "--address" => set_once(&mut command.addresses, flag, parse_addresses(value)?),
        "--reads" => set_once(&mut command.reads, flag, parse_reads(value)?),
        _ => Err(usage(format!("unknown flag {flag}"))),
    }
}

/// `--space`: `lv1` or `lv2`.
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] for any other word.
pub fn parse_space(value: &str) -> Result<Space, RunnerPs3Error> {
    Space::from_name(value).ok_or_else(|| usage(format!("--space takes lv1 or lv2, not {value:?}")))
}

/// `--width`: 1, 2, 4 or 8 bytes.
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] for any other value.
pub fn parse_width(value: &str) -> Result<u8, RunnerPs3Error> {
    match value {
        "1" | "2" | "4" | "8" => Ok(value.parse().unwrap_or_default()),
        other => Err(usage(format!("--width takes 1, 2, 4 or 8, not {other:?}"))),
    }
}

/// One `--address`: `0x` and hex digits.
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] for a value that is not `0x` hex.
pub fn parse_address(value: &str) -> Result<u64, RunnerPs3Error> {
    // from_str_radix also takes a leading `+`, which is not a hex digit.
    value
        .strip_prefix("0x")
        .filter(|digits| digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|digits| u64::from_str_radix(digits, 16).ok())
        .ok_or_else(|| usage(format!("address {value:?} is not 0x hex")))
}

/// `--address`: addresses separated by commas.
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] for an address that is not `0x` hex.
pub fn parse_addresses(value: &str) -> Result<Vec<u64>, RunnerPs3Error> {
    value.split(',').map(parse_address).collect()
}

/// `--reads`: 1 to [`crate::peek::MAX_READS`].
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] for any other value.
pub fn parse_reads(value: &str) -> Result<u32, RunnerPs3Error> {
    value
        .parse::<u32>()
        .ok()
        .filter(|reads| (1..=crate::peek::MAX_READS).contains(reads))
        .ok_or_else(|| {
            usage(format!(
                "--reads takes 1 to {}, not {value:?}",
                crate::peek::MAX_READS
            ))
        })
}

/// `--debugger`: `true` for `attached`, `false` for `none`.
fn parse_debugger(value: &str) -> Result<bool, RunnerPs3Error> {
    match value {
        "none" => Ok(false),
        "attached" => Ok(true),
        other => Err(usage(format!(
            "--debugger takes none or attached, not {other:?}"
        ))),
    }
}

/// `--poll-ms`: a millisecond count above zero.
fn parse_poll_ms(value: &str) -> Result<u64, RunnerPs3Error> {
    let ms: u64 = value
        .parse()
        .map_err(|_| usage(format!("--poll-ms {value:?} is not a millisecond count")))?;
    if ms == 0 {
        return Err(usage("--poll-ms must be greater than zero".to_string()));
    }
    Ok(ms)
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

    /// The peek the flags name; `--reads` defaults to [`DEFAULT_READS`].
    ///
    /// # Errors
    ///
    /// [`RunnerPs3Error::Usage`] for an absent `--space`, `--width` or
    /// `--address`, and every refusal of [`PeekPlan::new`].
    pub fn peek_plan(&self) -> Result<PeekPlan, RunnerPs3Error> {
        let required = |flag: &str| usage(format!("{flag} is required"));
        PeekPlan::new(
            self.space.ok_or_else(|| required("--space"))?,
            self.width.ok_or_else(|| required("--width"))?,
            self.addresses
                .clone()
                .ok_or_else(|| required("--address"))?,
            self.reads.unwrap_or(DEFAULT_READS),
        )
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
