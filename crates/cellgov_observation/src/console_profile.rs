//! Named console profiles: the hardware class a console capture assumes.
//!
//! One file holds every profile, at [`console_profiles_path`] under the
//! workspace root. A profile
//! names a class of console, never a unit: it carries only the hard
//! fields, the ones that can change what a microtest observes (the
//! board family, the kernel, the system software version, the CFW and
//! Cobra, which both patch LV2, and whether a debugger holds the
//! console). Every other console fact, such as the exact model string,
//! the webMAN version or the CFW build string, is soft: the runner
//! records it in the transcript and the provenance and never refuses on
//! it.
//!
//! The operator claims one profile per run; [`ConsoleProfiles::check`]
//! refuses a console that does not satisfy every hard field of the
//! claim, and names any other tracked profile it does satisfy.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::hardware_capture::ConsoleFacts;

/// The microtest directory, relative to the workspace root.
pub const MICRO_DIR: &str = "tests/micro";

/// The profiles file's name, under [`MICRO_DIR`].
pub const CONSOLE_PROFILES_FILE: &str = "console_profiles.toml";

/// The profiles file under the workspace `root`: the one default every
/// front end resolves, whatever its working directory.
pub fn console_profiles_path(root: &Path) -> PathBuf {
    root.join(MICRO_DIR).join(CONSOLE_PROFILES_FILE)
}

/// Every tracked profile and the one the committed assertions are held
/// to.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleProfiles {
    /// The profile the committed assertions are held to.
    pub reference: String,
    /// Each profile by name.
    pub profile: BTreeMap<String, ConsoleProfile>,
}

/// The hard fields of one class of console.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleProfile {
    /// Model prefixes; a model matches when it starts with one of them.
    pub models: Vec<String>,
    /// `cex` or `dex`.
    pub kernel: String,
    /// The exact system software version.
    pub firmware: String,
    /// The CFW name; the build string after it is soft.
    pub cfw: String,
    /// The exact Cobra payload version.
    pub cobra: String,
    /// Whether a debugger holds the console.
    pub debugger_attached: bool,
}

/// One hard field the console does not satisfy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldMismatch {
    /// The field name, as the profiles file spells it.
    pub field: &'static str,
    /// What the profile requires.
    pub expected: String,
    /// What the console reports.
    pub observed: String,
}

impl fmt::Display for FieldMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is {:?}, the profile requires {}",
            self.field, self.observed, self.expected
        )
    }
}

/// Why the profiles file did not load, or a console failed its claim.
#[derive(Debug, thiserror::Error)]
pub enum ConsoleProfileError {
    /// The file cannot be read.
    #[error("console profiles {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file does not parse, or carries a field no profile has.
    #[error("console profiles: {0}")]
    Parse(#[from] toml::de::Error),
    /// `reference` names no profile in the file.
    #[error("console profiles: the reference {0:?} names no profile")]
    UnknownReference(String),
    /// A profile lists no model prefix, so no console can satisfy it, or
    /// an empty one, which every console would satisfy.
    #[error("console profile {0:?} lists no model, or an empty model prefix")]
    NoModels(String),
    /// The run claimed a profile the file does not hold.
    #[error("console profile {claimed:?} is not tracked; the tracked profiles are {known}")]
    UnknownProfile {
        /// The claimed name.
        claimed: String,
        /// The tracked names, comma-separated.
        known: String,
    },
    /// The console does not satisfy a hard field of the claimed profile.
    #[error(
        "the console does not satisfy profile {profile:?}: {}; {}",
        join(mismatches),
        satisfied_hint(satisfied)
    )]
    Mismatch {
        /// The claimed profile.
        profile: String,
        /// Each hard field it fails, in file order.
        mismatches: Vec<FieldMismatch>,
        /// Every other tracked profile the console satisfies.
        satisfied: Vec<String>,
    },
}

fn join(mismatches: &[FieldMismatch]) -> String {
    mismatches
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn satisfied_hint(satisfied: &[String]) -> String {
    if satisfied.is_empty() {
        format!("no tracked profile matches it; add one to {MICRO_DIR}/{CONSOLE_PROFILES_FILE}")
    } else {
        let flags: Vec<String> = satisfied.iter().map(|n| format!("--profile {n}")).collect();
        format!("it satisfies {}", flags.join(" or "))
    }
}

impl ConsoleProfile {
    /// Each hard field `console` fails, in file order.
    pub fn mismatches(&self, console: &ConsoleFacts) -> Vec<FieldMismatch> {
        let mut out = Vec::new();
        let mut require = |field, holds: bool, expected: String, observed: &str| {
            if !holds {
                out.push(FieldMismatch {
                    field,
                    expected,
                    observed: observed.to_string(),
                });
            }
        };
        require(
            "models",
            self.models
                .iter()
                .any(|p| console.model.starts_with(p.as_str())),
            format!("a model starting with one of {:?}", self.models),
            &console.model,
        );
        require(
            "kernel",
            console.kernel == self.kernel,
            format!("{:?}", self.kernel),
            &console.kernel,
        );
        require(
            "firmware",
            console.firmware == self.firmware,
            format!("{:?}", self.firmware),
            &console.firmware,
        );
        require(
            "cfw",
            cfw_matches(&console.cfw, &self.cfw),
            format!("{:?} with any build string after it", self.cfw),
            &console.cfw,
        );
        require(
            "cobra",
            console.cobra == self.cobra,
            format!("{:?}", self.cobra),
            &console.cobra,
        );
        require(
            "debugger_attached",
            console.debugger_attached == self.debugger_attached,
            self.debugger_attached.to_string(),
            &console.debugger_attached.to_string(),
        );
        out
    }
}

/// The CFW name matches when the observed string is the name, or the
/// name followed by a separator and a build string. A separator is any
/// character that is not a letter or a digit, so `EvilNAT 4.93`,
/// `EvilNAT-4.93` and `EvilNAT\t4.93` match and `EvilNATX` does not.
fn cfw_matches(observed: &str, name: &str) -> bool {
    observed
        .strip_prefix(name)
        .is_some_and(|rest| rest.chars().next().is_none_or(|c| !c.is_alphanumeric()))
}

impl ConsoleProfiles {
    /// Parse and check the profiles file text.
    ///
    /// # Errors
    ///
    /// [`ConsoleProfileError::Parse`] for bad TOML or an unknown field,
    /// [`ConsoleProfileError::UnknownReference`], and
    /// [`ConsoleProfileError::NoModels`].
    pub fn parse(text: &str) -> Result<Self, ConsoleProfileError> {
        let profiles: Self = toml::from_str(text)?;
        if !profiles.profile.contains_key(&profiles.reference) {
            return Err(ConsoleProfileError::UnknownReference(
                profiles.reference.clone(),
            ));
        }
        if let Some((name, _)) = profiles
            .profile
            .iter()
            .find(|(_, p)| p.models.is_empty() || p.models.iter().any(String::is_empty))
        {
            return Err(ConsoleProfileError::NoModels(name.clone()));
        }
        Ok(profiles)
    }

    /// Read and parse the profiles file at `path`.
    ///
    /// # Errors
    ///
    /// [`ConsoleProfileError::Io`] or a [`Self::parse`] error.
    pub fn load(path: &Path) -> Result<Self, ConsoleProfileError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConsoleProfileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text)
    }

    /// Every profile `console` satisfies, in name order.
    pub fn satisfied_by(&self, console: &ConsoleFacts) -> Vec<String> {
        self.profile
            .iter()
            .filter(|(_, p)| p.mismatches(console).is_empty())
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Check `console` against the hard fields of the `claimed` profile.
    ///
    /// # Errors
    ///
    /// [`ConsoleProfileError::UnknownProfile`] for a name the file does
    /// not hold, and [`ConsoleProfileError::Mismatch`] naming every
    /// failed field and every other profile the console satisfies.
    pub fn check(&self, claimed: &str, console: &ConsoleFacts) -> Result<(), ConsoleProfileError> {
        let profile =
            self.profile
                .get(claimed)
                .ok_or_else(|| ConsoleProfileError::UnknownProfile {
                    claimed: claimed.to_string(),
                    known: self.profile.keys().cloned().collect::<Vec<_>>().join(", "),
                })?;
        let mismatches = profile.mismatches(console);
        if mismatches.is_empty() {
            return Ok(());
        }
        Err(ConsoleProfileError::Mismatch {
            profile: claimed.to_string(),
            mismatches,
            satisfied: self.satisfied_by(console),
        })
    }
}

#[cfg(test)]
#[path = "tests/console_profile_tests.rs"]
mod tests;
