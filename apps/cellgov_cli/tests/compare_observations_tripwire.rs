//! Guard-liveness gate for the divergence-classifier cluster in
//! `cellgov_compare`. The audit's bucket-B finding is that whenever
//! `compare_observations` runs on a pair of observations that
//! produce at least one byte divergence, four guarded paths fire
//! as a unit:
//!
//! - the `collect_byte_divergences` producer
//!   (`observation_compare.rs:499`) populates one `ByteDivergence`
//!   per run, with `length > 0`.
//! - the `classify` consumer (`classify.rs:232`) runs on each
//!   `ByteDivergence`.
//! - the `format_observation_compare_human` render-time offset
//!   guard (`observation_compare.rs:633`) walks the same runs to
//!   produce DIVERGE lines.
//! - the `summarize` lowest-offset non-overlap guard
//!   (`summary.rs:486`) is reached when more than one run crosses
//!   the summarizer.
//!
//! The fixture is constructed in-test from `cellgov_compare`'s
//! public types so this test never re-bakes a particular title or
//! corpus snapshot. A correctness-improving trajectory shift in
//! any title -- new byte divergences appearing, old ones being
//! reclassified, the corpus being re-decrypted -- does not break
//! this test, by design. It breaks only when the producer /
//! consumer wiring breaks: a guarded function dropped from the
//! pipeline, a `debug_assert!` removed, or an arm renamed without
//! updating downstream.
//!
//! The two OneMissing-shape guards
//! (`observation_compare.rs:544/572`) sit on
//! `StateHashCompare::OneMissing` and `StepCompare::OneMissing`
//! paths and are exercised in `cellgov_compare`'s in-crate unit
//! tests (`compare_state_hashes`/`compare_steps` over a Some/None
//! pair); the bucket-B treatment here covers the divergence-
//! classifier cluster specifically.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap panics on unexpected failure are the right behavior"
)]

use cellgov_compare::{
    classify, compare_observations, format_observation_compare_human, summarize, ByteDivergence,
    ClassifierContext, DivergenceClass, NamedMemoryRegion, Observation, ObservationMetadata,
    ObservedOutcome, RegionPairOutcome, CODE_REGION_NAME, ELF_HEADER_SIZE,
};

/// Build a minimal observation pair that diverges in two regions at
/// three byte runs. Title-agnostic: all addresses, region names, and
/// data shapes are local to this test and carry no corpus state.
///
/// Layout: a `code` region of `ELF_HEADER_SIZE` bytes (needed for
/// `ClassifierContext::from_observation`) with two divergence runs,
/// plus a `scratch` region with one more run -- three runs total
/// across two regions, enough to exercise `summarize`'s lowest-
/// offset comparison across the divergence set.
fn synthetic_divergent_pair() -> (Observation, Observation) {
    let mut a_code = vec![0u8; ELF_HEADER_SIZE];
    let mut b_code = vec![0u8; ELF_HEADER_SIZE];
    // Run 1: 4 bytes at offset 8.
    a_code[8..12].copy_from_slice(&[0xAA, 0xAA, 0xAA, 0xAA]);
    b_code[8..12].copy_from_slice(&[0xBB, 0xBB, 0xBB, 0xBB]);
    // Run 2: 2 bytes at offset 32.
    a_code[32] = 0x11;
    a_code[33] = 0x22;
    b_code[32] = 0x33;
    b_code[33] = 0x44;

    let mut a_scratch = vec![0u8; 16];
    let mut b_scratch = vec![0u8; 16];
    // Run 3: 1 byte at offset 4.
    a_scratch[4] = 0xCC;
    b_scratch[4] = 0xDD;

    let regions_a = vec![
        NamedMemoryRegion {
            name: CODE_REGION_NAME.into(),
            addr: 0x0001_0000,
            data: a_code,
        },
        NamedMemoryRegion {
            name: "scratch".into(),
            addr: 0x0002_0000,
            data: a_scratch,
        },
    ];
    let regions_b = vec![
        NamedMemoryRegion {
            name: CODE_REGION_NAME.into(),
            addr: 0x0001_0000,
            data: b_code,
        },
        NamedMemoryRegion {
            name: "scratch".into(),
            addr: 0x0002_0000,
            data: b_scratch,
        },
    ];
    let make = |regions: Vec<NamedMemoryRegion>, runner: &str| Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions: regions,
        events: Vec::new(),
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: runner.into(),
            steps: None,
        },
        tty_log: Vec::new(),
    };
    (
        make(regions_a, "synthetic-a"),
        make(regions_b, "synthetic-b"),
    )
}

#[test]
fn divergence_classifier_cluster_fires_on_synthetic_pair() {
    let (a, b) = synthetic_divergent_pair();

    let result = compare_observations(&a, &b);

    // Format-time offset guard at observation_compare.rs:633.
    // Running format on a live divergent result is what makes the
    // debug_assert silence non-vacuous.
    let formatted = format_observation_compare_human(&result);
    assert!(
        !formatted.is_empty(),
        "format_observation_compare_human returned empty on a divergent pair; \
         the DIVERGE-emit branch is no longer wired"
    );

    // Producer output: every ByteDivergence carries length > 0 per
    // collect_byte_divergences' invariant.
    let mut byte_divs: Vec<(u64, &ByteDivergence)> = Vec::new();
    for pair in &result.region_compare.pairs {
        if let RegionPairOutcome::ByteDivergence { addr, bytes, .. } = pair {
            for div in bytes {
                byte_divs.push((*addr, div));
            }
        }
    }
    assert!(
        !byte_divs.is_empty(),
        "synthetic divergent pair produced zero ByteDivergence runs; \
         compare_observations no longer reaches the byte-divergence emit branch"
    );
    assert!(
        byte_divs.len() >= 2,
        "synthetic pair must produce >=2 runs across the two regions so \
         summarize's lowest-offset non-overlap guard at summary.rs:486 is exercised; \
         observed only {}",
        byte_divs.len()
    );

    // Classifier consumer: classify.rs:232 fires on every run.
    let ctx = ClassifierContext::from_observation(&a)
        .expect("synthetic 'code' region is well-formed and large enough");
    let classes: Vec<DivergenceClass> = byte_divs
        .iter()
        .map(|(addr, div)| classify(div, *addr, &ctx))
        .collect();
    assert_eq!(classes.len(), byte_divs.len());

    // Summarize: feeds the classes into the lowest-offset non-overlap
    // guard. Structural exercise only; the output's contents are not
    // asserted on (this is a liveness test, not a snapshot).
    let _ = summarize(&result, &classes);
}

#[test]
fn synthetic_pair_is_constructed_divergent_not_identical() {
    // Vacuity pin. If a future edit accidentally makes the synthetic
    // pair identical (e.g. swapping which buffer gets a 0xBB), every
    // assertion above passes silently against an empty divergence
    // set. This test enforces that the constructor itself stays
    // honest: the gate is meaningful only on actually-divergent
    // observations.
    let (a, b) = synthetic_divergent_pair();
    assert_ne!(
        a, b,
        "synthetic_divergent_pair returned identical observations -- \
         the liveness gate would pass vacuously"
    );
    assert!(
        a.memory_regions
            .iter()
            .any(|r| r.name == CODE_REGION_NAME && r.data.len() >= ELF_HEADER_SIZE),
        "synthetic_divergent_pair must include a 'code' region of >=ELF_HEADER_SIZE so \
         ClassifierContext::from_observation succeeds"
    );
}
