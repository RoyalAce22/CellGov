//! CellGov's SPU stops the `spu_raw_status` probe's program where a
//! retail console does: the same `SPU_Status` and `SPU_NPC` words the
//! console reported through the problem-state window, read from its
//! committed capture.

use cellgov_core::Runtime;
use cellgov_exec::{StopRegisters, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// `tests/micro/spu_raw_status/spu/main.S` as the SPU assembler encodes
/// it (`spu-objdump` of the built image): `il $3,0x55`, `nop $127`,
/// `stop 0x1234`, `stop 0x2222`.
const PROGRAM: [u32; 4] = [0x4080_2a83, 0x4020_007f, 0x0000_1234, 0x0000_2222];

const CONSOLE: &str = "../../tests/micro/spu_raw_status/ps3/cech20-cex-493/observation.json";

fn console_stop() -> StopRegisters {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    StopRegisters {
        status: word(8),
        npc: word(12),
    }
}

#[test]
fn the_probe_program_stops_with_the_consoles_status_and_next_pc() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 100);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        for (i, word) in PROGRAM.iter().enumerate() {
            spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        spu
    });
    let step = rt.step().expect("a runnable unit");
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
    assert_eq!(step.result.yield_reason, YieldReason::Finished);
    assert_eq!(rt.unit_stop_registers(unit), Some(console_stop()));
}
