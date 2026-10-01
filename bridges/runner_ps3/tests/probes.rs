//! The console probes' committed answers: what a retail console reported
//! for a raw SPU stopped by `stop 0x1234`, and the on-SPU compare mode's
//! counts for a sound model and a model broken at one input.
//!
//! Each capture also replays (`tests/replay.rs`); this target pins the
//! values themselves, so a recapture that changes one is a failure to
//! read, not a silent update.

use std::path::Path;

use cellgov_compare::baseline;

fn payload(test: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(test)
        .join("ps3/cech20-cex-493/observation.json");
    let observation = baseline::load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    observation.memory_regions[0].data.clone()
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
}

fn doubleword(bytes: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
}

#[test]
fn a_raw_spu_stopped_by_stop_and_signal_reports_its_code_and_the_next_address() {
    let p = payload("spu_raw_status");
    assert_eq!(p.len(), 64);
    assert_eq!((word(&p, 0), word(&p, 4)), (0, 0), "no step failed");
    // SPU_Status: the stop code in the upper halfword, and the
    // stop-and-signal bit (0x2); the run bit (0x1) clear.
    assert_eq!(word(&p, 8), 0x1234_0002);
    // SPU_NextPC: the address after the `stop` at local-store 0x8.
    assert_eq!(word(&p, 12), 0xC);
    assert_eq!(word(&p, 20), 1, "the first status poll saw the stop");
    // Interrupt-status reads: class 0 and 2 answer CELL_OK; class 1 is
    // refused to a user process with CELL_EINVAL.
    assert_eq!(
        (word(&p, 24), word(&p, 28), word(&p, 32)),
        (0, 0x8001_0002, 0)
    );
    assert_eq!(doubleword(&p, 40), 0, "class 0 status");
    assert_eq!(
        doubleword(&p, 56),
        0x12,
        "class 2 status: stop-and-signal plus bit 0x10"
    );
}

#[test]
fn the_on_spu_compare_finds_no_mismatch_in_a_sound_model_and_the_one_planted_in_a_broken_one() {
    let p = payload("spu_sweep_compare");
    assert_eq!(p.len(), 32);
    assert_eq!(word(&p, 0), 0, "the sweep ran");
    assert_eq!(word(&p, 4), 0, "the identity model never disagrees");
    assert_eq!(word(&p, 8), 1, "the broken model disagrees once");
    assert_eq!(word(&p, 12), 0x1234_5678, "at the planted input");
    assert_eq!(word(&p, 16), 0x1234_5678 | 0x5A5A_5A5A, "the hardware's or");
    assert_eq!(
        word(&p, 20),
        (0x1234_5678 | 0x5A5A_5A5A) ^ 1,
        "the broken model"
    );
    assert_eq!(
        (u64::from(word(&p, 24)) << 32) | u64::from(word(&p, 28)),
        1 << 32,
        "every 32-bit input"
    );
}
