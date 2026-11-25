//! Which bits of SRR0 and SRR1 hold the interrupted context on one path,
//! as a context restore before an `rfid` gives them.
//!
//! A bit holds the interrupted context in one of these cases:
//! - a load from storage gave it;
//! - no word that the scan steps on the path wrote it, so it holds what
//!   the interrupt saved.
//!
//! The scan follows the path back through each join that it reaches by a
//! fall-through. It takes each path that enters a join by a branch to
//! leave SRR0 and SRR1 as the interrupt saved them.
//!
//! The census takes an `rfid` to resume an interrupted word in the mode
//! of that word when these hold the interrupted context:
//! - each bit of SRR0;
//! - MSR\[IR\] and MSR\[PR\] of SRR1.
//!
//! The census already takes an interrupt and its return to leave
//! MSR\[IR\] as it was, and this premise applies the same rule to a frame
//! restore and to the return of a handler. It misses code that writes
//! SRR1 with MSR\[IR\] at 0 where the scan does not see the write:
//! - a context frame that the kernel builds itself;
//! - a write on a path that enters a join before the `rfid` by a branch.

use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_IR, MSR_PR, SPR_SRR0, SPR_SRR1};

use super::msr_bits::changes_every_register;
use super::real_mode::MsrWord;
use crate::instruction::{spr_number, PpuInstruction};

/// The bits of one GPR that hold the interrupted SRR0, and the bits that
/// hold the interrupted SRR1.
///
/// A copy of SRR0 holds no bit of the interrupted SRR1, and a copy of
/// SRR1 holds no bit of the interrupted SRR0. So a move of such a copy
/// into the other register writes that register.
///
/// [PPC-Book3 p:21 s:3.4] `mfspr` places the SPR that its SPR field names into RT.
/// [PPC-Book3 p:19 s:3.4] `mtspr` copies RS into the SPR that its SPR field names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct GprBits {
    srr0: u64,
    srr1: u64,
}

impl GprBits {
    /// No bit holds the interrupted context.
    const NONE: Self = Self { srr0: 0, srr1: 0 };

    /// A value from storage, whose every bit holds the interrupted SRR0
    /// or the interrupted SRR1.
    const LOADED: Self = Self {
        srr0: u64::MAX,
        srr1: u64::MAX,
    };

    /// The bits of `self` that the mask `keep` holds.
    fn kept(self, keep: u64) -> Self {
        Self {
            srr0: self.srr0 & keep,
            srr1: self.srr1 & keep,
        }
    }
}

/// The bits on one path that hold the interrupted context, in each GPR,
/// in SRR0 and in SRR1.
///
/// The default value is the value where control enters only by a branch:
/// each bit of SRR0 and SRR1 holds what the interrupt saved, and no GPR
/// bit holds the context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SrrRestore {
    gprs: [GprBits; 32],
    srr0: u64,
    srr1: u64,
}

impl Default for SrrRestore {
    fn default() -> Self {
        Self {
            gprs: [GprBits::NONE; 32],
            srr0: u64::MAX,
            srr1: u64::MAX,
        }
    }
}

impl SrrRestore {
    /// The value after a word that can write each GPR, SRR0 and SRR1:
    /// no bit holds the interrupted context.
    pub(super) fn forgotten() -> Self {
        Self {
            gprs: [GprBits::NONE; 32],
            srr0: 0,
            srr1: 0,
        }
    }

    /// The value at a join that the path of `self` reaches by a
    /// fall-through.
    ///
    /// The census takes each other path into the join to leave SRR0 and
    /// SRR1 as the interrupt saved them. So the join keeps the SRR bits of
    /// the fall-through path, and no GPR bit.
    pub(super) fn at_join(self) -> Self {
        Self {
            gprs: [GprBits::NONE; 32],
            ..self
        }
    }

    /// Whether each bit of SRR0, and MSR\[IR\] and MSR\[PR\] of SRR1,
    /// hold the interrupted context.
    ///
    /// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0.
    pub(super) fn restores_context(self) -> bool {
        let mode = MSR_IR | MSR_PR;
        self.srr0 == u64::MAX && self.srr1 & mode == mode
    }

    /// Steps over the word `raw`, which decodes as `decoded` and writes
    /// the GPRs of the mask `written`.
    ///
    /// `constant` gives the one constant of a GPR before the word, when
    /// the scan keeps exactly one.
    ///
    /// A call and an `sc` can write each GPR, SRR0 and SRR1, so each
    /// forgets every bit.
    ///
    /// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt sets SRR0 and SRR1.
    /// [PPC-Book3 p:19 s:3.4] `mtspr` copies RS into the SPR that its SPR field names.
    pub(super) fn step(
        &mut self,
        raw: u32,
        decoded: &PpuInstruction,
        constant: impl Fn(u8) -> Option<u64>,
        written: u32,
    ) {
        if changes_every_register(decoded) {
            *self = Self::forgotten();
            return;
        }
        match MsrWord::of(raw) {
            Some(MsrWord::MtSrr0 { rs }) => self.srr0 = self.gpr(rs).srr0,
            Some(MsrWord::MtSrr1 { rs }) => self.srr1 = self.gpr(rs).srr1,
            _ => {}
        }
        let result = self
            .srr_copy(raw)
            .or_else(|| self.result(decoded, constant));
        for gpr in (0..32u8).filter(|&gpr| written & (1 << gpr) != 0) {
            self.gprs[usize::from(gpr)] = GprBits::NONE;
        }
        if let Some((rt, bits)) = result {
            self.gprs[usize::from(rt)] = bits;
        }
    }

    /// The bits of `gpr` that hold the interrupted context.
    fn gpr(&self, gpr: u8) -> GprBits {
        self.gprs.get(usize::from(gpr)).copied().unwrap_or_default()
    }

    /// The GPR that the word `raw` copies SRR0 or SRR1 into, and the bits
    /// of that register that hold the interrupted context.
    ///
    /// [PPC-Book3 p:21 s:3.4] `mfspr` is primary opcode 31 with extended opcode 339, and it places the SPR that its SPR field names into RT.
    fn srr_copy(&self, raw: u32) -> Option<(u8, GprBits)> {
        if (raw >> 26, (raw >> 1) & 0x3FF) != (31, 339) {
            return None;
        }
        let bits = match spr_number(raw) {
            SPR_SRR0 => GprBits {
                srr0: self.srr0,
                srr1: 0,
            },
            SPR_SRR1 => GprBits {
                srr0: 0,
                srr1: self.srr1,
            },
            _ => return None,
        };
        Some((((raw >> 21) & 0x1F) as u8, bits))
    }

    /// The GPR that `decoded` writes with bits that hold the interrupted
    /// context, and those bits.
    ///
    /// These words give such bits:
    /// - a doubleword load, and a word load that fills the high word with
    ///   0, give each bit of RT, as a bit of SRR0 or of SRR1;
    /// - `ori` and `oris` keep the bits of RS outside their immediate;
    /// - `mr` keeps each bit of RS;
    /// - `andc` with a constant second source keeps the bits of RS
    ///   outside that constant;
    /// - `rlwimi` keeps the bits of RA outside its mask.
    ///
    /// [PPC-Book1 p:37 s:3.3.2] `lwz`, `lwzx`, `lwzu` and `lwzux` load a word from storage into RT bits 32:63 and set RT bits 0:31 to 0.
    /// [PPC-Book1 p:39 s:3.3.2] `ld`, `ldx`, `ldu` and `ldux` load a doubleword from storage into RT.
    /// [PPC-Book1 p:66 s:3.3.11] `ori` and `oris` OR RS with UI or UI || 0x0000 and place the result into RA.
    /// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
    /// [PPC-Book1 p:68 s:3.3.11] `andc` ANDs RS with the complement of RB into RA.
    /// [PPC-Book1 p:76 s:3.3.12.1] `rlwimi` inserts the rotated RS into RA under MASK(MB + 32, ME + 32) and leaves the other bits of RA unchanged.
    fn result(
        &self,
        decoded: &PpuInstruction,
        constant: impl Fn(u8) -> Option<u64>,
    ) -> Option<(u8, GprBits)> {
        match *decoded {
            PpuInstruction::Ld { rt, .. }
            | PpuInstruction::Ldx { rt, .. }
            | PpuInstruction::Ldu { rt, .. }
            | PpuInstruction::Ldux { rt, .. }
            | PpuInstruction::Lwz { rt, .. }
            | PpuInstruction::Lwzx { rt, .. }
            | PpuInstruction::Lwzu { rt, .. }
            | PpuInstruction::Lwzux { rt, .. } => Some((rt, GprBits::LOADED)),
            PpuInstruction::Ori { ra, rs, imm } => Some((ra, self.gpr(rs).kept(!u64::from(imm)))),
            PpuInstruction::Oris { ra, rs, imm } => {
                Some((ra, self.gpr(rs).kept(!(u64::from(imm) << 16))))
            }
            PpuInstruction::Mr { ra, rs } => Some((ra, self.gpr(rs))),
            PpuInstruction::Or { ra, rs, rb, .. } if rs == rb => Some((ra, self.gpr(rs))),
            PpuInstruction::Andc { ra, rs, rb, .. } => {
                constant(rb).map(|cleared| (ra, self.gpr(rs).kept(!cleared)))
            }
            PpuInstruction::Rlwimi { ra, mb, me, .. } => {
                Some((ra, self.gpr(ra).kept(!word_mask(mb, me))))
            }
            _ => None,
        }
    }
}

/// MASK(`mb` + 32, `me` + 32): the bits that an `rlwimi` with these
/// fields inserts.
///
/// [PPC-Book1 p:76 s:3.3.12.1] `rlwimi` inserts the rotated RS into RA under MASK(MB + 32, ME + 32).
fn word_mask(mb: u8, me: u8) -> u64 {
    let (begin, end) = (u32::from(mb) + 32, u32::from(me) + 32);
    let from_begin = u64::MAX >> begin;
    let to_end = u64::MAX << (63 - end);
    if begin <= end {
        from_begin & to_end
    } else {
        from_begin | to_end
    }
}
