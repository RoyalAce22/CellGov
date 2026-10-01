//! A microtest capture taken on a PS3 and committed beside the test.
//!
//! `tests/micro/<name>/ps3/` holds four files the console runner wrote:
//! `observation.json` (an [`Observation`] whose runner is
//! [`RUNNER_PS3_CEX`]), `cgov_frame.bin` (the bytes fetched from the
//! console), `provenance.json` (a [`CaptureProvenance`]) and
//! `transcript.log`. Where the directory exists the capture is the
//! reference for that test; where it does not, the emulator
//! observations stand alone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::observation::Observation;

/// The runner string a console capture carries in its observation.
///
/// The compare driver checks state hashes only between observations of
/// one runner string, and a console capture carries no hashes, so the
/// string must differ from every emulator's.
pub const RUNNER_PS3_CEX: &str = "ps3-cex";

/// The one `provenance.json` layout this crate reads.
pub const CAPTURE_PROVENANCE_SCHEMA: u32 = 1;

/// The directory a capture lives in, under the microtest's own.
pub const CAPTURE_DIR: &str = "ps3";
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
    /// SHA-256 of each source file, by path relative to the test.
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
    /// The SPU image deployed beside the EBOOT, when the test has one.
    #[serde(default)]
    pub spu_elf_sha256: Option<String>,
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
    /// capture id derived from the test name and the frame hash, and a
    /// frame file named [`FRAME_FILE`].
    pub fn check(&self) -> Result<(), HardwareCaptureError> {
        if self.schema != CAPTURE_PROVENANCE_SCHEMA {
            return Err(HardwareCaptureError::Schema {
                found: self.schema,
                expected: CAPTURE_PROVENANCE_SCHEMA,
            });
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

/// Load the capture in `dir` (a `ps3/` directory) and check it against
/// itself: provenance schema and id, frame length, observation runner.
pub fn load(dir: &Path) -> Result<HardwareCapture, HardwareCaptureError> {
    let provenance: CaptureProvenance = read_json(&dir.join(PROVENANCE_FILE))?;
    provenance.check()?;
    let frame = read(&dir.join(FRAME_FILE))?;
    let found = frame.len() as u64;
    if found != provenance.frame.bytes {
        return Err(HardwareCaptureError::FrameLength {
            found,
            expected: provenance.frame.bytes,
        });
    }
    let observation: Observation = read_json(&dir.join(OBSERVATION_FILE))?;
    if observation.metadata.runner != RUNNER_PS3_CEX {
        return Err(HardwareCaptureError::Runner {
            found: observation.metadata.runner.clone(),
        });
    }
    Ok(HardwareCapture {
        dir: dir.to_path_buf(),
        observation,
        frame,
        provenance,
    })
}

/// The reference for the microtest in `test_dir`: its `ps3/` capture
/// when the directory exists, the emulator observations otherwise. A
/// capture directory that exists but does not load is an error, never
/// [`MicrotestReference::EmulatorOnly`].
pub fn microtest_reference(test_dir: &Path) -> Result<MicrotestReference, HardwareCaptureError> {
    let dir = test_dir.join(CAPTURE_DIR);
    if !dir.is_dir() {
        return Ok(MicrotestReference::EmulatorOnly);
    }
    load(&dir).map(|capture| MicrotestReference::Hardware(Box::new(capture)))
}

#[cfg(test)]
#[path = "tests/hardware_capture_tests.rs"]
mod tests;
