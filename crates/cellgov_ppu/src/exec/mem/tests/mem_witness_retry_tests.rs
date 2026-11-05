//! The dcbz and store-conditional witness counters count retirements.

use super::*;
use cellgov_ps3_abi::hw::ppu::DCBZ_BLOCK_BYTES;

/// Returns a store buffer with exactly `free` slots left. It finds the
/// capacity from a scratch buffer that it fills until an insert fails.
fn buffer_with_free_slots(free: usize) -> StoreBuffer {
    let mut buf = StoreBuffer::new();
    let mut staged = 0usize;
    for i in 0..4096u64 {
        if buf.insert(0x8000 + i * 8, 8, 0).is_err() {
            break;
        }
        staged += 1;
    }
    assert!(staged > free, "capacity {staged}");
    let mut refill = StoreBuffer::new();
    for i in 0..(staged - free) as u64 {
        refill.insert(0x8000 + i * 8, 8, 0).expect("staged");
    }
    refill
}

/// Runs `insn` against `buf` until the verdict is not `BufferFull` and
/// returns the attempt count. The helper flushes `buf` between attempts.
fn run_until_retired(insn: &PpuInstruction, s: &mut PpuState, buf: &mut StoreBuffer) -> usize {
    let mem = [0u8; 0x10000];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let mut effects = Vec::new();
    for attempt in 1..=2 {
        let v = execute(insn, s, UnitId::new(0), &views, &mut effects, buf);
        if v != ExecuteVerdict::BufferFull {
            assert_eq!(v, ExecuteVerdict::Continue);
            return attempt;
        }
        buf.flush(&mut effects, UnitId::new(0));
    }
    panic!("still BufferFull after a flush");
}

/// [CBE-Handbook p:147 s:6.1.6.4] dcbz is one store of a 128-byte block,
/// so one dcbz retires once.
#[test]
fn a_dcbz_that_retries_on_a_full_buffer_counts_once() {
    let mut s = PpuState::new();
    s.set_gpr(1, 0x2000);
    s.set_gpr(2, 0);
    let mut buf = buffer_with_free_slots(DCBZ_BLOCK_BYTES / 8 - 1);
    let attempts = run_until_retired(&PpuInstruction::Dcbz { ra: 1, rb: 2 }, &mut s, &mut buf);
    assert_eq!(attempts, 2);
    assert_eq!(s.dcbz_executed, 1);
}

/// Returns `stwcx.` and `stdcx.`, each with the `(stwcx, stdcx)`
/// counter pair after one retirement.
fn store_conditionals() -> [(PpuInstruction, (u64, u64)); 2] {
    [
        (
            PpuInstruction::Stwcx {
                rs: 6,
                ra: 4,
                rb: 5,
            },
            (1, 0),
        ),
        (
            PpuInstruction::Stdcx {
                rs: 6,
                ra: 4,
                rb: 5,
            },
            (0, 1),
        ),
    ]
}

#[test]
fn a_store_conditional_that_retries_on_a_full_buffer_counts_once() {
    for (insn, counted) in store_conditionals() {
        let mut s = PpuState::new();
        s.set_gpr(4, 0x1000);
        s.set_gpr(5, 0);
        s.set_gpr(6, 0x1122_3344_5566_7788);
        s.set_reservation(Some(ReservedLine::containing(0x1000)));
        let mut buf = buffer_with_free_slots(0);
        let attempts = run_until_retired(&insn, &mut s, &mut buf);
        assert_eq!(attempts, 2, "{insn:?}");
        assert_eq!((s.stwcx_executed, s.stdcx_executed), counted, "{insn:?}");
        assert_eq!(s.cr_field(0) & 0b0010, 0b0010, "{insn:?}: stored");
    }
}

#[test]
fn a_store_conditional_that_faults_is_not_counted() {
    for (insn, _) in store_conditionals() {
        // 0x1002 is misaligned for both widths; 0x2000 is past the region.
        let faults = [
            (
                0x1002u64,
                ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(0x1002)),
            ),
            (
                0x2000,
                ExecuteVerdict::StoreFault(crate::exec::memory_helpers::unmapped(0x2000)),
            ),
        ];
        for (ea, fault) in faults {
            let mut s = PpuState::new();
            s.set_gpr(4, ea);
            s.set_gpr(5, 0);
            s.set_reservation(Some(ReservedLine::containing(0x1000)));
            let mut effects = Vec::new();
            let v = exec_with_mem(&insn, &mut s, 0, &[0u8; 0x2000], &mut effects);
            assert_eq!(v, fault, "{insn:?} at {ea:#x}");
            assert_eq!(
                (s.stwcx_executed, s.stdcx_executed),
                (0, 0),
                "{insn:?} at {ea:#x}"
            );
        }
    }
}
