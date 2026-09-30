//! `diff compare --save-baseline` then `--against-baseline` on the
//! same manifest must classify MATCH.
//!
//! The two runs must observe the same memory regions. A comparison
//! matches regions by name. A baseline saved without the manifest's
//! `[observe] memory_regions` reads every region the compare run
//! captures as a divergence, so no manifest that names a region
//! classifies MATCH.
//!
//! The manifest names the `dma` scenario, which is synthetic: the gate
//! needs no title, no firmware and no key vault.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap panics on unexpected failure are the right behavior"
)]

use cellgov_testkit::scratch::scratch_labeled;
use std::path::Path;
use std::process::Command;

/// The `dma` scenario copies `de ad be ef` from address 0 to address
/// 128, so the region below holds content rather than zeros.
const MANIFEST: &str = r#"
[test]
name = "baseline_round_trip"

[cellgov]
scenario = "dma"

[observe]
memory_regions = [
  { name = "dma_dst", addr = 128, size = 4 },
]

[expect]
outcome = "completed"
"#;

const DMA_PAYLOAD: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];

struct Scratch(cellgov_testkit::scratch::ScratchDir);

impl Scratch {
    fn new() -> Self {
        Self(scratch_labeled("compare_baseline_round_trip"))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

fn save(manifest: &Path, baseline: &Path) -> std::process::Output {
    // `--save-baseline` conflicts with `--mode`: a baseline records the
    // run, so no mode applies to it.
    Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["diff", "compare"])
        .arg(manifest)
        .arg("--save-baseline")
        .arg(baseline)
        .output()
        .expect("spawn cellgov diff compare --save-baseline")
}

fn compare_against(manifest: &Path, baseline: &Path, mode: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["diff", "compare"])
        .arg(manifest)
        .arg("--against-baseline")
        .arg(baseline)
        .args(["--mode", mode])
        .output()
        .expect("spawn cellgov diff compare --against-baseline")
}

#[test]
fn a_manifest_baseline_round_trip_classifies_match() {
    let scratch = Scratch::new();
    let manifest = scratch.path().join("round_trip.toml");
    let baseline = scratch.path().join("round_trip.json");
    std::fs::write(&manifest, MANIFEST).unwrap();

    let saved = save(&manifest, &baseline);
    assert!(
        saved.status.success(),
        "--save-baseline exited {:?}\nstdout: {}\nstderr: {}",
        saved.status.code(),
        String::from_utf8_lossy(&saved.stdout),
        String::from_utf8_lossy(&saved.stderr)
    );

    // Vacuity pin: the round trip below says nothing about region
    // threading unless the baseline carries the region the manifest
    // names.
    let observation = cellgov_compare::baseline::load(&baseline).expect("saved baseline parses");
    let region = observation
        .memory_regions
        .iter()
        .find(|r| r.name == "dma_dst")
        .unwrap_or_else(|| {
            panic!(
                "saved baseline carries no \"dma_dst\" region; regions present: {:?}",
                observation
                    .memory_regions
                    .iter()
                    .map(|r| r.name.as_str())
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(
        region.data, DMA_PAYLOAD,
        "the dma scenario no longer lands its payload at address 128; \
         the region would be all zeros and the pin would be weaker"
    );

    // Both modes that read memory_regions. Either one reports a
    // missing baseline region as a divergence.
    for mode in ["memory", "strict"] {
        let against = compare_against(&manifest, &baseline, mode);
        let stdout = String::from_utf8_lossy(&against.stdout);
        assert!(
            stdout.contains("classification: MATCH"),
            "mode {mode}: a baseline saved from this manifest did not match its own \
             re-run\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&against.stderr)
        );
        assert!(
            against.status.success(),
            "mode {mode}: --against-baseline exited {:?} on a MATCH",
            against.status.code()
        );
    }
}
