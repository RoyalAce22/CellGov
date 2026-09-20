//! `EnvTaps` hands the boot its observers, and the store watch stamps
//! each write with the PC the PPU half saw last.

use cellgov_boot::DebugTaps;
use cellgov_event::UnitId;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;

use super::EnvTaps;
use crate::game::taps::error::TapError;
use crate::game::taps::store_watch::{pack_record, StoreWatchSpec, RECORD_LEN};
use crate::game::taps::value_sample::ValueSampleSpec;

#[test]
fn a_store_write_carries_the_last_dispatched_pc() {
    let dir = cellgov_testkit::scratch::scratch_labeled("taps_bundle_store");
    let path = dir.join("store.bin");
    let spec = StoreWatchSpec::parse(Some("1000:10"), path.to_str())
        .unwrap()
        .unwrap();
    let taps = EnvTaps::open(None, Some(spec), None).unwrap();
    let ppu = taps.ppu().expect("the store watch needs the PPU half");
    let mut runtime = taps.runtime().expect("runtime half");
    assert!(
        taps.runtime().is_none(),
        "the runtime half is handed out once"
    );

    let mut state = PpuState::new();
    state.pc = 0x0001_2340;
    ppu.dispatch(UnitId::new(0), &PpuInstruction::Consumed, &state);
    runtime.write(0, 0x1004, &[0xEE; 4]);
    drop(runtime);

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[0..4], b"CGSW");
    assert_eq!(bytes.len(), 16 + RECORD_LEN);
    assert_eq!(
        &bytes[16..],
        &pack_record(
            0,
            0x0001_2340,
            0x1004,
            4,
            u64::from_le_bytes([0xEE, 0xEE, 0xEE, 0xEE, 0, 0, 0, 0])
        )
    );
}

#[test]
fn two_watches_naming_one_capture_are_refused_before_either_file_exists() {
    let dir = cellgov_testkit::scratch::scratch_labeled("taps_bundle_shared");
    let path = dir.join("shared.bin");
    let same = dir.join(".").join("shared.bin");
    let store = StoreWatchSpec::parse(Some("1000:10"), path.to_str())
        .unwrap()
        .unwrap();
    let sample = ValueSampleSpec::parse(Some("100:4"), same.to_str(), None)
        .unwrap()
        .unwrap();
    let err = EnvTaps::open(None, Some(store), Some(sample))
        .err()
        .expect("a shared capture path is refused");
    assert!(
        matches!(
            err,
            TapError::SharedCapture {
                first: "store-watch",
                second: "value-sample",
                ..
            }
        ),
        "{err}"
    );
    assert!(!path.exists(), "no capture is created");
}

#[test]
fn a_value_sample_alone_installs_no_ppu_observer() {
    let dir = cellgov_testkit::scratch::scratch_labeled("taps_bundle_sample");
    let path = dir.join("sample.bin");
    let spec = ValueSampleSpec::parse(Some("100:4"), path.to_str(), None)
        .unwrap()
        .unwrap();
    let taps = EnvTaps::open(None, None, Some(spec)).unwrap();
    assert!(taps.ppu().is_none());
    assert!(taps.runtime().is_some());
}
