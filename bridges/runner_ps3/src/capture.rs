//! From the fetched frame to the committed capture: the CGOV frame
//! parsed against the manifest's regions into an observation whose
//! runner is the console's, the capture loop that produces the four
//! committed files, and the replay that reproduces a committed
//! observation from its frame.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cellgov_observation::console_profile::{ConsoleProfileError, ConsoleProfiles};
use cellgov_observation::frame::{parse_frame, FrameRegion, FRAME_MAGIC};
use cellgov_observation::hardware_capture::{
    self, CaptureProvenance, ConsoleFacts, FRAME_FILE, OBSERVATION_FILE, PROVENANCE_FILE,
    RUNNER_PS3_CEX,
};
use cellgov_observation::identity::RunIdentity;
use cellgov_observation::manifest::{self, ConsoleManifest};
use cellgov_observation::observation::{Observation, ObservationMetadata, ObservedOutcome};

use crate::deploy::{self, Package};
use crate::error::RunnerPs3Error;
use crate::provenance::{self, CaptureInputs};
use crate::run::{self, ConsoleOps, Target};
use crate::transcript::Transcript;

/// The CGOV frame header: the magic and the big-endian payload length.
const FRAME_HEADER: usize = 8;

/// Refuse anything but one whole CGOV frame: the magic, a big-endian
/// payload length, and exactly that many payload bytes.
///
/// # Errors
///
/// [`RunnerPs3Error::Frame`] naming what is wrong.
pub fn check_frame(bytes: &[u8]) -> Result<(), RunnerPs3Error> {
    let Some((header, payload)) = bytes.split_at_checked(FRAME_HEADER) else {
        return Err(RunnerPs3Error::Frame(format!(
            "{} bytes is shorter than the {FRAME_HEADER}-byte header",
            bytes.len()
        )));
    };
    if &header[..4] != FRAME_MAGIC.as_slice() {
        return Err(RunnerPs3Error::Frame(
            "the bytes do not open with the CGOV magic".to_string(),
        ));
    }
    let stated = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
    if payload.len() as u64 != u64::from(stated) {
        return Err(RunnerPs3Error::Frame(format!(
            "the header states a {stated}-byte payload but {} bytes follow it",
            payload.len()
        )));
    }
    Ok(())
}

/// The manifest's regions as positions in the frame payload: each
/// region sits at its [`MemoryRegionSpec::payload_offset`], and the
/// observation reports it at its `addr`, the guest address CellGov's
/// own run reads it from.
///
/// [`MemoryRegionSpec::payload_offset`]: cellgov_observation::manifest::MemoryRegionSpec::payload_offset
///
/// # Errors
///
/// [`RunnerPs3Error::Usage`] for a manifest with no region, a region
/// outside space 0, an empty region, or two regions of one name.
pub fn payload_regions(manifest: &ConsoleManifest) -> Result<Vec<FrameRegion>, RunnerPs3Error> {
    let regions = &manifest.observe.memory_regions;
    let bad = |what: String| RunnerPs3Error::Usage(format!("manifest [observe]: {what}"));
    if regions.is_empty() {
        return Err(bad(
            "declares no region, so the observation would match anything".to_string(),
        ));
    }
    let mut seen = BTreeSet::new();
    for region in regions {
        if region.space != 0 {
            return Err(bad(format!(
                "region {} names space {}; a console run is one process",
                region.name, region.space
            )));
        }
        if region.size == 0 {
            return Err(bad(format!("region {} declares zero bytes", region.name)));
        }
        if !seen.insert(region.name.as_str()) {
            return Err(bad(format!("region {} is declared twice", region.name)));
        }
    }
    Ok(regions
        .iter()
        .map(|region| FrameRegion {
            name: region.name.clone(),
            offset: region.payload_offset(),
            size: region.size,
            guest_addr: region.addr,
        })
        .collect())
}

/// The observation the frame at `frame_path` converts to under
/// `manifest`, from a console running `firmware`.
///
/// # Errors
///
/// [`RunnerPs3Error::LocalRead`] for an unreadable frame,
/// [`RunnerPs3Error::Frame`] for anything but one whole frame that
/// holds every region, and a [`payload_regions`] refusal.
pub fn frame_to_observation(
    frame_path: &Path,
    manifest: &ConsoleManifest,
    firmware: &str,
) -> Result<Observation, RunnerPs3Error> {
    let bytes = std::fs::read(frame_path).map_err(|source| RunnerPs3Error::LocalRead {
        path: frame_path.to_path_buf(),
        source,
    })?;
    check_frame(&bytes)?;
    let regions = payload_regions(manifest)?;
    let memory_regions =
        parse_frame(&bytes, &regions).map_err(|e| RunnerPs3Error::Frame(e.to_string()))?;
    Ok(Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions,
        events: Vec::new(),
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: RUNNER_PS3_CEX.to_string(),
            steps: None,
        },
        tty_log: Vec::new(),
        identity: RunIdentity::default(),
        runner_firmware: Some(firmware.to_string()),
    })
}

/// An observation as the committed file holds it.
///
/// # Errors
///
/// [`RunnerPs3Error::Serialize`] when JSON refuses a value.
pub fn observation_json(observation: &Observation) -> Result<String, RunnerPs3Error> {
    Ok(serde_json::to_string_pretty(observation)?)
}

/// The firmware of tracked profile `claimed`.
///
/// # Errors
///
/// [`RunnerPs3Error::Profile`] when the profiles file does not track it.
pub fn profile_firmware(
    profiles: &ConsoleProfiles,
    claimed: &str,
) -> Result<String, RunnerPs3Error> {
    profiles
        .profile
        .get(claimed)
        .map(|profile| profile.firmware.clone())
        .ok_or_else(|| {
            ConsoleProfileError::UnknownProfile {
                claimed: claimed.to_string(),
                known: profiles
                    .profile
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
            }
            .into()
        })
}

/// The offline `convert` verb: the observation the frame at `frame_path`
/// converts to under the manifest at `manifest_path` and the tracked
/// profile `claimed`.
///
/// # Errors
///
/// A manifest, profile or [`frame_to_observation`] error.
pub fn convert(
    frame_path: &Path,
    manifest_path: &Path,
    profiles: &ConsoleProfiles,
    claimed: &str,
) -> Result<Observation, RunnerPs3Error> {
    let manifest = manifest::load_console(manifest_path)?;
    frame_to_observation(frame_path, &manifest, &profile_firmware(profiles, claimed)?)
}

/// Reproduce the committed capture in `capture_dir` (a
/// `ps3/<profile>/` directory) under the manifest at `manifest_path`:
/// the capture loads with every check the reference lookup makes, names
/// this manifest's test, and its frame converts to its committed
/// `observation.json` byte for byte.
///
/// # Errors
///
/// [`RunnerPs3Error::Capture`] for a capture that does not load,
/// [`RunnerPs3Error::Frame`] for one whose observation does not
/// reproduce or that names another test, and a manifest or conversion
/// error.
pub fn replay(
    capture_dir: &Path,
    manifest_path: &Path,
    profiles: &ConsoleProfiles,
) -> Result<(), RunnerPs3Error> {
    let capture = hardware_capture::load(capture_dir, profiles)?;
    let manifest = manifest::load_console(manifest_path)?;
    if capture.provenance.microtest.name != manifest.test.name {
        return Err(RunnerPs3Error::Frame(format!(
            "{} records test {:?} but its manifest is {:?}",
            capture_dir.display(),
            capture.provenance.microtest.name,
            manifest.test.name
        )));
    }
    let observation = frame_to_observation(
        &capture_dir.join(FRAME_FILE),
        &manifest,
        &capture.provenance.console.firmware,
    )?;
    let committed_path = capture_dir.join(OBSERVATION_FILE);
    let committed = std::fs::read(&committed_path).map_err(|source| RunnerPs3Error::LocalRead {
        path: committed_path.clone(),
        source,
    })?;
    if observation_json(&observation)?.as_bytes() != committed.as_slice() {
        return Err(RunnerPs3Error::Frame(format!(
            "{} does not reproduce from its frame",
            committed_path.display()
        )));
    }
    Ok(())
}

/// One capture's settings, from the command line and the manifest.
#[derive(Debug, Clone)]
pub struct CapturePlan {
    /// The manifest file.
    pub manifest_path: PathBuf,
    /// The manifest, as the console runner reads it.
    pub manifest: ConsoleManifest,
    /// The directory the four files go to.
    pub out: PathBuf,
    /// The revision the runner was built from.
    pub harness_revision: String,
    /// The wait between polls for the result, in milliseconds.
    pub poll_ms: u64,
    /// Whether the run may remove an existing game directory of the appid.
    pub reclaim: bool,
    /// Whether to leave the package on the console after the run.
    pub keep_deployed: bool,
    /// `--recapture --reason`: why this capture replaces the one in
    /// `out`. `None` refuses an existing `out`.
    pub recapture_reason: Option<String>,
    /// The command that clears the console, for refusals.
    pub clear_with: String,
}

impl CapturePlan {
    /// The console paths this plan deploys to and reads from.
    pub fn target(&self) -> Target {
        Target::new(&self.manifest.ps3.appid, &self.manifest.result_file_name())
    }
}

/// The directory the runner converts a capture in before it reaches `out`:
/// `out` with `.staging` after its name, beside it.
fn staging_dir(out: &Path) -> PathBuf {
    let mut name = out.file_name().unwrap_or_default().to_os_string();
    name.push(".staging");
    out.with_file_name(name)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), RunnerPs3Error> {
    std::fs::write(path, bytes).map_err(|source| RunnerPs3Error::LocalWrite {
        path: path.to_path_buf(),
        source,
    })
}

/// Every file under `dir` except `build/` and the capture directories,
/// by path relative to `dir` with `/` separators, the manifest aside.
fn source_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, RunnerPs3Error> {
    let mut out = Vec::new();
    let mut pending = vec![(String::new(), dir.to_path_buf())];
    while let Some((prefix, here)) = pending.pop() {
        let entries = std::fs::read_dir(&here).map_err(|source| RunnerPs3Error::LocalRead {
            path: here.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| RunnerPs3Error::LocalRead {
                path: here.clone(),
                source,
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = format!("{prefix}{name}");
            let path = entry.path();
            if path.is_dir() {
                if !(prefix.is_empty()
                    && (name == "build" || name == hardware_capture::CAPTURE_DIR))
                {
                    pending.push((format!("{relative}/"), path));
                }
            } else if relative != "manifest.toml" {
                out.push((relative, path));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The shared build inputs every microtest build reads, `../common/`
/// beside the test directory, keyed by that relative path; none when
/// the directory is absent.
fn common_files(test_dir: &Path) -> Result<Vec<(String, PathBuf)>, RunnerPs3Error> {
    let common = test_dir.join("..").join("common");
    if !common.is_dir() {
        return Ok(Vec::new());
    }
    Ok(source_files(&common)?
        .into_iter()
        .map(|(relative, path)| (format!("../common/{relative}"), path))
        .collect())
}

/// The capture loop on a console whose identity is `facts`: preflight,
/// deploy, start, wait, fetch, then the frame, observation and
/// provenance written to `plan.out`, cleanup, and the transcript.
///
/// A failure before the frame is in hand still runs cleanup (unless the
/// plan keeps the package) and returns the failure. A cleanup failure
/// after the runner writes the files returns [`RunnerPs3Error::Cleanup`], and
/// the capture stays written.
///
/// # Errors
///
/// [`RunnerPs3Error::Refused`] for an existing `out` without a recapture
/// reason, and any error of the steps above. `may_reclaim` answers the
/// preflight's question under `plan.reclaim`.
pub fn capture<C: ConsoleOps>(
    console: &mut C,
    plan: &CapturePlan,
    facts: &ConsoleFacts,
    sleep: &mut dyn FnMut(Duration),
    now: &mut dyn FnMut() -> Result<String, RunnerPs3Error>,
    may_reclaim: &mut dyn FnMut(&run::Reclaim) -> Result<bool, RunnerPs3Error>,
    transcript: &mut Transcript,
) -> Result<CaptureProvenance, RunnerPs3Error> {
    if plan.out.exists() && plan.recapture_reason.is_none() {
        return Err(RunnerPs3Error::Refused {
            reason: format!("{} already holds a capture", plan.out.display()),
            clear_with: "rerun with --recapture --reason \"<why this replaces it>\"".to_string(),
        });
    }
    let target = plan.target();
    let package = Package::of(&plan.manifest_path, &plan.manifest);
    let inputs = CaptureInputs {
        name: plan.manifest.test.name.clone(),
        manifest: plan.manifest_path.clone(),
        sources: [
            source_files(&package.test_dir)?,
            common_files(&package.test_dir)?,
        ]
        .concat(),
        eboot: package.eboot.clone(),
        ps3_elf: package.ps3_elf.clone(),
        reference_elf: package.reference_elf.clone(),
        siblings: package.siblings.clone(),
        param_sfo: package.param_sfo.clone(),
        result_path: target.result_path.clone(),
        console: facts.clone(),
        harness_revision: plan.harness_revision.clone(),
        recapture_reason: plan.recapture_reason.clone(),
    };
    run::preflight(
        console,
        &target,
        plan.reclaim,
        may_reclaim,
        &plan.clear_with,
        transcript,
    )?;
    let timeout_ms = plan.manifest.ps3.timeout_ms;
    let fetched = deploy::deploy(console, &target, &package, transcript)
        .and_then(|()| run::start(console, &target, transcript))
        .and_then(|()| {
            run::wait_for_result(
                console,
                &target,
                timeout_ms,
                plan.poll_ms,
                sleep,
                transcript,
            )
        })
        .and_then(|()| run::fetch_result(console, &target, timeout_ms, transcript))
        .and_then(|frame| check_frame(&frame).map(|()| frame));
    // Everything that can still fail runs against a staging directory
    // beside `out`, so a conversion that fails leaves `out` (and a
    // capture a recapture would replace) untouched.
    let staging = staging_dir(&plan.out);
    let prepared = fetched.and_then(|frame| {
        let captured_at = now()?;
        std::fs::create_dir_all(&staging).map_err(|source| RunnerPs3Error::LocalWrite {
            path: staging.clone(),
            source,
        })?;
        let staged = staging.join(FRAME_FILE);
        write(&staged, &frame)?;
        let observation = frame_to_observation(&staged, &plan.manifest, &facts.firmware)?;
        let record = provenance::build(&inputs, &frame, captured_at)?;
        let record_json = serde_json::to_string_pretty(&record)?;
        Ok((frame, observation_json(&observation)?, record, record_json))
    });
    let (frame, observation, record, record_json) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            if staging.exists() {
                if let Err(e) = std::fs::remove_dir_all(&staging) {
                    transcript.decision(format!("{} remains: {e}", staging.display()));
                }
            }
            if !plan.keep_deployed {
                if let Err(cleanup) = run::cleanup(console, &target, transcript) {
                    transcript.decision(format!("after the failure, {cleanup}"));
                }
            }
            return Err(error);
        }
    };
    std::fs::create_dir_all(&plan.out).map_err(|source| RunnerPs3Error::LocalWrite {
        path: plan.out.clone(),
        source,
    })?;
    write(&plan.out.join(FRAME_FILE), &frame)?;
    write(&plan.out.join(OBSERVATION_FILE), observation.as_bytes())?;
    write(&plan.out.join(PROVENANCE_FILE), record_json.as_bytes())?;
    std::fs::remove_dir_all(&staging).map_err(|source| RunnerPs3Error::LocalWrite {
        path: staging.clone(),
        source,
    })?;
    transcript.decision(format!("captured {}", record.capture_id));
    let cleaned = if plan.keep_deployed {
        transcript.decision("the package stays deployed (--keep-deployed)");
        Ok(())
    } else {
        run::cleanup(console, &target, transcript)
    };
    transcript
        .write(&plan.out)
        .map_err(|source| RunnerPs3Error::LocalWrite {
            path: plan.out.join(hardware_capture::TRANSCRIPT_FILE),
            source,
        })?;
    cleaned.map(|()| record)
}

#[cfg(test)]
#[path = "tests/capture_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/capture_layout_tests.rs"]
mod capture_layout_tests;
