use super::constants::{Constants, Value};
use super::lost::{LostGprs, Origins};
use super::tests::word;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_NOP;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_IR;

/// `mtspr spr,rs`.
///
/// [PPC-Book3 p:19 s:3.4] `mtspr` is primary opcode 31 with extended opcode 467, RS in bits 6:10 and the SPR number in bits 11:20 with its halves swapped.
fn mtspr(spr: u16, rs: u8) -> u32 {
    let spr = u32::from(spr);
    (31 << 26) | (u32::from(rs) << 21) | ((spr & 0x1F) << 16) | ((spr >> 5) << 11) | (467 << 1)
}

/// [PPC-Book3 p:119 s:Appendix G] `rfid` is an XL-form word with primary opcode 19 and extended opcode 18.
const RFID: u32 = (19 << 26) | (18 << 1);

/// An SRR0 constant above 4 GiB whose low-order 32 bits are 0x3004.
const HIGH: u64 = 0x1_0000_3004;

/// The lost marks of the GPRs of the bits of `lost`, with origins that
/// name no drop.
fn marked(lost: u32) -> LostGprs {
    let mut gprs = LostGprs::default();
    for gpr in (0..32u8).filter(|&gpr| lost & (1 << gpr) != 0) {
        gprs.set(gpr, Some(Origins::unnamed()));
    }
    gprs
}

/// The constants after `mtspr SRR0,r3; mtspr SRR1,r4`, with `srr0` in r3,
/// `srr1` in r4, and the GPRs of the bits of `lost` marked as lost.
fn copied(srr0: u64, srr1: u64, lost: u32) -> Constants {
    let mut constants = Constants::default();
    constants.gprs[3] = Some(Value::One(srr0));
    constants.gprs[4] = Some(Value::One(srr1));
    constants.lost_gprs = marked(lost);
    constants.step(0, mtspr(SPR_SRR0, 3));
    constants.step(4, mtspr(SPR_SRR1, 4));
    constants
}

/// [PPC-Book3 p:13 s:2.3] `rfid` places the OR of SRR1 bits 0 and 1 into MSR\[SF\], and fetches from 32 zeros || SRR0 32:61 || 0b00 when SF = 0 in the new MSR.
#[test]
fn an_srr1_copied_from_a_lost_gpr_lets_the_rfid_fetch_with_sf_at_0_or_1() {
    let srr1 = MSR_SF | MSR_IR;
    let kept = copied(HIGH, srr1, 0);
    assert_eq!(kept.translated_rfid_targets(RFID), BTreeSet::from([HIGH]));
    let lost = copied(HIGH, srr1, 1 << 4);
    assert_eq!(
        lost.translated_rfid_targets(RFID),
        BTreeSet::from([0x3004, HIGH])
    );
}

/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\].
#[test]
fn an_rfid_is_lost_only_with_a_lost_srr0_and_an_srr1_that_can_set_ir() {
    let lost = |srr1, gprs, raw| copied(HIGH, srr1, gprs).translated_rfid_lost(raw).is_some();
    assert!(lost(MSR_IR, 1 << 3, RFID));
    assert!(lost(0, (1 << 3) | (1 << 4), RFID));
    assert!(!lost(MSR_IR, 0, RFID));
    assert!(!lost(MSR_IR, 1 << 4, RFID));
    assert!(!lost(0, 1 << 3, RFID));
    assert!(!lost(MSR_IR, 1 << 3, PPC_NOP));
}

/// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR, so the routine that a call runs can leave SRR0 unchanged.
#[test]
fn a_call_keeps_the_lost_mark_of_srr0() {
    let mut constants = copied(HIGH, MSR_IR, 1 << 3);
    constants.step(
        8,
        word(PpuInstruction::B {
            offset: 8,
            aa: false,
            link: true,
        }),
    );
    assert!(constants.translated_rfid_lost(RFID).is_some());
}

/// `mfspr rt,spr`.
///
/// [PPC-Book3 p:21 s:3.4] `mfspr` is primary opcode 31 with extended opcode 339, RT in bits 6:10 and the SPR number in bits 11:20 with its halves swapped.
fn mfspr(rt: u8, spr: u16) -> u32 {
    let spr = u32::from(spr);
    (31 << 26) | (u32::from(rt) << 21) | ((spr & 0x1F) << 16) | ((spr >> 5) << 11) | (339 << 1)
}

/// [PPC-Book3 p:21 s:3.4] `mfspr` places the SPR that its SPR field names into RT.
#[test]
fn an_mfspr_of_a_lost_srr0_or_srr1_marks_its_gpr_as_lost() {
    let mut constants = copied(HIGH, MSR_IR, (1 << 3) | (1 << 4));
    constants.lost_gprs = LostGprs::default();
    constants.step(8, mfspr(5, SPR_SRR0));
    constants.step(12, mfspr(6, SPR_SRR1));
    assert!(constants.is_lost(5));
    assert!(constants.is_lost(6));
    let mut kept = copied(HIGH, MSR_IR, 0);
    kept.lost_gprs = marked(1 << 5);
    kept.step(8, mfspr(5, SPR_SRR0));
    assert!(!kept.is_lost(5));
}
