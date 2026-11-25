//! CellGov's PPU against the `ppu_ctrl_threads` console capture: the CTRL
//! value that a thread fixed on one PPE hardware thread reads beside a
//! runnable sibling and with none.

use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_spr::{
    CTRL_CT_THREAD0, CTRL_CT_THREAD1, CTRL_TE_THREAD0, CTRL_TE_THREAD1,
};
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_ctrl_threads/ps3/cech20-cex-493/observation.json";

/// The payload's word 37 when the probe wrote every word.
const DONE: u64 = 0x4354_5254;

/// CTRL TE, bits 8:9.
const TE: u64 = 0x00C0_0000;

/// Every distinct CTRL value that the three console threads read.
fn console_values() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 304);
    let w: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[37], DONE, "the probe wrote every word");
    let mut values = Vec::new();
    for k in 0..3 {
        let b = 12 * k;
        for i in 0..4 {
            if w[b + 8 + i] != 0 && !values.contains(&w[b + 4 + i]) {
                values.push(w[b + 4 + i]);
            }
        }
    }
    values
}

/// The CTRL that a thread with the fixed CTRL `fixed` reads beside
/// `siblings` runnable PPE threads.
fn fixed_read(fixed: u32, siblings: u32) -> u64 {
    const CODE: u64 = 0x1000;
    let mut mem = GuestMemory::new(0x2000);
    for (i, w) in [0x7C68_22A6u32, 0x4800_0000].iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(CODE + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = CODE;
    unit.fix_ctrl(fixed);
    let ctx = ExecutionContext::new(&mem).with_ppe_siblings(siblings);
    unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    assert_eq!(unit.state().pc, CODE + 4, "mfspr r3, 136 retired");
    unit.state().gpr[3]
}

#[test]
fn a_fixed_thread_reads_both_te_bits_beside_a_sibling_and_its_own_alone() {
    let console = console_values();
    for ct in [CTRL_CT_THREAD0, CTRL_CT_THREAD1] {
        let fixed = ct | CTRL_TE_THREAD0 | CTRL_TE_THREAD1;
        let beside = fixed_read(fixed, 1);
        assert_eq!(beside, u64::from(fixed), "CT 0x{ct:08x} beside a sibling");
        assert!(
            console.contains(&beside),
            "0x{beside:08x} not among {console:x?}"
        );
        // The value the console read on the same hardware thread with one
        // thread enabled.
        let console_alone = console
            .iter()
            .copied()
            .find(|v| v >> 30 == u64::from(ct) >> 30 && v & TE != TE)
            .unwrap_or_else(|| panic!("no one-thread value for CT 0x{ct:08x}"));
        assert_eq!(fixed_read(fixed, 0), console_alone, "CT 0x{ct:08x} alone");
    }
}
