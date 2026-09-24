//! The shared scalar load / store path: base-register writeback and
//! the update-form validity rules.

use super::*;

#[test]
fn lfsux_with_ra_index_equal_to_frt_index_is_a_valid_form() {
    // RA and FRT index different register files, so r3 as the base
    // and f3 as the target is a valid encoding; only RA=0 is not.
    let mut mem = vec![0u8; 0x100];
    mem[0x20..0x24].copy_from_slice(&1.5f32.to_bits().to_be_bytes());
    let mut s = PpuState::new();
    s.set_gpr(3, 0x10);
    s.set_gpr(5, 0x10);
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lfsux {
            frt: 3,
            ra: 3,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.fpr[3], 1.5f64.to_bits());
    assert_eq!(s.gpr[3], 0x20);
}

#[test]
fn lfsux_with_ra_zero_faults_as_an_invalid_form() {
    let mut s = PpuState::new();
    s.set_gpr(5, 0x10);
    s.set_fpr(3, 0x77);
    let (gpr, fpr) = (*s.gpr.as_array(), *s.fpr.as_array());
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lfsux {
            frt: 3,
            ra: 0,
            rb: 5,
        },
        &mut s,
        0,
        &[0x3Fu8; 0x100],
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Fault(PpuFault::InvalidForm("lfsux")));
    assert_eq!((*s.gpr.as_array(), *s.fpr.as_array()), (gpr, fpr));
    assert!(effects.is_empty());
}

#[test]
fn stwu_buffer_full_leaves_ra_unchanged() {
    let mut store_buf = StoreBuffer::new();
    while !store_buf.is_full() {
        store_buf.insert(0x1000, 4, 0).expect("staged");
    }
    let mut s = PpuState::new();
    s.set_gpr(1, 0x80);
    s.set_gpr(5, 0xDEAD_BEEF);
    let mem = [0u8; 0x200];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let mut effects = Vec::new();
    let v = execute(
        &PpuInstruction::Stwu {
            rs: 5,
            ra: 1,
            imm: -4,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::BufferFull);
    assert_eq!(
        s.gpr[1], 0x80,
        "a store that was not staged must not move RA"
    );
}

#[test]
fn ldux_fault_leaves_both_rt_and_ra_unchanged() {
    let mem = vec![0u8; 0x40];
    let mut s = PpuState::new();
    // RB is nonzero so EA differs from RA; a writeback ahead of the
    // load result would show as RA == EA.
    s.set_gpr(3, 0x1111);
    s.set_gpr(4, 0x1000_0000);
    s.set_gpr(5, 0x40);
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Ldux {
            rt: 3,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert!(matches!(v, ExecuteVerdict::MemFault(_)));
    assert_eq!(s.gpr[3], 0x1111);
    assert_eq!(s.gpr[4], 0x1000_0000);
}

#[test]
fn stfdu_stores_the_double_verbatim_and_writes_back_ra() {
    let mut s = PpuState::new();
    s.set_gpr(1, 0x100);
    s.set_fpr(2, 0x3FF8_0000_0000_0001);
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Stfdu {
            frs: 2,
            ra: 1,
            imm: -16,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[1], 0xF0);
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0xF0);
            assert_eq!(bytes.bytes(), &0x3FF8_0000_0000_0001u64.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}
