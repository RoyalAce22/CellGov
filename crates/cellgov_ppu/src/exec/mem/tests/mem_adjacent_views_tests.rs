//! Loads and stores that run from one writable view into an adjacent
//! one, as an access runs across two pages mapped side by side.

use super::*;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};

const EDGE: u64 = 0x1_0000;

/// Two read-write regions that meet at `EDGE`, plus a third after a gap.
fn memory() -> GuestMemory {
    GuestMemory::from_regions(vec![
        Region::new(0, EDGE as usize, "low", PageSize::Page64K),
        Region::new(EDGE, 0x1000, "high", PageSize::Page4K),
        Region::new(EDGE + 0x2000, 0x1000, "past_gap", PageSize::Page4K),
    ])
    .unwrap()
}

fn run(
    insn: PpuInstruction,
    s: &mut PpuState,
    mem: &GuestMemory,
) -> (ExecuteVerdict, Vec<Effect>, StoreBuffer) {
    let views: Vec<cellgov_mem::RegionView<'_>> = mem.region_views().collect();
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let v = execute(
        &insn,
        s,
        UnitId::new(0),
        &views,
        &mut effects,
        &mut store_buf,
    );
    (v, effects, store_buf)
}

#[test]
fn a_store_across_two_adjacent_writable_views_is_buffered_whole() {
    let mem = memory();
    let mut s = PpuState::new();
    s.set_gpr(1, EDGE - 2);
    s.set_gpr(3, 0xAABB_CCDD);
    let (v, _, store_buf) = run(
        PpuInstruction::Stw {
            rs: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        &mem,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(store_buf.forward(EDGE - 2, 4), Some(0xAABB_CCDD));
}

#[test]
fn a_load_across_two_adjacent_writable_views_reads_both_and_records_the_whole_range() {
    let mut mem = memory();
    let range = ByteRange::new(GuestAddr::new(EDGE - 4), 8).unwrap();
    mem.apply_commit(range, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    let mut s = PpuState::new();
    s.set_gpr(1, EDGE - 4);
    let (v, effects, _) = run(
        PpuInstruction::Ld {
            rt: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        &mem,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[3], 0x0102_0304_0506_0708);
    assert_eq!(
        effects,
        vec![Effect::SharedReadIntent {
            range,
            source: UnitId::new(0)
        }]
    );
}

#[test]
fn a_store_running_past_adjacent_views_into_a_gap_faults_at_the_first_unheld_byte() {
    let mem = memory();
    let mut s = PpuState::new();
    s.set_gpr(1, EDGE + 0x1000 - 2);
    let (v, _, store_buf) = run(
        PpuInstruction::Stw {
            rs: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        &mem,
    );
    assert!(
        matches!(v, ExecuteVerdict::StoreFault(cellgov_mem::MemError::Unmapped(ctx)) if ctx.addr == EDGE + 0x1000),
        "{v:?}"
    );
    assert!(store_buf.is_empty());
}

#[test]
fn a_store_from_a_writable_view_into_a_write_refusing_one_faults_at_the_refusing_views_first_byte()
{
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, EDGE as usize, "low", PageSize::Page64K),
        Region::with_access(
            EDGE,
            0x1000,
            "reserved",
            PageSize::Page4K,
            cellgov_mem::RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    let mut s = PpuState::new();
    s.set_gpr(1, EDGE - 2);
    let (v, _, store_buf) = run(
        PpuInstruction::Stw {
            rs: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        &mem,
    );
    assert_eq!(
        v,
        ExecuteVerdict::StoreFault(cellgov_mem::MemError::ReservedWrite {
            addr: EDGE,
            region: "reserved",
        })
    );
    assert!(store_buf.is_empty());
}

#[test]
fn a_load_running_past_adjacent_views_into_a_gap_faults_at_the_first_unheld_byte() {
    let mem = memory();
    let mut s = PpuState::new();
    s.set_gpr(1, EDGE + 0x1000 - 4);
    s.set_gpr(3, 0xDEAD_BEEF);
    let (v, effects, _) = run(
        PpuInstruction::Ld {
            rt: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        &mem,
    );
    assert!(
        matches!(v, ExecuteVerdict::MemFault(cellgov_mem::MemError::Unmapped(ctx)) if ctx.addr == EDGE + 0x1000),
        "{v:?}"
    );
    assert_eq!(s.gpr[3], 0xDEAD_BEEF, "a faulting load leaves RT alone");
    assert!(effects.is_empty(), "a faulting load records no read");
}
