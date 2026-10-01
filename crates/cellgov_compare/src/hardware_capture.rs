//! A microtest capture taken on a PS3 and committed beside the test.
//!
//! `tests/micro/<name>/ps3/<profile>/` holds four files the console
//! runner wrote under the console profile the run claimed:
//! `observation.json` (an [`Observation`] whose runner is
//! [`RUNNER_PS3_CEX`]), `cgov_frame.bin` (the bytes fetched from the
//! console), `provenance.json` (a [`CaptureProvenance`]) and
//! `transcript.log`. The capture under the reference profile is the
//! reference for that test; where there is none, the emulator
//! observations stand alone. A capture under another profile is never
//! the reference.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::console_profile::{ConsoleProfileError, ConsoleProfiles};
use crate::observation::{Observation, ObservedOutcome};

/// The runner string a console capture carries in its observation.
///
/// The compare driver checks state hashes only between observations of
/// one runner string, and a console capture carries no hashes, so the
/// string must differ from every emulator's.
pub const RUNNER_PS3_CEX: &str = "ps3-cex";

/// The one `provenance.json` layout this crate reads.
pub const CAPTURE_PROVENANCE_SCHEMA: u32 = 1;

/// The directory the captures live in, under the microtest's own; each
/// sits in a subdirectory named for its console profile.
pub const CAPTURE_DIR: &str = "ps3";
/// The kernel a [`RUNNER_PS3_CEX`] capture runs on.
pub const CAPTURE_KERNEL: &str = "cex";
/// The observation the runner converted from the frame.
pub const OBSERVATION_FILE: &str = "observation.json";
/// The bytes fetched from the console, untouched.
pub const FRAME_FILE: &str = "cgov_frame.bin";
/// The provenance record.
pub const PROVENANCE_FILE: &str = "provenance.json";
/// The redacted exchange with the console.
pub const TRANSCRIPT_FILE: &str = "transcript.log";

/// How many leading hex digits of the frame hash the capture id carries.
const CAPTURE_ID_HASH_DIGITS: usize = 12;

/// Where, when and from what the runner took a capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureProvenance {
    /// [`CAPTURE_PROVENANCE_SCHEMA`].
    pub schema: u32,
    /// `micro:<name>#<12 hex of the frame hash>`.
    pub capture_id: String,
    /// The one wall-clock read the runner makes, as RFC 3339 text.
    pub captured_at: String,
    /// The console the frame came from.
    pub console: ConsoleFacts,
    /// How the runner reached the console.
    pub transport: TransportFacts,
    /// The runner build that took the capture.
    pub harness: HarnessFacts,
    /// The microtest as built.
    pub microtest: MicrotestFacts,
    /// Hashes of the files deployed to the console.
    pub artifacts: ArtifactHashes,
    /// The frame as fetched.
    pub frame: FrameFacts,
    /// One line in the shape of a parity digest row,
    /// `<sha256>  <key>  <bytes>`, so an archive row can cite the
    /// capture.
    pub digest: String,
    /// Why this capture replaced an earlier one, when it did.
    #[serde(default)]
    pub recapture_reason: Option<String>,
}

/// The public facts about the console; no identifier of the unit.
///
/// Every observed value is kept, hard and soft; which fields are hard is
/// the [`crate::console_profile`] module's to say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleFacts {
    /// The console profile the run claimed.
    pub profile: String,
    /// Model name, such as `CECH-2001A`.
    pub model: String,
    /// `cex` or `dex`.
    pub kernel: String,
    /// System software version, such as `4.93`.
    pub firmware: String,
    /// Custom firmware name and version.
    pub cfw: String,
    /// Cobra payload version.
    pub cobra: String,
    /// webMAN version, or `None` when the status page states none.
    #[serde(default)]
    pub webman: Option<String>,
    /// Whether a debugger held the console during the run.
    pub debugger_attached: bool,
}

/// How the runner moved files and started the test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportFacts {
    /// The transport kind; the runner writes `webman-filedrop`.
    pub kind: String,
}

/// The runner that took the capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessFacts {
    /// The runner's name.
    pub runner: String,
    /// The revision the runner was built from.
    pub revision: String,
    /// Where that revision can be read.
    pub link: String,
}

/// The microtest as built for the capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MicrotestFacts {
    /// The test name.
    pub name: String,
    /// SHA-256 of the manifest.
    pub manifest_sha256: String,
    /// SHA-256 of each source file, by path relative to the test; the
    /// shared build inputs sit under `../common/`.
    pub sources: BTreeMap<String, String>,
}

/// SHA-256 of each deployed or referenced file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactHashes {
    /// The packaged `EBOOT.BIN`.
    pub eboot_sha256: String,
    /// The console relink the EBOOT wraps.
    pub ps3_elf_sha256: String,
    /// The reference ELF the emulators run.
    pub reference_elf_sha256: String,
    /// Each file deployed beside the EBOOT (the manifest's `[ps3]
    /// files`), by name.
    pub siblings: BTreeMap<String, String>,
    /// The `PARAM.SFO`.
    pub param_sfo_sha256: String,
}

/// The frame as fetched from the console.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameFacts {
    /// The committed file name, [`FRAME_FILE`].
    pub file: String,
    /// SHA-256 of the bytes.
    pub sha256: String,
    /// Byte count.
    pub bytes: u64,
    /// Where the console wrote it.
    pub result_path: String,
}

impl CaptureProvenance {
    /// The capture id for a test and its frame hash.
    pub fn capture_id(name: &str, frame_sha256: &str) -> String {
        let digits: String = frame_sha256.chars().take(CAPTURE_ID_HASH_DIGITS).collect();
        format!("micro:{name}#{digits}")
    }

    /// The record's own consistency: the schema this crate reads, a
    /// frame hash of 64 lowercase hex digits, a capture id derived from
    /// the test name and that hash, and a frame file named
    /// [`FRAME_FILE`].
    pub fn check(&self) -> Result<(), HardwareCaptureError> {
        if self.schema != CAPTURE_PROVENANCE_SCHEMA {
            return Err(HardwareCaptureError::Schema {
                found: self.schema,
                expected: CAPTURE_PROVENANCE_SCHEMA,
            });
        }
        if !is_sha256_hex(&self.frame.sha256) {
            return Err(HardwareCaptureError::FrameHashShape(
                self.frame.sha256.clone(),
            ));
        }
        let expected = Self::capture_id(&self.microtest.name, &self.frame.sha256);
        if self.capture_id != expected {
            return Err(HardwareCaptureError::CaptureId {
                found: self.capture_id.clone(),
                expected,
            });
        }
        if self.frame.file != FRAME_FILE {
            return Err(HardwareCaptureError::FrameFile {
                found: self.frame.file.clone(),
            });
        }
        Ok(())
    }
}

/// One committed capture, loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareCapture {
    /// The directory it was read from.
    pub dir: PathBuf,
    /// The converted observation.
    pub observation: Observation,
    /// The bytes fetched from the console.
    pub frame: Vec<u8>,
    /// The provenance record.
    pub provenance: CaptureProvenance,
}

/// What a microtest directory holds as its reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicrotestReference {
    /// A console capture, which is the reference.
    Hardware(Box<HardwareCapture>),
    /// No capture directory; the emulator observations stand alone.
    EmulatorOnly,
}

/// Why a capture directory did not load.
#[derive(Debug, thiserror::Error)]
pub enum HardwareCaptureError {
    /// A file of the capture cannot be read.
    #[error("capture file {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A JSON file of the capture does not parse.
    #[error("capture file {path}: {source}")]
    Json {
        /// The file.
        path: PathBuf,
        /// The parse error.
        #[source]
        source: serde_json::Error,
    },
    /// The provenance names a schema this crate does not read.
    #[error("capture provenance schema {found}, this build reads {expected}")]
    Schema {
        /// The schema the file names.
        found: u32,
        /// The schema this crate reads.
        expected: u32,
    },
    /// The capture id does not derive from the test name and frame hash.
    #[error("capture id {found:?} does not match the test and frame, expected {expected:?}")]
    CaptureId {
        /// The id as written.
        found: String,
        /// The id the name and hash derive.
        expected: String,
    },
    /// The provenance names a frame file other than the committed one.
    #[error("capture frame file {found:?}, the committed name is {FRAME_FILE:?}")]
    FrameFile {
        /// The name as written.
        found: String,
    },
    /// The frame on disk is not the length the provenance records.
    #[error("capture frame holds {found} bytes, the provenance records {expected}")]
    FrameLength {
        /// Bytes on disk.
        found: u64,
        /// Bytes the provenance records.
        expected: u64,
    },
    /// The observation carries a runner string other than the console's.
    #[error("capture observation runner {found:?}, a console capture is {RUNNER_PS3_CEX:?}")]
    Runner {
        /// The runner as written.
        found: String,
    },
    /// The recorded frame hash is not 64 lowercase hex digits.
    #[error("capture frame hash {0:?} is not 64 lowercase hex digits")]
    FrameHashShape(String),
    /// The frame on disk does not hash to the recorded value.
    #[error("capture frame hashes to {found}, the provenance records {recorded}")]
    FrameHash {
        /// The hash of the bytes on disk.
        found: String,
        /// The hash the provenance records.
        recorded: String,
    },
    /// A path the capture layout needs as a directory is something else.
    #[error("capture path {0} is not a directory")]
    NotADirectory(PathBuf),
    /// The capture names a test other than the directory it sits under.
    #[error("capture names test {found:?} but sits under test {expected:?}")]
    TestName {
        /// The name the provenance records.
        found: String,
        /// The test directory's name.
        expected: String,
    },
    /// The capture sits under a profile directory other than its own.
    #[error("capture records profile {profile:?} but sits in directory {directory:?}")]
    ProfileDirectory {
        /// The directory's name.
        directory: String,
        /// The profile the provenance records.
        profile: String,
    },
    /// The recorded console fails its own profile, or the profile is not
    /// tracked.
    #[error("capture console: {0}")]
    Profile(#[from] ConsoleProfileError),
    /// The recorded kernel is not the one the runner string names.
    #[error(
        "capture console kernel {found:?}, a {RUNNER_PS3_CEX:?} capture runs on {CAPTURE_KERNEL:?}"
    )]
    Kernel {
        /// The kernel as recorded.
        found: String,
    },
    /// The observation's firmware is not the console's.
    #[error("capture observation firmware {observation:?}, the console ran {console:?}")]
    ObservationFirmware {
        /// The observation's `runner_firmware`.
        observation: Option<String>,
        /// The provenance's console firmware.
        console: String,
    },
    /// The observation carries a field a console capture never sets.
    #[error("capture observation carries {0}, which a console capture never sets")]
    ObservationField(&'static str),
    /// A file sits directly in the capture directory, where only
    /// profile directories belong: an old-layout capture, or a stray.
    #[error("capture path {0} is a file; a capture lives in a ps3/<profile>/ directory")]
    LooseFile(PathBuf),
}

fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn read(path: &Path) -> Result<Vec<u8>, HardwareCaptureError> {
    std::fs::read(path).map_err(|source| HardwareCaptureError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, HardwareCaptureError> {
    let bytes = read(path)?;
    serde_json::from_slice(&bytes).map_err(|source| HardwareCaptureError::Json {
        path: path.to_path_buf(),
        source,
    })
}

/// Load the capture in `dir` (a `ps3/<profile>/` directory) and check
/// it: the provenance record's own consistency; its profile against the
/// directory name and against the tracked profile's hard fields; the
/// kernel the runner string implies; the frame's length and hash; the
/// observation's runner, firmware and console-only shape; and the
/// transcript's presence.
pub fn load(
    dir: &Path,
    profiles: &ConsoleProfiles,
) -> Result<HardwareCapture, HardwareCaptureError> {
    let provenance: CaptureProvenance = read_json(&dir.join(PROVENANCE_FILE))?;
    provenance.check()?;
    let console = &provenance.console;
    let directory = dir.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if directory != console.profile {
        return Err(HardwareCaptureError::ProfileDirectory {
            directory: directory.to_string(),
            profile: console.profile.clone(),
        });
    }
    profiles.check(&console.profile, console)?;
    if console.kernel != CAPTURE_KERNEL {
        return Err(HardwareCaptureError::Kernel {
            found: console.kernel.clone(),
        });
    }
    let frame = read(&dir.join(FRAME_FILE))?;
    let found = frame.len() as u64;
    if found != provenance.frame.bytes {
        return Err(HardwareCaptureError::FrameLength {
            found,
            expected: provenance.frame.bytes,
        });
    }
    let hash = sha256_hex(&frame);
    if hash != provenance.frame.sha256 {
        return Err(HardwareCaptureError::FrameHash {
            found: hash,
            recorded: provenance.frame.sha256.clone(),
        });
    }
    let observation: Observation = read_json(&dir.join(OBSERVATION_FILE))?;
    check_observation(&observation, console)?;
    read(&dir.join(TRANSCRIPT_FILE))?;
    Ok(HardwareCapture {
        dir: dir.to_path_buf(),
        observation,
        frame,
        provenance,
    })
}

/// The observation a console capture converts to: the console's runner
/// string and firmware, a completed outcome, and no events, TTY log,
/// step count, state hashes or run identity.
fn check_observation(
    observation: &Observation,
    console: &ConsoleFacts,
) -> Result<(), HardwareCaptureError> {
    if observation.metadata.runner != RUNNER_PS3_CEX {
        return Err(HardwareCaptureError::Runner {
            found: observation.metadata.runner.clone(),
        });
    }
    if observation.runner_firmware.as_deref() != Some(console.firmware.as_str()) {
        return Err(HardwareCaptureError::ObservationFirmware {
            observation: observation.runner_firmware.clone(),
            console: console.firmware.clone(),
        });
    }
    if observation.state_hashes.is_some() {
        return Err(HardwareCaptureError::ObservationField("state hashes"));
    }
    if !observation.identity.is_empty() {
        return Err(HardwareCaptureError::ObservationField("a run identity"));
    }
    if observation.outcome != ObservedOutcome::Completed {
        return Err(HardwareCaptureError::ObservationField(
            "an outcome other than completed",
        ));
    }
    if !observation.events.is_empty() {
        return Err(HardwareCaptureError::ObservationField("events"));
    }
    if !observation.tty_log.is_empty() {
        return Err(HardwareCaptureError::ObservationField("a TTY log"));
    }
    if observation.metadata.steps.is_some() {
        return Err(HardwareCaptureError::ObservationField("a step count"));
    }
    Ok(())
}

/// Whether `path` is a directory: `Ok(false)` only when nothing is
/// there. A link whose target is gone is something there, not absence.
fn directory_present(path: &Path) -> Result<bool, HardwareCaptureError> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => Ok(true),
        Ok(_) => Err(HardwareCaptureError::NotADirectory(path.to_path_buf())),
        Err(source)
            if source.kind() == ErrorKind::NotFound && std::fs::symlink_metadata(path).is_err() =>
        {
            Ok(false)
        }
        Err(source) => Err(HardwareCaptureError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Every `ps3/<profile>/` directory under the microtest in `test_dir`,
/// sorted; none when `ps3/` is absent.
///
/// # Errors
///
/// [`HardwareCaptureError::LooseFile`] for a file directly in `ps3/`,
/// [`HardwareCaptureError::NotADirectory`] when `ps3/` is a file, and
/// [`HardwareCaptureError::Io`] when it cannot be read.
pub fn profile_directories(test_dir: &Path) -> Result<Vec<PathBuf>, HardwareCaptureError> {
    let root = test_dir.join(CAPTURE_DIR);
    if !directory_present(&root)? {
        return Ok(Vec::new());
    }
    let io = |source| HardwareCaptureError::Io {
        path: root.clone(),
        source,
    };
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(io)? {
        let path = entry.map_err(io)?.path();
        match directory_present(&path) {
            Ok(true) => dirs.push(path),
            Ok(false) | Err(HardwareCaptureError::NotADirectory(_)) => {
                return Err(HardwareCaptureError::LooseFile(path))
            }
            Err(other) => return Err(other),
        }
    }
    dirs.sort();
    Ok(dirs)
}

/// The reference for the microtest in `test_dir`: its capture under the
/// reference profile when that directory exists, the emulator
/// observations when nothing is there. A directory that exists but does
/// not load, or names another test, and a file directly in `ps3/`, are
/// errors, never [`MicrotestReference::EmulatorOnly`].
pub fn microtest_reference(
    test_dir: &Path,
    profiles: &ConsoleProfiles,
) -> Result<MicrotestReference, HardwareCaptureError> {
    let dir = test_dir.join(CAPTURE_DIR).join(&profiles.reference);
    if !profile_directories(test_dir)?.contains(&dir) {
        return Ok(MicrotestReference::EmulatorOnly);
    }
    let capture = load(&dir, profiles)?;
    let expected = test_dir
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    if capture.provenance.microtest.name != expected {
        return Err(HardwareCaptureError::TestName {
            found: capture.provenance.microtest.name.clone(),
            expected: expected.to_string(),
        });
    }
    Ok(MicrotestReference::Hardware(Box::new(capture)))
}

#[cfg(test)]
#[path = "tests/hardware_capture_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/capture_shape_tests.rs"]
mod capture_shape_tests;
