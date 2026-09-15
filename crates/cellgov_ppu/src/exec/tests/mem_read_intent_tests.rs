//! Which PPU loads record a read of committed memory. The store
//! buffer's forwarding decides it for each load.

use super::*;
use cellgov_effects::Effect;

/// Byte ranges the effects record as read, in emission order.
fn read_ranges(effects: &[Effect]) -> Vec<(u64, u64)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } => Some((range.start().raw(), range.length())),
            _ => None,
        })
        .collect()
}

fn word_at(addr: usize, value: u32) -> Vec<u8> {
    let mut mem = vec![0u8; 0x2000];
    mem[addr..addr + 4].copy_from_slice(&value.to_be_bytes());
    mem
}

#[test]
fn a_load_of_committed_memory_records_its_own_span() {
    let mem = word_at(0x1008, 0xDEAD_BEEF);
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1000);
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lwz {
            rt: 3,
            ra: 1,
            imm: 8,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(read_ranges(&effects), [(0x1008, 4)]);
}

#[test]
fn the_source_of_a_read_is_the_loading_unit() {
    let mem = word_at(0x1008, 1);
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1000);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwz {
            rt: 3,
            ra: 1,
            imm: 8,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::SharedReadIntent { source, .. } if *source == uid()
    )));
}

#[test]
fn a_load_the_store_buffer_forwards_whole_records_no_read() {
    let mem = vec![0u8; 0x2000];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1000);
    s.set_gpr(5, 0x1234_5678);
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    execute(
        &PpuInstruction::Stw {
            rs: 5,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    execute(
        &PpuInstruction::Lwz {
            rt: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(s.gpr[3], 0x1234_5678, "the load took the buffered bytes");
    assert!(
        read_ranges(&effects).is_empty(),
        "committed memory was never read: {effects:?}"
    );
}

#[test]
fn a_load_the_store_buffer_covers_only_in_part_records_its_whole_span() {
    let mem = vec![0u8; 0x2000];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1000);
    s.set_gpr(5, 0xAB);
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    // One byte staged inside the word the load then reads.
    execute(
        &PpuInstruction::Stb {
            rs: 5,
            ra: 1,
            imm: 1,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    execute(
        &PpuInstruction::Lwz {
            rt: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(s.gpr[3], 0x00AB_0000, "the buffered byte overlays the word");
    assert_eq!(read_ranges(&effects), [(0x1000, 4)]);
}

#[test]
fn a_faulting_load_records_no_read() {
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1000);
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let v = execute(
        &PpuInstruction::Lwz {
            rt: 3,
            ra: 1,
            imm: 8,
        },
        &mut s,
        uid(),
        &[],
        &mut effects,
        &mut store_buf,
    );
    assert!(matches!(v, ExecuteVerdict::MemFault(_)));
    assert!(read_ranges(&effects).is_empty());
}

#[test]
fn a_store_records_no_read() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1000);
    s.set_gpr(5, 7);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stw {
            rs: 5,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert!(read_ranges(&effects).is_empty());
}

#[test]
fn lmw_records_one_read_per_word() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1000);
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lmw {
            rt: 29,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(
        read_ranges(&effects),
        [(0x1000, 4), (0x1004, 4), (0x1008, 4)]
    );
}

#[test]
fn a_vector_load_records_the_whole_aligned_line() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1004);
    s.set_gpr(2, 0);
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lvx {
            vt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(read_ranges(&effects), [(0x1000, 16)]);
}

#[test]
fn an_unaligned_lvrx_records_the_aligned_line_it_reads() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1003);
    s.set_gpr(2, 0);
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lvrx {
            vt: 7,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(read_ranges(&effects), [(0x1000, 16)]);
}

/// [CBE-Handbook p:744 s:A.3.3 Table A-9] a quadword-aligned `lvrx`
/// makes no attempt to access storage: an unmapped line neither
/// faults nor is recorded as read.
#[test]
fn a_quadword_aligned_lvrx_touches_no_storage() {
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1000);
    s.set_gpr(2, 0);
    s.set_vr(7, u128::MAX);
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let v = execute(
        &PpuInstruction::Lvrx {
            vt: 7,
            ra: 1,
            rb: 2,
        },
        &mut s,
        uid(),
        &[],
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.vr[7], 0);
    assert!(read_ranges(&effects).is_empty(), "{effects:?}");
}

#[test]
fn a_quadword_aligned_lvrxl_touches_no_storage() {
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1000);
    s.set_gpr(2, 0);
    s.set_vr(7, u128::MAX);
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let v = execute(
        &PpuInstruction::Lvrxl {
            vt: 7,
            ra: 1,
            rb: 2,
        },
        &mut s,
        uid(),
        &[],
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.vr[7], 0);
    assert!(read_ranges(&effects).is_empty(), "{effects:?}");
}

#[test]
fn lwarx_records_the_read_before_it_takes_the_reservation() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1000);
    s.set_gpr(2, 0);
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lwarx {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::SharedReadIntent { .. },
                Effect::ReservationAcquire { .. }
            ]
        ),
        "{effects:?}"
    );
}
