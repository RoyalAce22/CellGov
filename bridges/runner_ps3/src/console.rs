//! Which console the runner is talking to: the public identity read
//! from webMAN's status page and compared with the tracked console
//! profile. No unit identifier is ever extracted: the scan keeps three
//! words of the firmware line, the version word of the webMAN banner,
//! and a `CECH-` model token, and nothing else.
//!
//! The firmware, the kernel and Cobra are hard fields the page states;
//! a page that does not state one is a refusal naming it. The model and
//! the CFW name are hard fields the page in use does not state, so the
//! operator states them (`--model`, `--cfw`); an operator value may
//! fill a field the page leaves out and may not contradict one it
//! states. The webMAN version is soft: recorded, absent when the page
//! omits it, never a refusal.

use cellgov_compare::console_profile::{ConsoleProfileError, ConsoleProfiles};
use cellgov_compare::hardware_capture::ConsoleFacts;

use crate::error::RunnerPs3Error;
use crate::transcript::Transcript;

/// The variable that names the claimed profile when `--profile` is
/// absent.
pub const PROFILE_ENV: &str = "CELLGOV_PS3_PROFILE";

/// The status page webMAN serves.
pub const STATUS_PATH: &str = "/cpursx.ps3";

/// The profile a run claims: `--profile` when given, else
/// [`PROFILE_ENV`]. A blank value counts as absent.
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] naming both sources when neither is set.
pub fn claimed_profile(flag: Option<&str>, env: Option<&str>) -> Result<String, RunnerPs3Error> {
    stated(flag).or_else(|| stated(env)).ok_or_else(|| {
        RunnerPs3Error::Usage(format!(
            "no console profile claimed; pass --profile <name> or set {PROFILE_ENV}"
        ))
    })
}

/// What the status page states; each field is absent when the page
/// does not state it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusPage {
    /// System software version, such as `4.93`.
    pub firmware: Option<String>,
    /// `cex` or `dex`, lowercased.
    pub kernel: Option<String>,
    /// Cobra payload version.
    pub cobra: Option<String>,
    /// webMAN version, as the banner prints it.
    pub webman: Option<String>,
    /// A `CECH-` model name, when the page prints one.
    pub model: Option<String>,
}

/// The facts the operator states for fields the page leaves out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperatorFacts {
    /// `--model`.
    pub model: Option<String>,
    /// `--cfw`, the CFW name and any build string after it.
    pub cfw: Option<String>,
    /// `--debugger`: whether a debugger holds the console. The page does
    /// not say, and a hard field is never assumed.
    pub debugger_attached: Option<bool>,
}

/// Why the console's identity is not established.
#[derive(Debug, thiserror::Error)]
pub enum ConsoleError {
    /// The operator left out a hard field only the operator states.
    #[error("the console's {field} is not stated: the status page does not state it; pass {flag}")]
    OperatorMissing {
        /// The field.
        field: &'static str,
        /// The flag that states it.
        flag: &'static str,
    },
    /// The status page does not state a hard field webMAN states.
    #[error(
        "the console's {field} is not stated: check that webMAN answers {STATUS_PATH} with its \
         firmware line"
    )]
    PageMissing {
        /// The field.
        field: &'static str,
    },
    /// The operator states a value the page contradicts.
    #[error("--{field} {operator:?} contradicts the status page, which states {page:?}")]
    Contradiction {
        /// The field.
        field: &'static str,
        /// What the page states.
        page: String,
        /// What the operator states.
        operator: String,
    },
}

/// The page's facts, from its text with markup removed.
///
/// The firmware line reads `<storage> Firmware: <version> <kernel>
/// Cobra <version>`; the banner reads `webMAN <version> ...`.
pub fn parse_status_page(html: &str) -> StatusPage {
    let text = strip_markup(html);
    let firmware_words: Vec<&str> = text
        .lines()
        .find_map(|line| line.split_once("Firmware:").map(|(_, rest)| rest))
        .map(|rest| rest.split_whitespace().collect())
        .unwrap_or_default();
    let kernel = firmware_words
        .get(1)
        .filter(|k| k.eq_ignore_ascii_case("cex") || k.eq_ignore_ascii_case("dex"))
        .map(|k| k.to_ascii_lowercase());
    let cobra = firmware_words
        .iter()
        .position(|w| w.eq_ignore_ascii_case("cobra"))
        .and_then(|at| firmware_words.get(at + 1))
        .map(|v| (*v).to_string());
    let webman = text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some("webMAN"))
            .then(|| words.next())
            .flatten()
            .map(str::to_string)
    });
    let model = text
        .split_whitespace()
        .find(|w| w.starts_with("CECH-") && w.len() > "CECH-".len())
        .map(str::to_string);
    StatusPage {
        firmware: firmware_words.first().map(|v| (*v).to_string()),
        kernel,
        cobra,
        webman,
        model,
    }
}

/// The page with every `<...>` tag replaced by a line break.
fn strip_markup(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push('\n');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// A stated value with surrounding whitespace removed; blank is absent.
fn stated(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The console's facts under the `profile` the run claims.
///
/// # Errors
///
/// [`ConsoleError::PageMissing`] for a hard field the page should state
/// and does not, [`ConsoleError::OperatorMissing`] for one only the
/// operator states and did not, and [`ConsoleError::Contradiction`] for
/// an operator value the page contradicts.
pub fn identify(
    page: &StatusPage,
    operator: &OperatorFacts,
    profile: &str,
) -> Result<ConsoleFacts, ConsoleError> {
    let from_page = |value: &Option<String>, field| {
        stated(value.as_deref()).ok_or(ConsoleError::PageMissing { field })
    };
    let firmware = from_page(&page.firmware, "firmware")?;
    let kernel = from_page(&page.kernel, "kernel")?;
    let cobra = from_page(&page.cobra, "cobra")?;
    let page_model = stated(page.model.as_deref());
    let operator_model = stated(operator.model.as_deref());
    let model = match (page_model, operator_model) {
        (Some(page), Some(operator)) if page != operator => {
            return Err(ConsoleError::Contradiction {
                field: "model",
                page,
                operator,
            });
        }
        (Some(model), _) | (None, Some(model)) => model,
        (None, None) => {
            return Err(ConsoleError::OperatorMissing {
                field: "model",
                flag: "--model",
            });
        }
    };
    let cfw = stated(operator.cfw.as_deref()).ok_or(ConsoleError::OperatorMissing {
        field: "cfw",
        flag: "--cfw",
    })?;
    let debugger_attached = operator
        .debugger_attached
        .ok_or(ConsoleError::OperatorMissing {
            field: "debugger_attached",
            flag: "--debugger <none|attached>",
        })?;
    Ok(ConsoleFacts {
        profile: profile.to_string(),
        model,
        kernel,
        firmware,
        cfw,
        cobra,
        webman: stated(page.webman.as_deref()),
        debugger_attached,
    })
}

/// The console's facts, recorded in the transcript and checked against
/// the hard fields of the `claimed` profile.
///
/// # Errors
///
/// [`RunnerPs3Error::Console`] when the identity is not established, and
/// [`RunnerPs3Error::Profile`] when it fails the claim.
pub fn establish(
    html: &str,
    operator: &OperatorFacts,
    profiles: &ConsoleProfiles,
    claimed: &str,
    transcript: &mut Transcript,
) -> Result<ConsoleFacts, RunnerPs3Error> {
    let facts = identify(&parse_status_page(html), operator, claimed)?;
    transcript.decision(format!(
        "console: model {}, kernel {}, firmware {}, cfw {}, cobra {}, webman {}, debugger {}",
        facts.model,
        facts.kernel,
        facts.firmware,
        facts.cfw,
        facts.cobra,
        facts.webman.as_deref().unwrap_or("not stated"),
        if facts.debugger_attached {
            "attached"
        } else {
            "not attached"
        },
    ));
    profiles.check(claimed, &facts)?;
    transcript.decision(format!("console satisfies profile {claimed}"));
    Ok(facts)
}

/// The `status` report for `facts` under the `claimed` profile: the
/// claim, one verdict line per hard field, the soft fields, and every
/// other tracked profile the console satisfies. The second value is
/// the claim's own verdict.
///
/// # Errors
///
/// [`ConsoleProfileError::UnknownProfile`] when the file does not track
/// the claim.
pub fn status_report(
    facts: &ConsoleFacts,
    profiles: &ConsoleProfiles,
    claimed: &str,
) -> Result<(Vec<String>, Result<(), ConsoleProfileError>), ConsoleProfileError> {
    let verdict = profiles.check(claimed, facts);
    if let Err(unknown @ ConsoleProfileError::UnknownProfile { .. }) = verdict {
        return Err(unknown);
    }
    let failed = match &verdict {
        Err(ConsoleProfileError::Mismatch { mismatches, .. }) => mismatches.clone(),
        _ => Vec::new(),
    };
    let debugger = if facts.debugger_attached {
        "attached"
    } else {
        "none"
    };
    let mut lines = vec![format!("profile {claimed}")];
    for (field, observed) in [
        ("models", facts.model.as_str()),
        ("kernel", facts.kernel.as_str()),
        ("firmware", facts.firmware.as_str()),
        ("cfw", facts.cfw.as_str()),
        ("cobra", facts.cobra.as_str()),
        ("debugger_attached", debugger),
    ] {
        lines.push(match failed.iter().find(|m| m.field == field) {
            Some(m) => format!(
                "  {field}: {observed} FAILS, the profile requires {}",
                m.expected
            ),
            None => format!("  {field}: {observed} ok"),
        });
    }
    lines.push(format!(
        "soft: model {}, cfw {}, webman {}",
        facts.model,
        facts.cfw,
        facts.webman.as_deref().unwrap_or("not stated")
    ));
    let others: Vec<String> = profiles
        .satisfied_by(facts)
        .into_iter()
        .filter(|name| name != claimed)
        .collect();
    lines.push(if others.is_empty() {
        "also satisfies: none".to_string()
    } else {
        format!("also satisfies: {}", others.join(", "))
    });
    Ok((lines, verdict))
}

#[cfg(test)]
#[path = "tests/console_tests.rs"]
mod tests;
