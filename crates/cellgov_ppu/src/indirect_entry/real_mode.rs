//! The words where control enters in real mode, and the words where an
//! `rfid`, `mtmsr` or `mtmsrd` can put control into real mode.

use std::collections::BTreeSet;

use cellgov_ps3_abi::format::elf::ELF64_E_ENTRY;
use cellgov_ps3_abi::hw::ppu_interrupt::INTERRUPT_VECTOR_REAL_ADDRESSES;
use cellgov_ps3_abi::hw::ppu_mmu::REAL_MODE_IGNORED_EA;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_BIT1, MSR_SF, SPR_SRR0, SPR_SRR1};

use super::bound::IndirectBound;
use super::constants::{Constants, Value};
use super::toc::{FunctionSteps, WordTocs};
use crate::hypercall_cfg::{low_indirect_words, real_indirect_words, Code, CodeSegment};
use crate::instruction::spr_number;

/// The real-mode switches and the 32-bit switches of the code.
#[derive(Debug, Default)]
pub(super) struct Switches {
    /// The [`IndirectBound::real_mode_switches`].
    ///
    /// [`IndirectBound::real_mode_switches`]: super::IndirectBound::real_mode_switches
    pub(super) targets: BTreeSet<u64>,
    /// The [`IndirectBound::unbounded_switches`].
    ///
    /// [`IndirectBound::unbounded_switches`]: super::IndirectBound::unbounded_switches
    pub(super) unbounded: BTreeSet<u64>,
    /// The [`IndirectBound::mode32_switches`].
    ///
    /// [`IndirectBound::mode32_switches`]: super::IndirectBound::mode32_switches
    pub(super) mode32: BTreeSet<u64>,
    /// The [`IndirectBound::unbounded_mode32_switches`].
    ///
    /// [`IndirectBound::unbounded_mode32_switches`]: super::IndirectBound::unbounded_mode32_switches
    pub(super) unbounded_mode32: BTreeSet<u64>,
}

impl Switches {
    /// The switches of `code`, from the constants that each path to a
    /// switch builds.
    ///
    /// `tocs` holds the TOC of each word, and `every` holds each TOC that
    /// a descriptor gives. `bound` holds the bound without its switches.
    ///
    /// A real-mode switch target is also a word that control enters other
    /// than by a fall-through. So the scan repeats until no real-mode
    /// switch adds a target.
    /// The targets are words of the code, so the repeats end.
    pub(super) fn find(code: &Code, tocs: &WordTocs, every: &Value, bound: &IndirectBound) -> Self {
        let mut joins = joins(code, bound);
        loop {
            let switches = Self::scan(code, tocs, every, &joins);
            let before = joins.len();
            joins.extend(
                switches
                    .targets
                    .iter()
                    .flat_map(|&target| code.real_aliases(target)),
            );
            if joins.len() == before {
                return switches;
            }
        }
    }

    /// The switches of `code`, when control enters each word of `joins`
    /// with no constant that the scan knows.
    fn scan(code: &Code, tocs: &WordTocs, every: &Value, joins: &BTreeSet<u64>) -> Self {
        let mut switches = Self::default();
        for (index, segment) in code.segments().iter().enumerate() {
            let mut constants = Constants::default();
            for ((address, raw), &toc) in segment.addressed().zip(tocs.segment(index)) {
                if joins.contains(&address) {
                    constants = Constants::default();
                }
                constants.gprs[2] = toc.in_r2(constants.gprs[2].take(), every);
                match constants.real_mode_switch(address, raw) {
                    Switch::None => {}
                    Switch::To(addresses) => switches.targets.extend(addresses),
                    Switch::Unbounded => {
                        switches.unbounded.insert(address);
                    }
                }
                switches.add_mode32(code, address, &constants, raw);
                constants.step(address, raw);
            }
        }
        switches
    }

    /// Adds where the word `raw` at `address` can put control into
    /// 32-bit mode, with the `constants` of the paths to it.
    ///
    /// With MSR\[IR\] at 1 the word at the fetch address runs. With
    /// MSR\[IR\] at 0 each word at its real address runs.
    fn add_mode32(&mut self, code: &Code, address: u64, constants: &Constants, raw: u32) {
        for ir in [true, false] {
            match constants.mode32_switch(address, raw, ir) {
                Switch::None => {}
                Switch::To(addresses) if ir => self
                    .mode32
                    .extend(addresses.into_iter().filter(|&next| code.holds(next))),
                Switch::To(addresses) => self.mode32.extend(
                    addresses
                        .into_iter()
                        .flat_map(|next| code.real_aliases(next)),
                ),
                Switch::Unbounded => {
                    self.unbounded_mode32.insert(address);
                }
            }
        }
    }
}

/// The words of `code` that control can enter other than by a
/// fall-through, apart from the switch targets:
/// - each entry of `bound`;
/// - each target of a direct branch;
/// - each target that `bound` gives;
/// - each word of [`IndirectBound::real_named`];
/// - the word after each branch that links, where a `bclr` can return
///   with other registers.
///
/// Real mode ignores EA bits 0 and 1, so the set also holds each word at
/// the real address of each of these. Apart from the direct targets,
/// these are the words that [`real_indirect_words`] gives. The set also
/// holds each word that [`low_indirect_words`] gives, which a `bcctr` or
/// `bclr` in 32-bit mode can enter. The set omits the word after a
/// direct branch as a target of that branch. The branch reaches that
/// word with the constants of the fall-through. A branch that links
/// still makes that word a join, as a return point.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
fn joins(code: &Code, bound: &IndirectBound) -> BTreeSet<u64> {
    let direct = code
        .segments()
        .iter()
        .flat_map(CodeSegment::addressed)
        .flat_map(|(address, raw)| {
            let next = address.checked_add(4);
            FunctionSteps::of(address, raw)
                .target
                .into_iter()
                .flat_map(|target| code.real_aliases(target))
                .filter(move |&alias| Some(alias) != next)
        });
    real_indirect_words(code, bound)
        .chain(low_indirect_words(code, bound))
        .chain(direct)
        .collect()
}

/// Where one `rfid`, `mtmsr` or `mtmsrd` can put control into real mode.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Switch {
    /// The word cannot set MSR\[IR\] to 0, or it is not such a word.
    None,
    /// The word can put control into real mode at each of these
    /// addresses.
    To(BTreeSet<u64>),
    /// The word can put control into real mode at an address that the
    /// scan does not know, such as an SRR0 that the path loads.
    Unbounded,
}

/// The words of `code` where control enters in real mode:
/// - the ELF entry, where the kernel run starts thread 0 with MSR\[IR\]
///   and MSR\[DR\] at 0;
/// - each word whose real address is in the interrupt vectors.
///
/// Real mode ignores EA bits 0 and 1, so the real address of a word is
/// its address without them.
///
/// [PPC-Book3 p:62 s:5.4] An interrupt sets MSR bits IR and DR to 0 and MSR bit SF to 1.
pub(crate) fn real_mode_entries(elf: &[u8], code: &Code) -> BTreeSet<u64> {
    let elf_entry = elf
        .get(ELF64_E_ENTRY..ELF64_E_ENTRY + 8)
        .map(|entry| u64::from_be_bytes(entry.try_into().expect("eight bytes")))
        .filter(|&entry| code.holds(entry));
    let vectors = code
        .segments()
        .iter()
        .flat_map(CodeSegment::addressed)
        .map(|(address, _)| address)
        .filter(|address| {
            INTERRUPT_VECTOR_REAL_ADDRESSES.contains(&(address & !REAL_MODE_IGNORED_EA))
        });
    elf_entry.into_iter().chain(vectors).collect()
}

/// A privileged word that writes SRR0 or SRR1, or that sets MSR\[IR\].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MsrWord {
    /// `rfid`.
    Rfid,
    /// `mtmsrd` with L = 0.
    Mtmsrd { rs: u8 },
    /// `mtmsr` with L = 0.
    Mtmsr { rs: u8 },
    /// `mtspr` of SRR0.
    MtSrr0 { rs: u8 },
    /// `mtspr` of SRR1.
    MtSrr1 { rs: u8 },
}

impl MsrWord {
    /// The word `raw` as an [`MsrWord`], or `None` for any other word.
    ///
    /// [PPC-Book3 p:119 s:Appendix G] `rfid` is primary opcode 19 with extended opcode 18.
    /// [PPC-Book3 p:22 s:3.4] `mtmsrd` is primary opcode 31 with extended opcode 178, RS in bits 6:10 and L in bit 15.
    /// [PPC-Book3 p:91 s:8.3] `mtmsr` is primary opcode 31 with extended opcode 146, RS in bits 6:10 and L in bit 15.
    /// [PPC-Book3 p:19 s:3.4] `mtspr` is primary opcode 31 with extended opcode 467 and RS in bits 6:10.
    pub(super) fn of(raw: u32) -> Option<Self> {
        let rs = ((raw >> 21) & 0x1F) as u8;
        let l = (raw >> 16) & 1 == 1;
        Some(match (raw >> 26, (raw >> 1) & 0x3FF) {
            (19, 18) => Self::Rfid,
            (31, 178) if !l => Self::Mtmsrd { rs },
            (31, 146) if !l => Self::Mtmsr { rs },
            (31, 467) if spr_number(raw) == SPR_SRR0 => Self::MtSrr0 { rs },
            (31, 467) if spr_number(raw) == SPR_SRR1 => Self::MtSrr1 { rs },
            _ => return None,
        })
    }
}

/// The address that the next fetch reaches from `address` under the MSR
/// value `msr`.
///
/// With MSR\[SF\] = 0, the fetch address has its high-order 32 bits at 0.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
/// [PPC-Book1 p:15 s:1.12] A change from 64-bit mode to 32-bit mode sets the high-order 32 bits of the next instruction address to 0.
pub(super) fn fetch_address(address: u64, msr: u64) -> u64 {
    if msr & (MSR_SF | MSR_BIT1) != 0 {
        address
    } else {
        address & u64::from(u32::MAX)
    }
}
