//! The identity triple a run is composed from, and the cross-triple
//! check every comparison runs before it reports agreement.
//!
//! A divergence between two runs composed from different triples says
//! nothing about a regression. Every machine artifact a boot writes
//! carries the triple, and every comparator reports a mismatch between
//! the two sides.

use std::fmt;

use serde::{Deserialize, Serialize};

use cellgov_mem::Fnv1aHasher;
use cellgov_trace::{TraceReader, TraceRecord};

/// Prefix of the one-line, machine-readable form a boot prints on
/// stderr so a parent process can recover the triple its child ran.
pub const RUN_IDENTITY_SENTINEL: &str = "RUN_IDENTITY";

/// Which firmware answered the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirmwareIdentity {
    /// Console-visible version, and the key the store files the entry
    /// under (`"4.91"`).
    pub version: String,
    /// PUP-header `image_version` as zero-padded hex.
    pub image_version: String,
    /// SHA-256 over the PUP the entry was installed from.
    pub pup_sha256: String,
}

/// The version a title tree's PARAM.SFO names, under the key it came
/// from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppVersion {
    /// PARAM.SFO `APP_VER`.
    AppVer(String),
    /// PARAM.SFO `VERSION`, which names the version when the table has
    /// no `APP_VER`.
    SfoVersion(String),
}

impl AppVersion {
    /// The key the version came from, as the wire form spells it.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            Self::AppVer(_) => "app_ver",
            Self::SfoVersion(_) => "sfo_version",
        }
    }

    /// The version string.
    #[must_use]
    pub fn value(&self) -> &str {
        match self {
            Self::AppVer(v) | Self::SfoVersion(v) => v,
        }
    }
}

impl fmt::Display for AppVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.key(), self.value())
    }
}

/// Which of a title's installed versions the run composed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "GameIdentityWire", into = "GameIdentityWire")]
pub struct GameIdentity {
    /// The store key, and the guest directory name the title mounts
    /// under.
    pub title_id: String,
    /// The selected version: `"base"`, or `"update:<ver>"`.
    pub version: String,
    /// The version the executable's tree names in its PARAM.SFO.
    /// `None` when the table names none.
    pub app_version: Option<AppVersion>,
}

impl GameIdentity {
    /// The version as a report prints it: the key and the value, or a
    /// note that the tree named none.
    #[must_use]
    pub fn app_version_label(&self) -> String {
        self.app_version
            .as_ref()
            .map_or_else(|| "no version key".to_string(), ToString::to_string)
    }
}

/// The wire shape of [`GameIdentity`], with the version under the key
/// its tree named it by.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GameIdentityWire {
    title_id: String,
    version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_ver: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sfo_version: Option<String>,
}

impl From<GameIdentity> for GameIdentityWire {
    fn from(id: GameIdentity) -> Self {
        let (app_ver, sfo_version) = match id.app_version {
            Some(AppVersion::AppVer(v)) => (Some(v), None),
            Some(AppVersion::SfoVersion(v)) => (None, Some(v)),
            None => (None, None),
        };
        Self {
            title_id: id.title_id,
            version: id.version,
            app_ver,
            sfo_version,
        }
    }
}

impl TryFrom<GameIdentityWire> for GameIdentity {
    type Error = TwoVersionKeys;

    fn try_from(wire: GameIdentityWire) -> Result<Self, Self::Error> {
        let app_version = match (wire.app_ver, wire.sfo_version) {
            (Some(_), Some(_)) => return Err(TwoVersionKeys),
            (Some(v), None) => Some(AppVersion::AppVer(v)),
            (None, Some(v)) => Some(AppVersion::SfoVersion(v)),
            (None, None) => None,
        };
        Ok(Self {
            title_id: wire.title_id,
            version: wire.version,
            app_version,
        })
    }
}

/// A game identity that names its version under both keys.
#[derive(Debug, thiserror::Error)]
#[error("game identity names both app_ver and sfo_version; a tree's version comes from one key")]
pub struct TwoVersionKeys;

/// The triple every machine artifact a boot writes embeds.
///
/// A half is absent when the run has no store entry to name it:
///
/// - a boot against `--firmware-dir` names no firmware version;
/// - a title that ships inside the firmware has no version axis.
///
/// An artifact written before the store carried versions names
/// neither, so absence never reads as a mismatch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunIdentity {
    /// `None` for a run with no managed firmware.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<FirmwareIdentity>,
    /// `None` for a title with no store entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<GameIdentity>,
}

impl RunIdentity {
    /// True when neither half is named, which is how an artifact
    /// written before versioning reads.
    pub fn is_empty(&self) -> bool {
        self.firmware.is_none() && self.game.is_none()
    }

    /// Fingerprint of the firmware half; 0 when it is absent.
    pub fn firmware_fingerprint(&self) -> u64 {
        self.firmware.as_ref().map_or(0, |f| {
            fingerprint(&[&f.version, &f.image_version, &f.pup_sha256])
        })
    }

    /// Fingerprint of the game half; 0 when it is absent.
    pub fn game_fingerprint(&self) -> u64 {
        self.game.as_ref().map_or(0, |g| {
            let (key, value) = g
                .app_version
                .as_ref()
                .map_or(("", ""), |v| (v.key(), v.value()));
            fingerprint(&[&g.title_id, &g.version, key, value])
        })
    }

    /// The trace header record for this identity.
    pub fn trace_header(&self) -> TraceRecord {
        TraceRecord::RunIdentity {
            format_version: cellgov_trace::TRACE_FORMAT_VERSION,
            firmware: self.firmware_fingerprint(),
            game: self.game_fingerprint(),
        }
    }

    /// One line per half, in the order a report prints them.
    pub fn render_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        match &self.game {
            Some(g) => out.push(format!(
                "game     {} {}  ({})",
                g.title_id,
                g.version,
                g.app_version_label()
            )),
            None => out.push("game     (unidentified)".to_string()),
        }
        match &self.firmware {
            Some(f) => out.push(format!(
                "firmware {}  (image {}, pup sha256 {})",
                f.version, f.image_version, f.pup_sha256
            )),
            None => out.push("firmware (unidentified)".to_string()),
        }
        out
    }

    /// The `RUN_IDENTITY {json}` line a boot prints on stderr.
    ///
    /// # Errors
    ///
    /// Returns the serialization failure.
    pub fn render_sentinel_line(&self) -> Result<String, serde_json::Error> {
        Ok(format!(
            "{RUN_IDENTITY_SENTINEL} {}",
            serde_json::to_string(self)?
        ))
    }

    /// Recover the identity a child process printed.
    ///
    /// Returns `Ok(None)` when `text` holds no sentinel line, which is
    /// how a run that predates the header reads. A line carries the
    /// identity only when a space follows the sentinel.
    ///
    /// # Errors
    ///
    /// [`SentinelParseError`] when the text holds more than one
    /// sentinel line, or when the one it holds does not parse.
    pub fn parse_sentinel_lines(text: &str) -> Result<Option<Self>, SentinelParseError> {
        let mut found: Option<Self> = None;
        for line in text.lines() {
            let Some(payload) = line
                .trim_end()
                .strip_prefix(RUN_IDENTITY_SENTINEL)
                .and_then(|rest| rest.strip_prefix(' '))
            else {
                continue;
            };
            if found.is_some() {
                return Err(SentinelParseError::Repeated);
            }
            found = Some(serde_json::from_str(payload).map_err(|source| {
                SentinelParseError::Malformed {
                    line: line.to_string(),
                    source,
                }
            })?);
        }
        Ok(found)
    }
}

/// Why a `RUN_IDENTITY` line could not be read back.
#[derive(Debug, thiserror::Error)]
pub enum SentinelParseError {
    /// A boot prints the line once; two mean two runs' output was
    /// concatenated, and neither can be attributed.
    #[error("more than one {RUN_IDENTITY_SENTINEL} line")]
    Repeated,
    /// The payload after the sentinel is not a [`RunIdentity`].
    #[error("malformed {RUN_IDENTITY_SENTINEL} line {line:?}: {source}")]
    Malformed {
        /// The line as printed.
        line: String,
        /// Why the payload was refused.
        #[source]
        source: serde_json::Error,
    },
}

/// FNV-1a over the fields, each length-prefixed so `("a", "bc")` and
/// `("ab", "c")` do not collide.
///
/// The length goes in little-endian, the byte order [`Fnv1aHasher`]
/// requires for a multi-byte value.
fn fingerprint(fields: &[&str]) -> u64 {
    let mut h = Fnv1aHasher::new();
    for field in fields {
        h.write(&(field.len() as u64).to_le_bytes());
        h.write(field.as_bytes());
    }
    h.finish()
}

/// The header a trace stream leads with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceIdentity {
    /// Format version the stream was written in.
    pub format_version: u32,
    /// Fingerprint of the run's firmware half; 0 when it was absent.
    pub firmware: u64,
    /// Fingerprint of the run's game half; 0 when it was absent.
    pub game: u64,
}

impl TraceIdentity {
    /// The stream-level spelling of [`RunIdentity::is_empty`].
    fn names_nothing(self) -> bool {
        self.firmware == 0 && self.game == 0
    }
}

/// The identity header of a binary trace, or `None` for a stream that
/// does not lead with one.
///
/// A stream whose first record is anything else is format 1 and names
/// no identity. A first record that does not decode also reads as
/// `None`: the caller that hands the same bytes to a comparator
/// reports the decode failure.
pub fn trace_identity(bytes: &[u8]) -> Option<TraceIdentity> {
    match TraceReader::new(bytes).next()? {
        Ok(TraceRecord::RunIdentity {
            format_version,
            firmware,
            game,
        }) => Some(TraceIdentity {
            format_version,
            firmware,
            game,
        }),
        Ok(_) | Err(_) => None,
    }
}

/// The lines a trace comparison prints when its two streams lead with
/// different identity headers, empty when they agree.
///
/// A state file carries the triple as a fingerprint, so these lines
/// report that the two runs were composed differently without naming
/// how; the JSON artifacts of the same runs carry the names.
///
/// A stream makes no claim when it has no header, or when its header
/// names neither half. Such a stream never warns, which matches
/// [`cross_identity_warning`] on the same pair of runs.
pub fn cross_trace_identity_warning(
    a: Option<TraceIdentity>,
    a_label: &str,
    b: Option<TraceIdentity>,
    b_label: &str,
) -> Vec<String> {
    let (Some(a_id), Some(b_id)) = (a, b) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // The version says how to read the rest of the stream, so a
    // disagreement is reported even when neither side names a triple.
    if a_id.format_version != b_id.format_version {
        out.push(format!(
            "WARN: cross-format comparison: {a_label} is trace format {}, {b_label} is {}. \
             The two streams were written under different record contracts.",
            a_id.format_version, b_id.format_version,
        ));
    }
    // Every run leads its stream with a header, so an unidentified run
    // leads with an all-zero one.
    if a_id.names_nothing() || b_id.names_nothing() {
        return out;
    }
    let mut differs = Vec::new();
    if a_id.firmware != b_id.firmware {
        differs.push("firmware");
    }
    if a_id.game != b_id.game {
        differs.push("game version");
    }
    if differs.is_empty() {
        return out;
    }
    out.push(format!(
        "WARN: cross-triple comparison: {a_label} and {b_label} disagree on {}. \
         A divergence between two differently-composed runs is a difference between \
         versions until it is shown otherwise; it is not a regression.",
        differs.join(" and "),
    ));
    out.push(format!(
        "  {a_label}: firmware=0x{:016x} game=0x{:016x}",
        a_id.firmware, a_id.game
    ));
    out.push(format!(
        "  {b_label}: firmware=0x{:016x} game=0x{:016x}",
        b_id.firmware, b_id.game
    ));
    out
}

/// Both sides' triples, then [`cross_identity_warning`]'s lines.
///
/// Empty when neither side is identified, so a comparison of two
/// synthetic scenarios prints nothing.
pub fn identity_report(
    a: &RunIdentity,
    a_label: &str,
    b: &RunIdentity,
    b_label: &str,
) -> Vec<String> {
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (id, label) in [(a, a_label), (b, b_label)] {
        out.push(format!("{label}:"));
        out.extend(id.render_lines().into_iter().map(|l| format!("  {l}")));
    }
    out.extend(cross_identity_warning(a, a_label, b, b_label));
    out
}

/// The lines a comparison prints when its two sides were not composed
/// from the same triple, empty when they were.
///
/// An unidentified side never produces one: a pre-versioning artifact
/// makes no claim to contradict.
pub fn cross_identity_warning(
    a: &RunIdentity,
    a_label: &str,
    b: &RunIdentity,
    b_label: &str,
) -> Vec<String> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if a.firmware != b.firmware {
        out.push(format!(
            "WARN: cross-firmware comparison: {a_label} ran {}, {b_label} ran {}",
            describe_firmware(a),
            describe_firmware(b),
        ));
    }
    if a.game != b.game {
        out.push(format!(
            "WARN: cross-version comparison: {a_label} ran {}, {b_label} ran {}",
            describe_game(a),
            describe_game(b),
        ));
    }
    if !out.is_empty() {
        out.push(
            "WARN: a divergence between two differently-composed runs is a difference \
             between versions until it is shown otherwise; it is not a regression."
                .to_string(),
        );
    }
    out
}

fn describe_firmware(id: &RunIdentity) -> String {
    id.firmware.as_ref().map_or_else(
        || "no managed firmware".to_string(),
        |f| format!("firmware {} (image {})", f.version, f.image_version),
    )
}

fn describe_game(id: &RunIdentity) -> String {
    id.game.as_ref().map_or_else(
        || "no store entry".to_string(),
        |g| format!("{} {} ({})", g.title_id, g.version, g.app_version_label()),
    )
}

#[cfg(test)]
#[path = "tests/identity_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/app_version_tests.rs"]
mod app_version_tests;
