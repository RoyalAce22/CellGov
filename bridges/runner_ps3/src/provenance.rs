//! The capture's provenance record: the console identity, the hashes of
//! what the runner deployed and fetched, and the one wall-clock read
//! the runner makes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cellgov_compare::hardware_capture::{
    ArtifactHashes, CaptureProvenance, ConsoleFacts, FrameFacts, HarnessFacts, MicrotestFacts,
    TransportFacts, CAPTURE_PROVENANCE_SCHEMA, FRAME_FILE,
};
use sha2::{Digest, Sha256};

use crate::error::RunnerPs3Error;

/// The transport every capture this runner takes goes through.
pub const TRANSPORT_KIND: &str = "webman-filedrop";

/// What a capture's provenance is built from.
#[derive(Debug, Clone)]
pub struct CaptureInputs {
    /// The microtest name.
    pub name: String,
    /// The manifest file.
    pub manifest: PathBuf,
    /// Each source file, by its path relative to the microtest.
    pub sources: Vec<(String, PathBuf)>,
    /// `build/ps3/EBOOT.BIN`.
    pub eboot: PathBuf,
    /// `build/ps3/<name>.elf`.
    pub ps3_elf: PathBuf,
    /// `build/<name>.elf`, the reference the emulators run.
    pub reference_elf: PathBuf,
    /// `build/ps3/spu_main.elf`, when the test has one.
    pub spu_elf: Option<PathBuf>,
    /// `build/ps3/PARAM.SFO`.
    pub param_sfo: PathBuf,
    /// Where the console wrote the frame.
    pub result_path: String,
    /// The reconciled console facts.
    pub console: ConsoleFacts,
    /// The revision the runner was built from.
    pub harness_revision: String,
    /// Why this capture replaces an earlier one, when it does.
    pub recapture_reason: Option<String>,
}

/// The lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn hash_file(path: &Path) -> Result<String, RunnerPs3Error> {
    std::fs::read(path)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|source| RunnerPs3Error::LocalIo {
            path: path.to_path_buf(),
            source,
        })
}

/// The record for `frame`, taken at `captured_at`.
///
/// # Errors
///
/// [`RunnerPs3Error::LocalIo`] naming the first input file that cannot
/// be read.
pub fn build(
    inputs: &CaptureInputs,
    frame: &[u8],
    captured_at: String,
) -> Result<CaptureProvenance, RunnerPs3Error> {
    let frame_sha256 = sha256_hex(frame);
    let bytes = frame.len() as u64;
    let mut sources = BTreeMap::new();
    for (relative, path) in &inputs.sources {
        sources.insert(relative.clone(), hash_file(path)?);
    }
    let spu_elf_sha256 = match &inputs.spu_elf {
        Some(path) => Some(hash_file(path)?),
        None => None,
    };
    Ok(CaptureProvenance {
        schema: CAPTURE_PROVENANCE_SCHEMA,
        capture_id: CaptureProvenance::capture_id(&inputs.name, &frame_sha256),
        captured_at,
        console: inputs.console.clone(),
        transport: TransportFacts {
            kind: TRANSPORT_KIND.to_string(),
        },
        harness: HarnessFacts {
            runner: env!("CARGO_PKG_NAME").to_string(),
            revision: inputs.harness_revision.clone(),
            link: format!(
                "{}/commit/{}",
                env!("CARGO_PKG_REPOSITORY"),
                inputs.harness_revision
            ),
        },
        microtest: MicrotestFacts {
            name: inputs.name.clone(),
            manifest_sha256: hash_file(&inputs.manifest)?,
            sources,
        },
        artifacts: ArtifactHashes {
            eboot_sha256: hash_file(&inputs.eboot)?,
            ps3_elf_sha256: hash_file(&inputs.ps3_elf)?,
            reference_elf_sha256: hash_file(&inputs.reference_elf)?,
            spu_elf_sha256,
            param_sfo_sha256: hash_file(&inputs.param_sfo)?,
        },
        frame: FrameFacts {
            file: FRAME_FILE.to_string(),
            sha256: frame_sha256.clone(),
            bytes,
            result_path: inputs.result_path.clone(),
        },
        digest: format!("{frame_sha256}  micro:{}  {bytes}", inputs.name),
        recapture_reason: inputs.recapture_reason.clone(),
    })
}

/// The wall clock now, as RFC 3339 UTC to the second. The runner's only
/// clock read; nothing orders or waits on it.
pub fn now_rfc3339() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    rfc3339(seconds)
}

/// `seconds` after the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339(seconds: u64) -> String {
    let days = seconds / 86_400;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01, by the
/// era-of-400-years method.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
#[path = "tests/provenance_tests.rs"]
mod tests;
