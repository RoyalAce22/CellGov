//! The stop record's code, resume address and `SPU_Status` word.

use super::*;
use cellgov_ps3_abi::hw::spu::SPU_LSLR_FULL;

/// [CBEA p:94 s:8.5.2] a stop copies the low 14 bits of the instruction to StopCode bits 2:15 and sets P.
#[test]
fn a_stop_keeps_its_14_bit_code_and_sets_p() {
    let stop = SpuStop::new(SpuStopKind::Stop, 0x3102, 0x100, SPU_LSLR_FULL);
    assert_eq!(stop.code, 0x3102);
    assert_eq!(stop.status_word(), 0x3102_0002);
    // [CBEA p:93 s:8.5.2] StopCode bits 0 and 1 are always zero.
    assert_eq!(
        SpuStop::new(SpuStopKind::Stop, 0xFFFF, 0x100, SPU_LSLR_FULL).status_word(),
        0x3FFF_0002
    );
}

/// [CBEA p:93 s:8.5.2] a stopd always sets StopCode to x'3FFF', whatever its fields hold.
#[test]
fn a_stopd_reports_the_breakpoint_code() {
    let stop = SpuStop::new(SpuStopKind::Stopd, 0x0123, 0x100, SPU_LSLR_FULL);
    assert_eq!(stop.code, 0x3FFF);
    assert_eq!(stop.status_word(), 0x3FFF_0002);
}

/// [CBEA p:93 s:8.5.2] C is bit 25 and I bit 26; [CBEA p:94 s:8.5.2] H is bit 29, and StopCode is valid only with P.
#[test]
fn a_halt_or_an_spu_error_sets_only_its_own_bit() {
    for (kind, bit) in [
        (SpuStopKind::Halt, 0x04),
        (SpuStopKind::InvalidInstruction, 0x20),
        (SpuStopKind::InvalidChannel, 0x40),
    ] {
        let stop = SpuStop::new(kind, 0x3FFF, 0x100, SPU_LSLR_FULL);
        assert_eq!(stop.code, 0, "{kind:?}");
        assert_eq!(stop.status_word(), bit, "{kind:?}");
    }
}

/// [SPU-ISA p:238 s:10] stop: PC <- PC + 4 & LSLR.
#[test]
fn the_resume_address_is_the_next_word_under_the_limit() {
    assert_eq!(
        SpuStop::new(SpuStopKind::Stop, 0, 0x100, SPU_LSLR_FULL).npc,
        0x104
    );
    assert_eq!(
        SpuStop::new(SpuStopKind::Stop, 0, 0x3_FFFC, SPU_LSLR_FULL).npc,
        0
    );
    assert_eq!(SpuStop::new(SpuStopKind::Stop, 0, 0x7FFC, 0x7FFF).npc, 0);
}
