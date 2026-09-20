use super::*;
use cellgov_boot::manifest::{CheckpointTrigger, Distribution, GameSource};
use cellgov_compare::{BootOverrides, RunIdentity};

fn manifest() -> TitleManifest {
    TitleManifest {
        content_id: "TEST00001".to_string(),
        short_name: "synthetic".to_string(),
        display_name: "Synthetic Title".to_string(),
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
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: Vec::new(),
    }
}

#[test]
fn an_overridden_run_names_every_override_in_the_banner() {
    let identity = RunIdentity {
        overrides: BootOverrides {
            skip_module_start: true,
            force_system_authid: true,
            prx_base: Some(0x3000_0000),
            disable_module_start_hle_stubs: true,
        },
        ..RunIdentity::default()
    };
    let composition = BootComposition {
        firmware: FirmwareChoice::None,
        game: GameChoice::Unstored,
        mounts: Vec::new(),
        eboot_dirs: Vec::new(),
        understated_firmware: Vec::new(),
        identity,
    };

    assert_eq!(
        render(&manifest(), &composition),
        vec![
            "title    synthetic  TEST00001  Synthetic Title",
            "game     no store entry  (content resolved from the VFS root)",
            "firmware none  (imports answer through trampolines)",
            "override skip_module_start force_system_authid prx_base=0x30000000 disable_module_start_hle_stubs",
        ]
    );
}
