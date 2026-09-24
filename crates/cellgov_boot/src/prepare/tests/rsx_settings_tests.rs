//! The RSX settings a manifest opts into reach the runtime.

use super::apply_rsx_settings;
use crate::manifest::{CheckpointTrigger, Distribution, GameSource, TitleManifest};
use cellgov_core::Runtime;
use cellgov_time::Budget;

fn manifest(rsx_mirror: bool, rsx_consume: bool) -> TitleManifest {
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
        bench_max_steps: None,
        system_ver: None,
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror,
        rsx_consume,
        content: None,
        mounts: Vec::new(),
        matrix: Vec::new(),
    }
}

fn runtime() -> Runtime {
    Runtime::new(cellgov_mem::GuestMemory::new(0x1000), Budget::new(1), 1)
}

#[test]
fn each_rsx_flag_reaches_the_runtime_on_its_own() {
    for (mirror, consume) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut rt = runtime();
        apply_rsx_settings(&mut rt, &manifest(mirror, consume));
        assert_eq!(
            (
                rt.rsx_mirror_writes_enabled(),
                rt.rsx_consume_fifo_enabled()
            ),
            (mirror, consume),
            "mirror = {mirror}, consume = {consume}"
        );
    }
}
