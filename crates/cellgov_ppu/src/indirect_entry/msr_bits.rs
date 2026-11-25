//! The bits of an MSR value that the scan knows when it keeps no constant
//! of the value: the bits that are 1, and the bits that hold the MSR of
//! the word.

use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_BIT1, MSR_IR, MSR_PR, MSR_SF};

use super::real_mode::MsrWord;
use crate::instruction::PpuInstruction;

/// The MSR bits that set MSR\[IR\] when a word writes MSR.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\].
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\].
const IR_SOURCES: u64 = MSR_IR | MSR_PR;

/// The MSR bits that set MSR\[SF\] when a word writes MSR.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` places the OR of SRR1 bits 0 and 1 into MSR\[SF\].
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[SF\] to RS bit 0 | RS bit 1.
const SF_SOURCES: u64 = MSR_SF | MSR_BIT1;

/// What the scan knows of the bits of one value.
///
/// The default value knows no bit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct MsrBits {
    /// The bits that are 1.
    ones: u64,
    /// The bits that are equal to the bits of MSR at the word.
    kept: u64,
}

impl MsrBits {
    /// Whether a word that writes MSR from the value leaves MSR\[IR\] at 1.
    pub(super) fn sets_ir(self) -> bool {
        self.ones & IR_SOURCES != 0
    }

    /// Whether a word that writes MSR from the value leaves MSR\[SF\] at 1.
    pub(super) fn sets_sf(self) -> bool {
        self.ones & SF_SOURCES != 0
    }

    /// Whether a word that writes MSR from the value leaves MSR\[IR\] as
    /// it was.
    ///
    /// The value holds MSR\[IR\] and MSR\[PR\]. Each word that writes MSR
    /// sets MSR\[IR\] from the OR of both bits, and an interrupt sets both
    /// bits to 0. So MSR\[PR\] at 1 comes only with MSR\[IR\] at 1, and the
    /// OR of the two bits is MSR\[IR\].
    ///
    /// [PPC-Book3 p:65 s:5.5] Figure 35: each interrupt sets MSR\[IR\] to 0, and sets MSR\[PR\] to 0.
    pub(super) fn keeps_ir(self) -> bool {
        self.kept & IR_SOURCES == IR_SOURCES
    }

    /// Whether a word that writes MSR from the value leaves MSR\[SF\] as
    /// it was.
    ///
    /// The value holds MSR\[SF\] and MSR bit 1. Each word that writes MSR
    /// sets MSR\[SF\] from the OR of both bits, and an interrupt sets
    /// MSR\[SF\] to 1. So MSR bit 1 at 1 comes only with MSR\[SF\] at 1.
    ///
    /// [PPC-Book3 p:13 s:2.3] `rfid` places the OR of SRR1 bits 0 and 1 into MSR\[SF\], and SRR1 bit 1 into MSR bit 1.
    /// [PPC-Book3 p:65 s:5.5] Each interrupt sets MSR\[SF\] to 1.
    pub(super) fn keeps_sf(self) -> bool {
        self.kept & SF_SOURCES == SF_SOURCES
    }
}

/// The [`MsrBits`] of each GPR and of SRR1 on one path.
///
/// The scan takes an interrupt and its return to leave MSR and every
/// register as they were, as it does for each constant that it keeps
/// across a word.
#[derive(Debug, Clone, Default)]
pub(super) struct MsrValues {
    gprs: [MsrBits; 32],
    srr1: MsrBits,
}

impl MsrValues {
    /// The bits of `gpr`.
    pub(super) fn gpr(&self, gpr: u8) -> MsrBits {
        self.gprs.get(usize::from(gpr)).copied().unwrap_or_default()
    }

    /// The bits of SRR1.
    pub(super) fn srr1(&self) -> MsrBits {
        self.srr1
    }

    /// Forgets the bits of `gpr`, which can now hold a value of another
    /// path.
    pub(super) fn forget_gpr(&mut self, gpr: u8) {
        if let Some(bits) = self.gprs.get_mut(usize::from(gpr)) {
            *bits = MsrBits::default();
        }
    }

    /// Steps over the word `raw`, which decodes as `decoded` and writes
    /// the GPRs of the mask `written`.
    ///
    /// `constant` gives the one constant of a GPR before the word, when
    /// the scan keeps exactly one.
    ///
    /// These words give their result known bits:
    /// - `mfmsr` gives a value whose every bit is a bit of MSR;
    /// - `ori` and `oris` set the bits of their immediate to 1;
    /// - `or` sets the bits of a constant source to 1, and copies a
    ///   source that it ORs with itself;
    /// - `andc` with a constant second source clears its bits.
    ///
    /// A word that writes MSR, a call and an `sc` can change MSR, so each
    /// forgets which bits hold MSR. A call and an `sc` can also write each
    /// GPR and SRR1.
    ///
    /// [PPC-Book3 p:23 s:3.4] `mfmsr` places the MSR into RT.
    /// [PPC-Book1 p:66 s:3.3.11] `ori` ORs RS with 0x0000 || UI, and `oris` ORs RS with UI || 0x0000, into RA.
    /// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB into RA.
    /// [PPC-Book1 p:68 s:3.3.11] `andc` ANDs RS with the complement of RB into RA.
    pub(super) fn step(
        &mut self,
        raw: u32,
        decoded: &PpuInstruction,
        constant: impl Fn(u8) -> Option<u64>,
        written: u32,
    ) {
        if changes_every_register(decoded) {
            *self = Self::default();
            return;
        }
        let result = self.result(raw, decoded, &constant);
        match MsrWord::of(raw) {
            Some(MsrWord::MtSrr1 { rs }) => self.srr1 = self.gpr(rs),
            Some(
                MsrWord::Rfid | MsrWord::Hrfid | MsrWord::Mtmsrd { .. } | MsrWord::Mtmsr { .. },
            ) => {
                self.forget_kept();
            }
            _ => {}
        }
        for gpr in (0..32u8).filter(|&gpr| written & (1 << gpr) != 0) {
            self.forget_gpr(gpr);
        }
        if let Some((rt, bits)) = result {
            self.gprs[usize::from(rt)] = bits;
        }
    }

    /// The GPR that `decoded` writes with known bits, and those bits.
    fn result(
        &self,
        raw: u32,
        decoded: &PpuInstruction,
        constant: &impl Fn(u8) -> Option<u64>,
    ) -> Option<(u8, MsrBits)> {
        if is_mfmsr(raw) {
            let rt = ((raw >> 21) & 0x1F) as u8;
            let every = MsrBits {
                ones: 0,
                kept: u64::MAX,
            };
            return Some((rt, every));
        }
        let ored = |source: MsrBits, bits: u64| MsrBits {
            ones: source.ones | bits,
            kept: source.kept & !bits,
        };
        match *decoded {
            PpuInstruction::Ori { ra, rs, imm } => Some((ra, ored(self.gpr(rs), u64::from(imm)))),
            PpuInstruction::Oris { ra, rs, imm } => {
                Some((ra, ored(self.gpr(rs), u64::from(imm) << 16)))
            }
            PpuInstruction::Or { ra, rs, rb, .. } if rs == rb => Some((ra, self.gpr(rs))),
            PpuInstruction::Or { ra, rs, rb, .. } => {
                let one_constant = |source: u8, other: u8| {
                    constant(other).map(|bits| ored(self.gpr(source), bits))
                };
                one_constant(rs, rb)
                    .or_else(|| one_constant(rb, rs))
                    .map(|bits| (ra, bits))
            }
            PpuInstruction::Andc { ra, rs, rb, .. } => constant(rb).map(|bits| {
                let source = self.gpr(rs);
                let cleared = MsrBits {
                    ones: source.ones & !bits,
                    kept: source.kept & !bits,
                };
                (ra, cleared)
            }),
            _ => None,
        }
    }

    /// Forgets which bits of each value hold MSR.
    fn forget_kept(&mut self) {
        for bits in self.gprs.iter_mut().chain([&mut self.srr1]) {
            bits.kept = 0;
        }
    }
}

/// Whether `decoded` is a call or an `sc`, after which a GPR, SRR1 or MSR
/// can hold a value that the path does not give.
pub(super) fn changes_every_register(decoded: &PpuInstruction) -> bool {
    matches!(
        *decoded,
        PpuInstruction::Sc { .. }
            | PpuInstruction::B { link: true, .. }
            | PpuInstruction::Bc { link: true, .. }
            | PpuInstruction::Bclr { link: true, .. }
            | PpuInstruction::Bcctr { link: true, .. }
    )
}

/// Whether the word `raw` is `mfmsr`.
///
/// [PPC-Book3 p:119 s:Appendix G] `mfmsr` is an X-form word with primary opcode 31 and extended opcode 83.
fn is_mfmsr(raw: u32) -> bool {
    raw >> 26 == 31 && (raw >> 1) & 0x3FF == 83
}
