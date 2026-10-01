//! Boots every bootable microtest under `tests/micro/` and
//! checks the `CGOV` payload it reports against the values its design
//! fixes.
//!
//! Compiled only under the `microtests` feature: the ELFs are
//! gitignored build output, so opting in declares the microtest tree built and
//! a missing artifact is a hard error rather than a skip. Build each
//! with `tests/micro/<name>/build.sh` in a ps3dev+PSL1GHT toolchain
//! image -- requirements are in each script's header.
//!
//! Every expectation below is derived from the microtest's own
//! documented output layout (a sum is `N*(N-1)/2` for that test's
//! `MESSAGES`, a counter is `2 * INCREMENTS_PER_THREAD`), never copied
//! from a previous run's output. A table transcribed from observed
//! bytes would re-pass whatever the code currently does, which is the
//! failure this gate exists to catch.

#![allow(
    clippy::print_stderr,
    reason = "integration test harness: stderr carries per-case verdicts and diagnostics"
)]
#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use std::path::PathBuf;
use std::process::Command;

use cellgov_compare::{Observation, ObservedOutcome};

#[path = "microtests/cases.rs"]
mod cases;
use cases::{Case, Exact, NonZero, CASES};

/// Walk up from `CARGO_MANIFEST_DIR` to the `[workspace]` Cargo.toml.
fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if let Ok(text) = std::fs::read_to_string(p.join("Cargo.toml")) {
            if text.contains("[workspace]") {
                return p;
            }
        }
        assert!(
            p.pop(),
            "could not find workspace root walking up from {}",
            env!("CARGO_MANIFEST_DIR")
        );
    }
}

fn manifest_path(name: &str) -> PathBuf {
    workspace_root()
        .join("tests")
        .join("micro")
        .join(name)
        .join("manifest.toml")
}

/// Boot one microtest and return its observation.
///
/// `run_id` discriminates re-runs of one case so the determinism pass
/// cannot race the payload pass on the same scratch file.
fn run_observation(case: &Case, run_id: &str) -> Observation {
    let manifest = manifest_path(case.name);
    assert!(
        manifest.is_file(),
        "{}: manifest missing at {}",
        case.name,
        manifest.display()
    );

    let scratch = workspace_root()
        .join("target")
        .join("microtests_scratch")
        .join(std::process::id().to_string())
        .join(case.name)
        .join(run_id);
    std::fs::create_dir_all(&scratch).expect("create scratch");
    let observation_path = scratch.join("observation.json");
    std::fs::remove_file(&observation_path).ok();

    let output = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["boot", "run"])
        .arg("--title-manifest")
        .arg(&manifest)
        .arg("--max-steps")
        .arg(case.max_steps.to_string())
        .arg("--save-observation")
        .arg(&observation_path)
        .current_dir(workspace_root())
        // These are freestanding PSL1GHT binaries that bind no firmware
        // namespace. Suppressing auto-discovery keeps the verdict the
        // same on a machine that happens to have firmware installed.
        .env("CELLGOV_NO_FIRMWARE_DIR", "1")
        .output()
        .expect("spawn cellgov boot run");

    if !output.status.success() {
        eprintln!(
            "--- stdout ---\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        eprintln!(
            "--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        panic!("{}: cellgov boot run exited non-zero", case.name);
    }

    // Every microtest writes its CGOV length word from a stack buffer;
    // a dropped capture means the driver refused a mapped address.
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(line) = stdout
        .lines()
        .find(|l| l.starts_with("tty_oob_captures_dropped:"))
    {
        panic!("{}: boot run dropped TTY captures: {line}", case.name);
    }

    let json = std::fs::read_to_string(&observation_path).unwrap_or_else(|e| {
        panic!(
            "{}: read {}: {e}\nthe microtests feature declares the microtest tree built; \
             build it with tests/micro/{}/build.sh",
            case.name,
            observation_path.display(),
            case.name,
        )
    });
    serde_json::from_str(&json).expect("deserialize Observation")
}

/// The `u32` words of the guest's `CGOV` frame.
///
/// The frame is 4 magic bytes, a big-endian `u32` length, then that
/// many payload bytes. It is located by scanning rather than read at
/// offset 0: the RSX microtests emit a human-readable PASS/FAIL line
/// ahead of the struct.
fn cgov_words(case: &Case, tty: &[u8]) -> Vec<u32> {
    let start = tty
        .windows(4)
        .position(|w| w == b"CGOV")
        .unwrap_or_else(|| {
            panic!(
                "{}: no CGOV frame in {} bytes of TTY: {:?}",
                case.name,
                tty.len(),
                String::from_utf8_lossy(&tty[..tty.len().min(120)]),
            )
        });
    let len_at = start + 4;
    let body_at = len_at + 4;
    assert!(
        body_at <= tty.len(),
        "{}: CGOV frame truncated before its length field",
        case.name
    );
    let len = u32::from_be_bytes(tty[len_at..body_at].try_into().unwrap()) as usize;
    assert!(
        body_at + len <= tty.len(),
        "{}: CGOV frame declares {len} payload bytes but only {} follow",
        case.name,
        tty.len() - body_at,
    );
    assert!(
        len.is_multiple_of(4),
        "{}: CGOV payload {len} bytes is not a whole number of u32 words",
        case.name
    );
    tty[body_at..body_at + len]
        .chunks_exact(4)
        .map(|c| u32::from_be_bytes(c.try_into().unwrap()))
        .collect()
}

/// Check one case's payload, returning a description of each mismatch.
fn check_payload(case: &Case, observation: &Observation) -> Vec<String> {
    let mut problems = Vec::new();
    let words = cgov_words(case, &observation.tty_log);
    if words.len() != case.fields.len() {
        problems.push(format!(
            "payload is {} words, table expects {} ({})",
            words.len(),
            case.fields.len(),
            case.fields
                .iter()
                .map(|(n, _)| *n)
                .collect::<Vec<_>>()
                .join(", "),
        ));
        return problems;
    }
    for (&word, &(field, expect)) in words.iter().zip(case.fields) {
        match expect {
            Exact(want) if word != want => {
                problems.push(format!("{field}: expected 0x{want:08x}, got 0x{word:08x}"))
            }
            NonZero if word == 0 => {
                problems.push(format!("{field}: expected non-zero, got 0"));
            }
            _ => {}
        }
    }
    problems
}

/// A run that did not reach `sys_process_exit` leaves a truncated or
/// absent payload, so the outcome has to settle before any field is
/// read.
fn check_outcome(case: &Case, observation: &Observation) -> Option<String> {
    match observation.outcome {
        ObservedOutcome::Completed | ObservedOutcome::ProcessExit => None,
        other => Some(format!(
            "outcome={other:?} (expected ProcessExit); steps={:?}, max_steps={}",
            observation.metadata.steps, case.max_steps,
        )),
    }
}

/// Does this manifest declare a title `boot run` can boot?
///
/// Mirrors `TitleManifest::load_from_text`'s layout acceptance: a
/// `title` table under `[cellgov]`, or -- when the file carries no
/// `cellgov` key at all -- one at the root. The decision is taken off
/// the parsed document rather than off line text: a scan for the
/// literal `[cellgov.title]` misses the root-level layout and a spaced
/// `[ cellgov.title ]` header alike, and `[cellgov]` alone is the
/// compare-harness scenario shape, not a title.
///
/// `cellgov_cli` has no library target, so an integration test cannot
/// link the loader and this restates its rule.
///
/// # Panics
///
/// On a manifest that does not parse: silently classifying it as
/// non-bootable is the same hole under a different cause.
fn declares_a_cellgov_title(path: &std::path::Path, text: &str) -> bool {
    let doc: toml::Table = text
        .parse()
        .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    match doc.get("cellgov") {
        Some(nested) => nested.get("title").is_some(),
        None => doc.get("title").is_some(),
    }
}

#[path = "microtests/microtests_tests.rs"]
mod tests;

#[path = "microtests/hardware_match_tests.rs"]
mod hardware_match_tests;
