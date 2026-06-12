//! lwarx / ldarx / stwcx. / stdcx. reservation semantics, CR0/XER.SO results, and reservation-clearing stores.

use super::*;

#[test]
fn ldarx_loads_from_memory() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x1008..0x1010].copy_from_slice(&0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Ldarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[5], 0xDEAD_BEEF_CAFE_BABE);
}

#[test]
fn stdcx_with_matching_reservation_emits_conditional_store() {
    let mut s = PpuState::new();
    // Pre-seed the reservation the way a prior ldarx at this line would.
    s.reservation = Some(ReservedLine::containing(0x1008));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.cr_field(0), 0b0010);
    assert!(s.reservation.is_none());
    // stdcx must emit ConditionalStore, never a SharedWriteIntent.
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::ConditionalStore { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x1008);
            assert_eq!(range.length(), 8);
            assert_eq!(bytes.bytes(), &0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
        }
        other => panic!("expected ConditionalStore, got {other:?}"),
    }
}

#[test]
fn stdcx_without_reservation_fails_silently() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.cr_field(0), 0b0000);
    assert!(effects.is_empty());
}

#[test]
fn stwcx_with_reservation_on_different_line_fails() {
    // 128-byte reservation granule: 0x1000 and 0x1080 sit on different lines.
    let mut s = PpuState::new();
    s.reservation = Some(ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x80;
    s.gpr[5] = 0xDEAD_BEEF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.cr_field(0), 0b0000);
    assert!(effects.is_empty());
    // PowerPC ABI: stwcx retires the reservation even on failure.
    assert!(s.reservation.is_none());
}

#[test]
fn same_unit_store_to_reserved_line_clears_local_reservation() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x1000..0x1004].copy_from_slice(&0xdeadbeefu32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x0;
    let mut effects = Vec::new();

    exec_with_mem(
        &PpuInstruction::Lwarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.reservation.map(|l| l.addr()), Some(0x1000));

    s.gpr[6] = 0x1040;
    s.gpr[7] = 0xAAAA_BBBBu64;
    let mut effects2 = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stw {
            rs: 7,
            ra: 6,
            imm: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects2,
    );
    assert!(
        s.reservation.is_none(),
        "same-unit store to reserved line must drop the local reservation"
    );

    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x0;
    s.gpr[5] = 0x5555_6666u64;
    let mut effects3 = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects3,
    );
    assert_eq!(
        s.cr_field(0),
        0b0000,
        "stwcx must fail after self-invalidation"
    );
    assert!(effects3.is_empty());
}

#[test]
fn lwarx_sets_local_reservation_and_emits_acquire() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x1040..0x1044].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x40;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Lwarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[5], 0xDEAD_BEEF);
    // Reservation tracks the enclosing 128-byte line, not the raw EA.
    assert_eq!(
        s.reservation.map(|l| l.addr()),
        Some(0x1000),
        "local reservation must be set to the enclosing line"
    );
    let acquires: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::ReservationAcquire { line_addr, source } => Some((*line_addr, *source)),
            _ => None,
        })
        .collect();
    assert_eq!(acquires, vec![(0x1000, UnitId::new(0))]);
}

#[test]
fn ldarx_sets_local_reservation_and_emits_acquire() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Ldarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.reservation.map(|l| l.addr()), Some(0x1000));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::ReservationAcquire {
            line_addr: 0x1000,
            ..
        }
    )));
}

#[test]
fn stwcx_on_matching_line_retires_local_reservation() {
    let mut s = PpuState::new();
    s.reservation = Some(ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x0;
    s.gpr[5] = 0xCAFE_BABE;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.cr_field(0), 0b0010);
    assert!(
        s.reservation.is_none(),
        "stwcx must retire the local reservation on success"
    );
}

#[test]
fn stdcx_on_matching_line_retires_local_reservation() {
    let mut s = PpuState::new();
    s.reservation = Some(ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x0;
    s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert!(s.reservation.is_none());
}

#[test]
fn ldarx_increments_counter_on_each_execution() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    assert_eq!(s.ldarx_executed, 0);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Ldarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.ldarx_executed, 1);
    s.gpr[4] = 0x10;
    exec_with_mem(
        &PpuInstruction::Ldarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.ldarx_executed, 2);
}

#[test]
fn lwarx_increments_counter_on_each_execution() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x40;
    assert_eq!(s.lwarx_executed, 0);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.lwarx_executed, 1);
    s.gpr[4] = 0x44;
    exec_with_mem(
        &PpuInstruction::Lwarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.lwarx_executed, 2);
}

#[test]
fn stdcx_increments_counter_on_each_execution() {
    let mut s = PpuState::new();
    s.reservation = Some(ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x0;
    s.gpr[5] = 0xCAFE_BABE_DEAD_BEEF;
    assert_eq!(s.stdcx_executed, 0);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.stdcx_executed, 1);
    // Counter counts arm entries, not successful conditional stores.
    exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.stdcx_executed, 2);
}

#[test]
fn stwcx_increments_counter_on_each_execution() {
    let mut s = PpuState::new();
    s.reservation = Some(ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x0;
    s.gpr[5] = 0xCAFE_BABE;
    assert_eq!(s.stwcx_executed, 0);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.stwcx_executed, 1);
    exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.stwcx_executed, 2);
}

#[test]
fn stvx_clears_overlapping_reservation() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.gpr[8] = 0;
    s.vr[0] = 0u128;
    s.reservation = Some(ReservedLine::containing(0x1000));
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stvx {
            vs: 0,
            ra: 1,
            rb: 8,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert!(
        s.reservation.is_none(),
        "stvx covering the reserved line must drop the reservation"
    );
}

#[test]
fn stfd_clears_overlapping_reservation() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.fpr[5] = 0xDEAD_BEEF_CAFE_F00Du64;
    s.reservation = Some(ReservedLine::containing(0x1000));
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stfd {
            frs: 5,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert!(
        s.reservation.is_none(),
        "stfd covering the reserved line must drop the reservation"
    );
}

#[test]
fn stwcx_success_propagates_xer_so_into_cr0() {
    // [PPC-Book2 p:25 s:3.3.2 Atomic Update Primitives] stwcx
    // CR0 = 0b00 || n || XER[SO]. Earlier code zeroed the SO bit
    // unconditionally.
    let mut s = PpuState::new();
    s.reservation = Some(ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0;
    s.gpr[5] = 0xDEAD_BEEF;
    s.set_xer_ov(true); // sets SO sticky
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    // success (0b0010) | SO (0b0001) = 0b0011.
    assert_eq!(s.cr_field(0), 0b0011);
}

#[test]
fn stwcx_failure_propagates_xer_so_into_cr0() {
    let mut s = PpuState::new();
    // No reservation: stwcx fails.
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0;
    s.gpr[5] = 0;
    s.set_xer_ov(true);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    // failure (0b0000) | SO (0b0001) = 0b0001.
    assert_eq!(s.cr_field(0), 0b0001);
}

#[test]
fn stdcx_success_propagates_xer_so_into_cr0() {
    let mut s = PpuState::new();
    s.reservation = Some(ReservedLine::containing(0x1008));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
    s.set_xer_ov(true);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.cr_field(0), 0b0011);
}

#[test]
fn stdcx_failure_propagates_xer_so_into_cr0() {
    // Symmetric to the success test; covers the failure code
    // path which writes a different CR0 value.
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    s.gpr[5] = 0;
    s.set_xer_ov(true);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.cr_field(0), 0b0001);
}

#[test]
fn lwarx_misaligned_ea_raises_alignment_fault() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x1;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Lwarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(
        result,
        ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(0x1001))
    );
    assert!(s.reservation.is_none());
    assert!(effects.is_empty());
}

#[test]
fn ldarx_misaligned_ea_raises_alignment_fault() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x4;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Ldarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(
        result,
        ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(0x1004))
    );
    assert!(s.reservation.is_none());
    assert!(effects.is_empty());
}

#[test]
fn stwcx_sets_cr0_neq_when_reservation_lost() {
    let mut s = PpuState::new();
    // No reservation held -> conditional store fails.
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0;
    s.gpr[5] = 0xAABB_CCDD;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    // LT=0, GT=0, EQ=0, SO=0.
    assert_eq!(s.cr_field(0), 0b0000);
}

#[test]
fn stwcx_always_clears_cr0_lt_and_gt() {
    // Pre-poison CR0 with LT=1, GT=1, EQ=0, SO=0. stwcx must
    // overwrite the whole field, not OR into it.
    let mut s = PpuState::new();
    s.set_cr_field(0, 0b1100);
    s.reservation = Some(ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0;
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    // Success: 0b0010. LT and GT must have been cleared.
    assert_eq!(s.cr_field(0), 0b0010);

    // Same check on the failure path.
    let mut s = PpuState::new();
    s.set_cr_field(0, 0b1100);
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0;
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.cr_field(0), 0b0000);
}

#[test]
fn stwcx_misaligned_ea_raises_alignment_fault() {
    let mut s = PpuState::new();
    s.reservation = Some(ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x2;
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(
        result,
        ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(0x1002))
    );
    // Fault path returns early: CR0 untouched, reservation intact,
    // no effects emitted.
    assert_eq!(s.cr_field(0), 0b0000);
    assert_eq!(s.reservation.map(|l| l.addr()), Some(0x1000));
    assert!(effects.is_empty());
}

#[test]
fn stdcx_clears_reservation_on_failure_too() {
    let mut s = PpuState::new();
    s.reservation = Some(ReservedLine::containing(0x2000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.cr_field(0), 0b0000);
    assert!(s.reservation.is_none());
}

#[test]
fn stdcx_always_clears_cr0_lt_and_gt() {
    let mut s = PpuState::new();
    s.set_cr_field(0, 0b1100);
    s.reservation = Some(ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.cr_field(0), 0b0010);

    let mut s = PpuState::new();
    s.set_cr_field(0, 0b1100);
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.cr_field(0), 0b0000);
}

#[test]
fn stdcx_misaligned_ea_raises_alignment_fault() {
    let mut s = PpuState::new();
    s.reservation = Some(ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x4;
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(
        result,
        ExecuteVerdict::Fault(PpuFault::AlignmentInterrupt(0x1004))
    );
    assert_eq!(s.cr_field(0), 0b0000);
    assert_eq!(s.reservation.map(|l| l.addr()), Some(0x1000));
    assert!(effects.is_empty());
}
