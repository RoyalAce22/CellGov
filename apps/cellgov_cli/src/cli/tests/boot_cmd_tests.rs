//! The bench step cap the anchor is recorded at.

use super::*;

fn manifest_with_bench_cap(bench_max_steps: Option<u64>) -> game::manifest::TitleManifest {
    use game::manifest::{CheckpointTrigger, Distribution, GameSource, TitleManifest};
    TitleManifest {
        content_id: "CG_TEST".to_string(),
        short_name: "test".to_string(),
        display_name: "test".to_string(),
        eboot_candidates: vec!["EBOOT.BIN".to_string()],
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps,
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
    }
}

#[test]
fn a_raised_manifest_cap_is_the_bench_default_not_the_hardcoded_one() {
    // The anchor is measured at this cap, and a bench run at any other
    // cap is reported incomparable and gates nothing.
    let title = manifest_with_bench_cap(Some(250_000_000));
    assert_eq!(default_bench_max_steps(&title), 250_000_000);
}

#[test]
fn a_manifest_without_a_bench_cap_takes_the_recorder_default() {
    let title = manifest_with_bench_cap(None);
    assert_eq!(
        default_bench_max_steps(&title) as u64,
        crate::paths::DEFAULT_BENCH_MAX_STEPS
    );
}
