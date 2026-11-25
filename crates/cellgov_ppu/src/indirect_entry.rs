//! Bounds the words of an ELF that control can enter in a way other than
//! a direct branch or a fall-through.
//!
//! A word is in the bound when one of these can enter it:
//! - the start of execution, at the ELF entry;
//! - an interrupt, at a word whose real address is in the interrupt
//!   vectors;
//! - an indirect branch (`bcctr`, `bclr`) or an `rfid`, at an address
//!   that the image names.
//!
//! The image names an address in three ways:
//! - a doubleword in the file bytes of a segment, such as a function
//!   descriptor or a TOC entry;
//! - a constant that the code builds in a GPR on a straight-line path;
//! - an entry of a relative jump table: a 32-bit offset from a named
//!   address, or from the word after a `bcctr`.
//!
//! The bound does not hold an address that the code computes in another
//! way, such as from a value it loads at run time.

use std::collections::BTreeSet;

use cellgov_ps3_abi::format::elf::ELF64_E_ENTRY;
use cellgov_ps3_abi::hw::ppu_interrupt::INTERRUPT_VECTOR_REAL_ADDRESSES;
use cellgov_ps3_abi::hw::ppu_mmu::REAL_MODE_IGNORED_EA;

use crate::caller_census::{maybe_writes_gpr, CallerScanError};
use crate::hypercall_cfg::{Code, CodeSegment};
use crate::instruction::PpuInstruction;
use crate::local_cfg::falls_through;

/// Most entries the bound reads from one relative jump table. The scan
/// refuses an ELF with a run of entries that goes on past it.
///
/// [PPC-Book1 p:61 s:3.3.9] `cmpli` compares with a 16-bit unsigned immediate, so it bounds a table index below 65536.
pub(crate) const JUMP_TABLE_LIMIT: u64 = 0x1_0000;

/// The file bytes of one segment, which start at `vaddr`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FileSegment<'elf> {
    /// The address of the first byte.
    pub(crate) vaddr: u64,
    /// The bytes the ELF file holds for the segment.
    pub(crate) bytes: &'elf [u8],
}

impl FileSegment<'_> {
    /// The `N` bytes at `address`, when the file bytes hold all of them.
    fn read<const N: usize>(&self, address: u64) -> Option<[u8; N]> {
        let start = usize::try_from(address.checked_sub(self.vaddr)?).ok()?;
        self.bytes
            .get(start..start.checked_add(N)?)?
            .try_into()
            .ok()
    }

    /// The big-endian value of each doubleword that starts at a
    /// word-aligned address.
    fn doublewords(&self) -> impl Iterator<Item = u64> + '_ {
        let first = (4 - self.vaddr % 4) % 4;
        (first as usize..)
            .step_by(4)
            .map_while(|offset| self.bytes.get(offset..offset.checked_add(8)?))
            .map(|bytes| u64::from_be_bytes(bytes.try_into().expect("eight bytes")))
    }
}

/// The words of `code` that control can enter other than by a direct
/// branch or a fall-through.
///
/// `files` holds the file bytes of every segment of `elf`.
///
/// # Errors
///
/// Returns [`CallerScanError::JumpTableTooLong`] when a relative jump
/// table runs past [`JUMP_TABLE_LIMIT`] entries.
pub(crate) fn indirect_entries(
    elf: &[u8],
    files: &[FileSegment<'_>],
    code: &Code,
) -> Result<BTreeSet<u64>, CallerScanError> {
    let mut bound = Bound {
        code,
        files,
        entries: BTreeSet::new(),
        tables: BTreeSet::new(),
    };
    if let Some(entry) = elf.get(ELF64_E_ENTRY..ELF64_E_ENTRY + 8) {
        bound.enter(u64::from_be_bytes(entry.try_into().expect("eight bytes")));
    }
    for segment in code.segments() {
        bound.interrupt_vectors(segment);
        bound.code_constants(segment);
    }
    for file in files {
        for value in file.doublewords() {
            bound.name(value);
        }
    }
    bound.jump_tables()?;
    Ok(bound.entries)
}

/// The bound as it grows.
struct Bound<'a> {
    code: &'a Code,
    files: &'a [FileSegment<'a>],
    /// The words that control can enter in another way.
    entries: BTreeSet<u64>,
    /// The named addresses that a relative jump table can start at.
    tables: BTreeSet<u64>,
}

impl Bound<'_> {
    /// Adds `address` to the bound when it is a word of the code.
    fn enter(&mut self, address: u64) {
        if self.code.holds(address) {
            self.entries.insert(address);
        }
    }

    /// Records an address that the image names: an indirect branch can
    /// reach it, and a relative jump table can start at it.
    fn name(&mut self, address: u64) {
        self.enter(address);
        if address.is_multiple_of(4)
            && self
                .files
                .iter()
                .any(|file| file.read::<4>(address).is_some())
        {
            self.tables.insert(address);
        }
    }

    /// Adds each word of `segment` that an interrupt can enter.
    ///
    /// Real mode ignores EA bits 0 and 1, so the real address of a word
    /// is its address without them.
    fn interrupt_vectors(&mut self, segment: &CodeSegment) {
        for (address, _) in segment.addressed() {
            if INTERRUPT_VECTOR_REAL_ADDRESSES.contains(&(address & !REAL_MODE_IGNORED_EA)) {
                self.entries.insert(address);
            }
        }
    }

    /// Names each constant that the code of `segment` builds in a GPR.
    fn code_constants(&mut self, segment: &CodeSegment) {
        let mut constants = Constants::default();
        for (address, raw) in segment.addressed() {
            if let Some(value) = constants.step(address, raw) {
                self.name(value);
            }
        }
    }

    /// Adds each target of each relative jump table.
    ///
    /// A table starts at a named address or at the word after a
    /// `bcctr`. Each entry is a 32-bit offset from the table's start.
    /// The run of entries ends at the first offset that reaches no word
    /// of the code, or that reaches back into the run itself.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bcctr` branches to the address in CTR.
    fn jump_tables(&mut self) -> Result<(), CallerScanError> {
        let after_bcctr: Vec<u64> = self
            .code
            .segments()
            .iter()
            .flat_map(CodeSegment::addressed)
            .filter(|&(_, raw)| {
                matches!(
                    crate::decode::decode(raw),
                    Ok(PpuInstruction::Bcctr { link: false, .. })
                )
            })
            .filter_map(|(address, _)| address.checked_add(4))
            .collect();
        self.tables.extend(after_bcctr);
        let tables = std::mem::take(&mut self.tables);
        for start in tables {
            self.jump_table(start)?;
        }
        Ok(())
    }

    /// Adds each target of the relative jump table at `start`.
    fn jump_table(&mut self, start: u64) -> Result<(), CallerScanError> {
        for index in 0..=JUMP_TABLE_LIMIT {
            let Some(at) = start.checked_add(index * 4) else {
                return Ok(());
            };
            let targets: Vec<u64> = self
                .files
                .iter()
                .filter_map(|file| file.read::<4>(at))
                .map(|bytes| start.wrapping_add_signed(i64::from(i32::from_be_bytes(bytes))))
                .filter(|target| !(start..=at).contains(target) && self.code.holds(*target))
                .collect();
            if targets.is_empty() {
                return Ok(());
            }
            if index == JUMP_TABLE_LIMIT {
                return Err(CallerScanError::JumpTableTooLong { start });
            }
            self.entries.extend(targets);
        }
        Ok(())
    }
}

/// The constants that the code has built in its GPRs and in LR on one
/// straight-line path.
#[derive(Debug, Default)]
struct Constants {
    gprs: [Option<u64>; 32],
    lr: Option<u64>,
}

impl Constants {
    /// Steps over the word `raw` at `address`, and returns the constant
    /// it writes into a GPR.
    ///
    /// A word that does not fall through ends the straight-line path, so
    /// the step forgets every constant.
    fn step(&mut self, address: u64, raw: u32) -> Option<u64> {
        let Ok(decoded) = crate::decode::decode(raw) else {
            *self = Self::default();
            return None;
        };
        if let Some((gpr, value)) = self.written(decoded) {
            self.gprs[usize::from(gpr)] = value;
            return value;
        }
        match decoded {
            PpuInstruction::Mtlr { rs } => self.lr = self.gpr(rs),
            PpuInstruction::B { link, .. } => self.after_branch(address, 0b10100, link),
            PpuInstruction::Bc { bo, link, .. }
            | PpuInstruction::Bclr { bo, link, .. }
            | PpuInstruction::Bcctr { bo, link, .. } => self.after_branch(address, bo, link),
            PpuInstruction::Lmw { rt, .. } => {
                for gpr in &mut self.gprs[usize::from(rt)..] {
                    *gpr = None;
                }
            }
            PpuInstruction::Lswi { .. }
            | PpuInstruction::Lswx { .. }
            | PpuInstruction::Sc { .. } => {
                self.gprs = [None; 32];
            }
            _ => self.forget_written(raw),
        }
        None
    }

    /// Forgets the constant in each GPR that the word `raw` may write.
    fn forget_written(&mut self, raw: u32) {
        for (gpr, value) in (0u32..).zip(self.gprs.iter_mut()) {
            if maybe_writes_gpr(raw, gpr) {
                *value = None;
            }
        }
    }

    /// The GPR that `decoded` writes and the constant it writes there,
    /// for a word that builds a constant.
    ///
    /// [PPC-Book1 p:51 s:3.3.8] `addi` and `addis` add EXTS(SI) or EXTS(SI || 0x0000) to RA, or to 0 when RA is 0.
    /// [PPC-Book1 p:66 s:3.3.13] `ori` and `oris` OR UI or UI || 0x0000 into RS.
    /// [PPC-Book1 p:72 s:3.3.12.1] `rldicl` and `rldicr` rotate RS left by SH and AND it with MASK(MB, 63) or MASK(0, ME).
    /// [PPC-Book1 p:82 s:3.3.16] `mflr` copies LR into RT.
    fn written(&self, decoded: PpuInstruction) -> Option<(u8, Option<u64>)> {
        let base = |ra: u8| if ra == 0 { Some(0) } else { self.gpr(ra) };
        Some(match decoded {
            PpuInstruction::Addi { rt, ra, imm } => (
                rt,
                base(ra).map(|value| value.wrapping_add_signed(i64::from(imm))),
            ),
            PpuInstruction::Addis { rt, ra, imm } => (
                rt,
                base(ra).map(|value| value.wrapping_add_signed(i64::from(imm) << 16)),
            ),
            PpuInstruction::Ori { ra, rs, imm } => {
                (ra, self.gpr(rs).map(|value| value | u64::from(imm)))
            }
            PpuInstruction::Oris { ra, rs, imm } => {
                (ra, self.gpr(rs).map(|value| value | u64::from(imm) << 16))
            }
            PpuInstruction::Rldicl { ra, rs, sh, mb, .. } => (
                ra,
                self.gpr(rs)
                    .map(|value| value.rotate_left(u32::from(sh)) & (u64::MAX >> mb)),
            ),
            PpuInstruction::Rldicr { ra, rs, sh, me, .. } => (
                ra,
                self.gpr(rs)
                    .map(|value| value.rotate_left(u32::from(sh)) & (u64::MAX << (63 - me))),
            ),
            PpuInstruction::Or { ra, rs, rb, .. } if rs == rb => (ra, self.gpr(rs)),
            PpuInstruction::Mflr { rt } => (rt, self.lr),
            _ => return None,
        })
    }

    fn gpr(&self, gpr: u8) -> Option<u64> {
        self.gprs.get(usize::from(gpr)).copied().flatten()
    }

    /// Steps over a branch at `address` with the BO field `bo`, which
    /// writes LR when `link` is set.
    ///
    /// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
    fn after_branch(&mut self, address: u64, bo: u8, link: bool) {
        if !falls_through(bo, link) {
            *self = Self::default();
            return;
        }
        if link {
            self.lr = address.checked_add(4);
        }
    }
}

#[cfg(test)]
#[path = "tests/indirect_entry_tests.rs"]
mod tests;
