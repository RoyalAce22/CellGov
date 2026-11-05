//! The primary thread's entry registers held to the `ppu_entry_state`
//! and `ppu_entry_tls` console captures: one image without a TLS
//! segment and one with.

use super::{seed_process_start_registers, ProcessStart};
use crate::prx::TlsArea;

/// The save area of the main thread starts 16 bytes into each payload.
const MAIN_SAVE: usize = 16;
/// The save area's LR slot.
const LR_SLOT: usize = 35;
/// The ELF entry (the OPD entry descriptor's address) of both probe
/// images their `build.sh` produces.
const PROBE_ENTRY_DESCRIPTOR: u64 = 0x1000_0000;

/// The main thread's 37 saved registers: r0..r31, CR, XER, CTR, LR,
/// VRSAVE.
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

/// CellGov's entry registers for the capture's image: the arguments
/// in r3..r5 as the console passed them, and the image's entry
/// descriptor.
fn cellgov(c: &[u64]) -> cellgov_ppu::state::PpuState {
    let mut state = cellgov_ppu::state::PpuState::new();
    for (r, &v) in c.iter().enumerate().take(6).skip(3) {
        state.set_gpr(r, v);
    }
    let start = ProcessStart {
        entry: PROBE_ENTRY_DESCRIPTOR,
        tls: TlsArea::above(0x1001_0000).expect("in range"),
    };
    seed_process_start_registers(&mut state, c[12] as u32, start);
    state
}

#[test]
fn the_primary_entry_registers_match_the_console_with_and_without_a_tls_segment() {
    for probe in ["ppu_entry_state", "ppu_entry_tls"] {
        let c = console(probe);
        let s = cellgov(&c);
        for r in [6, 8, 9, 11, 28, 29, 31] {
            assert_eq!(s.gpr[r], c[r], "{probe}: r{r}");
        }
    }
}

#[test]
fn r8_carries_the_entry_descriptor_address() {
    let mut state = cellgov_ppu::state::PpuState::new();
    let start = ProcessStart {
        entry: 0x1002_3450,
        tls: TlsArea::above(0x1001_0000).expect("in range"),
    };
    seed_process_start_registers(&mut state, 0x10_0000, start);
    assert_eq!(state.gpr[8], 0x1002_3450);
}

#[test]
fn the_registers_cellgov_leaves_zero_are_nonzero_on_the_console() {
    for probe in ["ppu_entry_state", "ppu_entry_tls"] {
        let c = console(probe);
        let s = cellgov(&c);
        for (name, console, ours) in [
            ("r7", c[7], s.gpr[7]),
            ("r10", c[10], s.gpr[10]),
            ("lr", c[LR_SLOT], s.lr()),
        ] {
            assert_ne!(console, 0, "{probe}: the console's {name}");
            assert_eq!(ours, 0, "{probe}: CellGov's {name}");
        }
    }
}

#[test]
fn r10_moves_with_the_image_on_the_console() {
    assert_ne!(console("ppu_entry_state")[10], console("ppu_entry_tls")[10]);
}
