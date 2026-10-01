//! The thermal and capacity interlock: the console's load, read from the
//! same status page its identity comes from, held to the [`LoadLimits`]
//! in the profiles file.
//!
//! The verbs that put work on the console (`deploy`, `run`, `fetch` and
//! `capture`) refuse while it is hot. `status`, `unlock` and `cleanup`
//! stay open: `status` is one GET, and leaving a game directory and a
//! mount behind because the console was hot is worse than a few FTP
//! deletes. A console is hot from the moment its hotter chip reads the
//! ceiling, and cool again only once it reads below the floor; the
//! marker at [`hot_path`] carries that state from one reading to the
//! next, across runs and front ends on this machine. `deploy`
//! and `capture` also refuse when `/dev_hdd0` holds less than the floor.
//!
//! The file-drop runner cannot see inside a run, so a run that heats the
//! console is caught at the next verb, not during it.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cellgov_observation::console_profile::LoadLimits;
use cellgov_observation::hardware_capture::LoadReading;
use serde::{Deserialize, Serialize};

use crate::console::{self, STATUS_PATH};
use crate::error::RunnerPs3Error;
use crate::run::ConsoleOps;
use crate::transcript::Transcript;
use crate::transport::TransportError;

/// The directory the hot markers live in, whichever front end runs
/// the verb: the machine's temp directory, with no process id in the
/// name.
pub fn default_marker_dir() -> PathBuf {
    std::env::temp_dir()
}

/// The marker under `dir` that says `host` was last read hot. Every
/// character of the host outside ASCII alphanumerics and `.` folds to
/// `_`.
pub fn hot_path(dir: &Path, host: &str) -> PathBuf {
    let safe: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("cellgov_runner_ps3_{safe}.hot"))
}

/// What the status page states about the console's load; each field is
/// absent when the page does not state it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleLoad {
    /// The Cell's temperature, in degrees Celsius.
    pub cpu_c: Option<u32>,
    /// The RSX's temperature, in degrees Celsius.
    pub rsx_c: Option<u32>,
    /// The fan speed, in percent.
    pub fan_percent: Option<u32>,
    /// The free space on `/dev_hdd0`, in bytes, the page's units taken
    /// as binary multiples.
    pub hdd_free_bytes: Option<u64>,
}

/// Where the console stands against the limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Thermal {
    /// Below the floor, or below the ceiling and not hot before.
    Cool,
    /// Below the ceiling but not yet below the floor since it was hot.
    Cooling,
    /// At or above the ceiling.
    Hot,
}

impl fmt::Display for Thermal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cool => "cool",
            Self::Cooling => "still cooling",
            Self::Hot => "hot",
        })
    }
}

impl Thermal {
    /// The state of a console whose hotter chip reads `hottest`, given
    /// whether the reading before it was hot.
    pub fn of(hottest: u32, limits: &LoadLimits, was_hot: bool) -> Self {
        if hottest >= limits.hot_c {
            Self::Hot
        } else if was_hot && hottest >= limits.cool_c {
            Self::Cooling
        } else {
            Self::Cool
        }
    }
}

/// Why the interlock refuses.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The page does not state a reading the interlock needs, so it
    /// cannot tell a cool console from a hot one.
    #[error(
        "the status page does not state the {0}; check that webMAN answers {STATUS_PATH} with \
         its CPU, RSX and HDD lines"
    )]
    Unstated(&'static str),
    /// The console is hot, or has not yet cooled below the floor.
    #[error(
        "the console is {thermal}: Cell {} C, RSX {} C; it is hot at {} C and cool again only \
         below {} C. Poll `{poll_with}` until it reads cool, or rerun with --wait-cool",
        reading.cpu_c,
        reading.rsx_c,
        limits.hot_c,
        limits.cool_c
    )]
    Hot {
        /// [`Thermal::Hot`] or [`Thermal::Cooling`].
        thermal: Thermal,
        /// The reading.
        reading: LoadReading,
        /// The limits it was held to.
        limits: LoadLimits,
        /// The `status` command that polls it.
        poll_with: String,
    },
    /// `--wait-cool` waited its limit and the console is still not cool.
    #[error(
        "the console is {thermal} after {waited_s} s of --wait-cool: Cell {} C, RSX {} C; it is \
         cool again only below {} C. Check its fan and airflow before rerunning",
        reading.cpu_c,
        reading.rsx_c,
        limits.cool_c
    )]
    StillHot {
        /// [`Thermal::Hot`] or [`Thermal::Cooling`].
        thermal: Thermal,
        /// The last reading.
        reading: LoadReading,
        /// The limits it was held to.
        limits: LoadLimits,
        /// How long the runner waited, in seconds.
        waited_s: u64,
    },
    /// `/dev_hdd0` holds less than a deploy needs.
    #[error(
        "/dev_hdd0 holds {free_bytes} bytes free and a deploy needs {required_bytes}; free space \
         on the console"
    )]
    Full {
        /// The free space the page states.
        free_bytes: u64,
        /// The floor, from the profiles file.
        required_bytes: u64,
    },
}

/// The degree sign, which the page prints itself or as `&deg;`.
const DEGREE: char = '\u{b0}';

/// The console's load from the status page's text with markup removed.
///
/// The page prints each temperature twice, in Celsius and Fahrenheit;
/// only the Celsius line is read. `&deg;` and the sign itself both
/// count as the degree sign.
pub fn parse_load(html: &str) -> ConsoleLoad {
    let text = console::strip_markup(html).replace("&deg;", &DEGREE.to_string());
    let first_word = |label: &str, unit: &str| {
        after(&text, label).find_map(|rest| {
            rest.split_whitespace()
                .next()?
                .strip_suffix(unit)?
                .parse()
                .ok()
        })
    };
    let celsius = format!("{DEGREE}C");
    let hdd_free_bytes = after(&text, "HDD:").find_map(|rest| {
        let mut words = rest.split_whitespace();
        bytes(words.next()?, words.next()?)
    });
    ConsoleLoad {
        cpu_c: first_word("CPU:", &celsius),
        rsx_c: first_word("RSX:", &celsius),
        fan_percent: first_word("FAN SPEED:", "%"),
        hdd_free_bytes,
    }
}

/// What follows `label` on each line of `text` that opens with it.
fn after<'t>(text: &'t str, label: &'t str) -> impl Iterator<Item = &'t str> {
    text.lines()
        .filter_map(move |line| line.trim().strip_prefix(label))
        .map(str::trim)
}

/// `amount` of `unit` (`KB`, `MB`, `GB` or `TB`, as binary multiples) in
/// bytes; `amount` may carry a decimal fraction.
fn bytes(amount: &str, unit: &str) -> Option<u64> {
    let scale: u64 = match unit {
        "KB" => 1 << 10,
        "MB" => 1 << 20,
        "GB" => 1 << 30,
        "TB" => 1 << 40,
        _ => return None,
    };
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    let whole: u64 = whole.parse().ok()?;
    let part = if fraction.is_empty() {
        0
    } else {
        let digits: u64 = fraction.parse().ok()?;
        let denominator = 10u64.checked_pow(u32::try_from(fraction.len()).ok()?)?;
        digits.checked_mul(scale)? / denominator
    };
    whole.checked_mul(scale)?.checked_add(part)
}

impl ConsoleLoad {
    /// The temperatures and fan, or the first temperature the page does
    /// not state.
    ///
    /// # Errors
    ///
    /// [`LoadError::Unstated`] naming the missing temperature.
    pub fn reading(&self) -> Result<LoadReading, LoadError> {
        Ok(LoadReading {
            cpu_c: self.cpu_c.ok_or(LoadError::Unstated("CPU temperature"))?,
            rsx_c: self.rsx_c.ok_or(LoadError::Unstated("RSX temperature"))?,
            fan_percent: self.fan_percent,
        })
    }

    /// The report line: each reading, or `not stated`.
    pub fn line(&self) -> String {
        let or_unstated = |value: Option<String>| value.unwrap_or_else(|| "not stated".to_string());
        format!(
            "load: Cell {}, RSX {}, fan {}, /dev_hdd0 {} free",
            or_unstated(self.cpu_c.map(|c| format!("{c} C"))),
            or_unstated(self.rsx_c.map(|c| format!("{c} C"))),
            or_unstated(self.fan_percent.map(|p| format!("{p}%"))),
            or_unstated(self.hdd_free_bytes.map(|b| format!("{} MiB", b >> 20))),
        )
    }
}

/// The state of `reading` on `host`, from the marker in `marker_dir`,
/// and the marker brought up to date: written when the console is hot,
/// removed once it is cool.
///
/// # Errors
///
/// [`RunnerPs3Error::LocalWrite`] when the runner cannot write or remove
/// the marker.
pub fn assess(
    reading: LoadReading,
    limits: &LoadLimits,
    marker_dir: &Path,
    host: &str,
) -> Result<Thermal, RunnerPs3Error> {
    let marker = hot_path(marker_dir, host);
    let thermal = Thermal::of(reading.cpu_c.max(reading.rsx_c), limits, marker.exists());
    let written = match thermal {
        Thermal::Hot => std::fs::write(
            &marker,
            format!("cpu_c={}\nrsx_c={}\n", reading.cpu_c, reading.rsx_c),
        ),
        Thermal::Cooling => Ok(()),
        Thermal::Cool => match std::fs::remove_file(&marker) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    };
    written.map_err(|source| RunnerPs3Error::LocalWrite {
        path: marker,
        source,
    })?;
    Ok(thermal)
}

/// How one verb is held to the interlock.
#[derive(Debug, Clone)]
pub struct Interlock<'a> {
    /// The limits, from the profiles file.
    pub limits: LoadLimits,
    /// The directory the marker lives in.
    pub marker_dir: &'a Path,
    /// The console.
    pub host: &'a str,
    /// `--wait-cool`: wait for a hot console instead of refusing.
    pub wait_cool: bool,
    /// Whether the verb deploys, so `/dev_hdd0` must hold the floor.
    pub needs_space: bool,
    /// The `status` command a refusal names.
    pub poll_with: String,
}

/// The status page, as text.
///
/// # Errors
///
/// [`RunnerPs3Error::Transport`] when the page does not answer.
pub fn status_page<C: ConsoleOps>(
    console: &mut C,
    transcript: &mut Transcript,
) -> Result<String, RunnerPs3Error> {
    let page = console.fetch(STATUS_PATH, transcript)?.ok_or_else(|| {
        TransportError::UnexpectedStatus {
            path: STATUS_PATH.to_string(),
            status: 404,
        }
    })?;
    Ok(String::from_utf8_lossy(&page).into_owned())
}

/// Admit a verb onto the console whose status page is `html`: the
/// reading it starts from, once the console is cool and, for a deploy,
/// has the space. Under `--wait-cool` a hot console is read again every
/// `wait_poll_s` until it is cool or `wait_limit_s` runs out.
///
/// # Errors
///
/// [`LoadError::Hot`] for a hot console without `--wait-cool`,
/// [`LoadError::StillHot`] when the wait runs out,
/// [`LoadError::Full`] for a deploy without the space, and
/// [`LoadError::Unstated`] for a page that does not state a reading.
pub fn admit<C: ConsoleOps>(
    console: &mut C,
    html: &str,
    interlock: &Interlock<'_>,
    sleep: &mut dyn FnMut(Duration),
    transcript: &mut Transcript,
) -> Result<LoadReading, RunnerPs3Error> {
    let limits = interlock.limits;
    let mut load = parse_load(html);
    let mut waited_s = 0;
    let reading = loop {
        let reading = load.reading()?;
        let thermal = assess(reading, &limits, interlock.marker_dir, interlock.host)?;
        transcript.decision(format!("{}; {thermal}", load.line()));
        if thermal == Thermal::Cool {
            break reading;
        }
        if !interlock.wait_cool {
            return Err(LoadError::Hot {
                thermal,
                reading,
                limits,
                poll_with: interlock.poll_with.clone(),
            }
            .into());
        }
        if waited_s >= limits.wait_limit_s {
            return Err(LoadError::StillHot {
                thermal,
                reading,
                limits,
                waited_s,
            }
            .into());
        }
        sleep(Duration::from_secs(limits.wait_poll_s));
        waited_s = waited_s.saturating_add(limits.wait_poll_s);
        load = parse_load(&status_page(console, transcript)?);
    };
    if interlock.needs_space {
        let free_bytes = load
            .hdd_free_bytes
            .ok_or(LoadError::Unstated("free space on /dev_hdd0"))?;
        let required_bytes = limits.hdd_floor_mib.saturating_mul(1 << 20);
        if free_bytes < required_bytes {
            return Err(LoadError::Full {
                free_bytes,
                required_bytes,
            }
            .into());
        }
    }
    Ok(reading)
}

#[cfg(test)]
#[path = "tests/load_tests.rs"]
mod tests;
