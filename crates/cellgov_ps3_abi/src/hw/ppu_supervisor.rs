//! The PPE registers that only privileged code reads and writes: the MSR
//! bits, the segment lookaside buffer, and the SPR numbers of the
//! supervisor registers.
//!
//! The bit numbers are the ones the ISA prints: bit 0 is the most
//! significant bit of the 64-bit register.

use super::ppu_interrupt::doubleword_bit;

/// MSR bit 0: 64-bit mode.
///
/// [PPC-Book3 p:10 s:2.2.3] MSR bit 0 is SF, bit 3 HV, bit 48 EE, bit 49 PR, bit 50 FP, bit 51 ME, bit 58 IR and bit 59 DR.
pub const MSR_SF: u64 = doubleword_bit(0);

/// MSR bit 1, which `mtmsrd`, `rfid` and `hrfid` OR into SF.
///
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 places the OR of RS bits 0 and 1 into MSR bit 0.
pub const MSR_BIT1: u64 = doubleword_bit(1);

/// MSR bit 3: hypervisor state.
///
/// [PPC-Book3 p:10 s:2.2.3] See [`MSR_SF`].
pub const MSR_HV: u64 = doubleword_bit(3);

/// MSR bit 48: external and decrementer interrupts enabled.
///
/// [PPC-Book3 p:10 s:2.2.3] See [`MSR_SF`].
pub const MSR_EE: u64 = doubleword_bit(48);

/// MSR bit 49: problem state.
///
/// [PPC-Book3 p:10 s:2.2.3] See [`MSR_SF`].
pub const MSR_PR: u64 = doubleword_bit(49);

/// MSR bit 50: floating-point instructions available.
///
/// [PPC-Book3 p:10 s:2.2.3] See [`MSR_SF`].
pub const MSR_FP: u64 = doubleword_bit(50);

/// MSR bit 51: machine check interrupts enabled.
///
/// [PPC-Book3 p:10 s:2.2.3] See [`MSR_SF`].
pub const MSR_ME: u64 = doubleword_bit(51);

/// MSR bit 52: floating-point exception mode 0.
///
/// [PPC-Book3 p:10 s:2.2.3] MSR bit 52 is FE0, bit 53 SE, bit 54 BE and bit 55 FE1.
pub const MSR_FE0: u64 = doubleword_bit(52);

/// MSR bit 53: single-step trace enabled.
///
/// [PPC-Book3 p:10 s:2.2.3] See [`MSR_FE0`].
pub const MSR_SE: u64 = doubleword_bit(53);

/// MSR bit 54: branch trace enabled.
///
/// [PPC-Book3 p:10 s:2.2.3] See [`MSR_FE0`].
pub const MSR_BE: u64 = doubleword_bit(54);

/// MSR bit 55: floating-point exception mode 1.
///
/// [PPC-Book3 p:10 s:2.2.3] See [`MSR_FE0`].
pub const MSR_FE1: u64 = doubleword_bit(55);

/// MSR bit 58: instruction address translation enabled.
///
/// [PPC-Book3 p:10 s:2.2.3] See [`MSR_SF`].
pub const MSR_IR: u64 = doubleword_bit(58);

/// MSR bit 59: data address translation enabled.
///
/// [PPC-Book3 p:10 s:2.2.3] See [`MSR_SF`].
pub const MSR_DR: u64 = doubleword_bit(59);

/// MSR bit 61: the performance monitor mark.
///
/// [PPC-Book3 p:108 s:E.1] MSR bit 61 is PMM, the performance monitor mark.
/// [CBE-Handbook p:304 s:10.4.1] The PPE's MSR has PMM at bit 61.
pub const MSR_PMM: u64 = doubleword_bit(61);

/// MSR bit 62: the interrupt is recoverable.
///
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 1 writes MSR bits 48 and 62 and leaves the rest.
pub const MSR_RI: u64 = doubleword_bit(62);

/// The MSR bits that every interrupt clears.
///
/// [CBE-Handbook p:292 s:9.12] Table 9-40: an interrupt clears BE, EE, FE0, FE1, FP, PR, SE, DR, IR and RI, sets SF, keeps ME, and keeps HV when LPCR\[LPES\] bits 0 and 1 are 1.
/// [PPC-Book3 p:65 s:5.5] Figure 35: every interrupt also sets PMM to 0, which Table 9-40 does not list.
pub const MSR_INTERRUPT_CLEARED: u64 = MSR_BE
    | MSR_EE
    | MSR_FE0
    | MSR_FE1
    | MSR_FP
    | MSR_PR
    | MSR_SE
    | MSR_DR
    | MSR_IR
    | MSR_RI
    | MSR_PMM;

/// Entries in each PPE hardware thread's SLB.
///
/// [CBE-Handbook p:79 s:4.2.1] Table 4-1: the PPE SLB has 64 entries per thread.
pub const PPE_SLB_ENTRIES: usize = 64;

/// The ESID field of an SLB entry's effective half: bits 0:35.
///
/// [PPC-Book3 p:51 s:4.11.3.1] `slbmte` takes the ESID from RB bits 0:35, the valid bit from RB bit 36 and the entry index from RB bits 52:63.
pub const SLB_ESID: u64 = !0 << (63 - 35);

/// The valid bit of an SLB entry's effective half: bit 36.
///
/// [PPC-Book3 p:51 s:4.11.3.1] See [`SLB_ESID`].
pub const SLB_VALID: u64 = doubleword_bit(36);

/// The entry index an SLB instruction's RB names: bits 52:63.
///
/// [PPC-Book3 p:51 s:4.11.3.1] See [`SLB_ESID`].
pub const SLB_INDEX: u64 = 0xFFF;

/// The fields of an SLB entry's virtual half: the VSID in bits 0:51 and
/// Ks, Kp, N, L and C in bits 52:56.
///
/// [PPC-Book3 p:51 s:4.11.3.1] `slbmte` loads the VSID and Ks, Kp, N, L and C from RS bits 0:56; bits 57:63 must be 0.
pub const SLB_VSID_FLAGS: u64 = !0 << (63 - 56);

/// The entry index bits of RB that the PPE implements: bits 58:63.
///
/// [CBE-Handbook p:84 s:4.2.5.2] The PPE does not implement index bits 52:57 or VSID bits 0:14 in any SLB instruction: a write ignores them and a read returns 0.
pub const PPE_SLB_INDEX: u64 = 0x3F;

/// The LP bit that the PPE adds to an SLB entry's virtual half: bit 59.
///
/// [CBE-Handbook p:84 s:4.2.5.2] The PPE adds LP to the RS of `slbmte` and the RT of `slbmfev` as bits 57:59, and implements only bit 59.
pub const PPE_SLB_LP: u64 = doubleword_bit(59);

/// The bits of an SLB entry's virtual half that the PPE keeps: VSID bits
/// 15:51, Ks, Kp, N, L, C and LP. See [`PPE_SLB_INDEX`] and [`PPE_SLB_LP`].
pub const PPE_SLB_VSID_FLAGS: u64 = (SLB_VSID_FLAGS & (!0u64 >> 15)) | PPE_SLB_LP;

/// SPR number of DSISR.
///
/// [PPC-Book3 p:21 s:3.4.2] Figure 18 numbers DSISR 18, DAR 19, DEC 22, SRR0 26, SRR1 27 and SPRG0-3 272-275.
pub const SPR_DSISR: u16 = 18;

/// SPR number of DAR. See [`SPR_DSISR`].
pub const SPR_DAR: u16 = 19;

/// SPR number of the decrementer. See [`SPR_DSISR`].
pub const SPR_DEC: u16 = 22;

/// SPR number of SRR0. See [`SPR_DSISR`].
pub const SPR_SRR0: u16 = 26;

/// SPR number of SRR1. See [`SPR_DSISR`].
pub const SPR_SRR1: u16 = 27;

/// SPR number of SPRG0; SPRG`k` is `SPR_SPRG0 + k`. See [`SPR_DSISR`].
pub const SPR_SPRG0: u16 = 272;

#[cfg(test)]
#[path = "tests/ppu_supervisor_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/ppu_msr_trace_fp_bits_tests.rs"]
mod msr_trace_fp_bits_tests;

#[cfg(test)]
#[path = "tests/ppu_msr_pmm_bit_tests.rs"]
mod msr_pmm_bit_tests;
