//! The primary thread's CR, r0 and CTR held to the `ppu_entry_state`
//! and `ppu_entry_tls` console captures.

use super::{seed_process_start_registers, ProcessStart};
use crate::prx::TlsArea;

/// The save area of the main thread starts 16 bytes into each payload.
const MAIN_SAVE: usize = 16;
/// The save area's CR and CTR slots.
const CR_SLOT: usize = 32;
const CTR_SLOT: usize = 34;
/// Entry code address of both probe images: the start of `.text`, as
/// their `build.sh` links it.
const PROBE_ENTRY_CODE: u64 = 0x1_0200;

/// The main thread's r0..r31, CR, XER, CTR, LR and VRSAVE.
fn console(probe: &str) -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/observation.json");
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(&p[12..16], b"ENTR", "{probe}: the probe wrote every word");
    p[MAIN_SAVE..MAIN_SAVE + 37 * 8]
        .chunks(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8")))
        .collect()
}

/// CellGov's entry state for a probe image loaded at its entry code.
fn cellgov() -> cellgov_ppu::state::PpuState {
    let mut state = cellgov_ppu::state::PpuState::new();
    state.pc = PROBE_ENTRY_CODE;
    let start = ProcessStart {
        entry: 0x1000_0000,
        tls: TlsArea::above(0x1001_0000).expect("in range"),
    };
    seed_process_start_registers(&mut state, 0x10_0000, start);
    state
}

#[test]
fn the_primary_entry_cr_r0_and_ctr_match_the_console() {
    let s = cellgov();
    for probe in ["ppu_entry_state", "ppu_entry_tls"] {
        let c = console(probe);
        assert_eq!(u64::from(s.cr()), c[CR_SLOT], "{probe}: CR");
        assert_eq!(s.gpr[0], c[0], "{probe}: r0");
        assert_eq!(s.ctr(), c[CTR_SLOT], "{probe}: CTR");
    }
}

#[test]
fn r0_and_ctr_follow_the_entry_code_address() {
    let mut state = cellgov_ppu::state::PpuState::new();
    state.pc = 0x2_3450;
    let start = ProcessStart {
        entry: 0x1000_0000,
        tls: TlsArea::above(0x1001_0000).expect("in range"),
    };
    seed_process_start_registers(&mut state, 0x10_0000, start);
    assert_eq!(state.gpr[0], 0x2_3450);
    assert_eq!(state.ctr(), 0x2_3450);
}
