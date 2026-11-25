//! The bound as it grows: the words that the file bytes, the code
//! constants and the relative jump tables name, in rounds until no round
//! names a new word.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::format::elf::ppc64_function_descriptor as descriptor;

use super::constants::{Constants, Value};
use super::ctr_flow::CtrFlow;
use super::real_mode::{real_mode_entries, Switches};
use super::toc::{function_tocs, FunctionToc, FunctionWords};
use crate::caller_census::CallerScanError;
use crate::hypercall_cfg::{Code, CodeSegment};
use crate::instruction::PpuInstruction;

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

    /// The code address, the TOC and the environment of a function
    /// descriptor that starts at each word-aligned address.
    pub(super) fn descriptors(&self) -> impl Iterator<Item = [u64; 3]> + '_ {
        let first = (4 - self.vaddr % 4) % 4;
        (first as usize..)
            .step_by(4)
            .map_while(|offset| {
                self.bytes
                    .get(offset..offset.checked_add(descriptor::SIZE)?)
            })
            .map(|bytes| {
                [
                    descriptor::CODE_OFFSET,
                    descriptor::TOC_OFFSET,
                    descriptor::ENV_OFFSET,
                ]
                .map(|field| {
                    u64::from_be_bytes(bytes[field..field + 8].try_into().expect("eight bytes"))
                })
            })
    }
}

/// The result of [`indirect_entries`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct IndirectBound {
    /// The words that control can enter other than by a direct branch or
    /// a fall-through, with the real-mode entries.
    pub(crate) entries: BTreeSet<u64>,
    /// Each word at the real address of a constant that the code builds.
    ///
    /// A `bcctr` or a `bclr` in real mode can reach these words with a
    /// CTR or LR that the scan does not record. A branch in virtual mode
    /// reaches only the word at the constant itself.
    pub(crate) real_named: BTreeSet<u64>,
    /// The address of each word that writes a constant into CTR or LR,
    /// with each address that an indirect branch reaches from it.
    ///
    /// A path of fall-throughs and direct branches goes from the word to
    /// the branch. The scan takes control that reaches the word in real
    /// mode to reach the address in real mode.
    pub(crate) targets: BTreeMap<u64, BTreeSet<u64>>,
    /// The address of each `bcctr` that can reach an address that the
    /// scan does not know.
    ///
    /// A path reaches the `bcctr` after one of these steps, with no CTR
    /// write between:
    /// - a decrement of a constant, because a loop can run it more than
    ///   once;
    /// - a call, because its routine can write CTR before it returns.
    pub(crate) lost_targets: BTreeSet<u64>,
    /// Each address where an `rfid`, `mtmsr` or `mtmsrd` can put control
    /// into real mode.
    ///
    /// The word sets MSR\[IR\] from its MSR value whatever MSR\[IR\] was
    /// before it, so the address runs in real mode whichever mode the
    /// word runs in.
    pub(crate) real_mode_switches: BTreeSet<u64>,
    /// The address of each `rfid`, `mtmsr` or `mtmsrd` that can put
    /// control into real mode at an address that the scan does not know.
    pub(crate) unbounded_switches: BTreeSet<u64>,
}

/// The words of `code` that control can enter other than by a direct
/// branch or a fall-through, and the targets of the indirect branches.
///
/// `files` holds the file bytes of every segment of `elf`.
///
/// The scan repeats in rounds until a round names no new word, because
/// each of these depends on the other:
/// - an indirect branch in a function reaches each named word, and in
///   real mode each word of [`IndirectBound::real_named`];
/// - r2 on the words of a function names more words.
///
/// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR, so r2 at the target is r2 at the branch.
///
/// The named words only grow, so the rounds end.
///
/// # Errors
///
/// Returns [`CallerScanError::JumpTableTooLong`] when a relative jump
/// table runs past [`JUMP_TABLE_LIMIT`] entries.
pub(crate) fn indirect_entries(
    elf: &[u8],
    files: &[FileSegment<'_>],
    code: &Code,
) -> Result<IndirectBound, CallerScanError> {
    let mut bound = Bound {
        code,
        files,
        named: BTreeSet::new(),
        real_named: BTreeSet::new(),
        tables: BTreeSet::new(),
        targets: BTreeMap::new(),
        ctr_writes: BTreeMap::new(),
    };
    for file in files {
        for value in file.doublewords() {
            bound.name(value);
        }
    }
    bound.tables.extend(tables_after_bcctr(code));
    let functions = function_tocs(files, code);
    let every = Value::sorted(functions.values().flatten().copied().collect());
    let mut words = FunctionWords::new(code, &functions);
    loop {
        let before = (bound.named.len(), bound.real_named.len());
        let reached: BTreeSet<u64> = bound.named.union(&bound.real_named).copied().collect();
        words.grow(&reached);
        bound.targets.clear();
        bound.ctr_writes.clear();
        for (index, segment) in code.segments().iter().enumerate() {
            bound.code_constants(index, segment, words.tocs.segment(index), &every);
        }
        bound.jump_tables()?;
        if (bound.named.len(), bound.real_named.len()) == before {
            break;
        }
    }
    let mut flow = CtrFlow::new(code, &bound.ctr_writes);
    flow.run();
    let lost_targets = flow.bcctr_targets(&mut bound.targets);
    let mut entries = real_mode_entries(elf, code);
    entries.extend(bound.named);
    let mut indirect = IndirectBound {
        entries,
        real_named: bound.real_named,
        targets: bound.targets,
        lost_targets,
        ..IndirectBound::default()
    };
    let switches = Switches::find(code, &words.tocs, &every, &indirect);
    indirect.real_mode_switches = switches.targets;
    indirect.unbounded_switches = switches.unbounded;
    Ok(indirect)
}

/// The address of the word after each `bcctr` of `code` that does not
/// link. A relative jump table can start there.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` branches to the address in CTR.
fn tables_after_bcctr(code: &Code) -> impl Iterator<Item = u64> + '_ {
    code.segments()
        .iter()
        .flat_map(CodeSegment::addressed)
        .filter(|&(_, raw)| {
            matches!(
                crate::decode::decode(raw),
                Ok(PpuInstruction::Bcctr { link: false, .. })
            )
        })
        .filter_map(|(address, _)| address.checked_add(4))
}

/// The bound as it grows, without the real-mode entries.
pub(super) struct Bound<'a> {
    code: &'a Code,
    files: &'a [FileSegment<'a>],
    /// The words of the code that the image names, and the targets of
    /// the relative jump tables.
    named: BTreeSet<u64>,
    /// The [`IndirectBound::real_named`].
    real_named: BTreeSet<u64>,
    /// The addresses that a relative jump table can start at, which the
    /// next call to [`Self::jump_tables`] reads. A later round can add
    /// an address again.
    tables: BTreeSet<u64>,
    /// The targets of the `bclr` words, from the TOCs of the latest
    /// round.
    targets: BTreeMap<u64, BTreeSet<u64>>,
    /// The constants that each `mtctr` copies into CTR on its
    /// straight-line path, by segment index and address, from the TOCs
    /// of the latest round.
    ctr_writes: BTreeMap<(usize, u64), Value>,
}

impl Bound<'_> {
    /// Adds the word that an indirect branch to `address` reaches to the
    /// bound, when it is a word of the code.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
    fn enter(&mut self, address: u64) {
        let word = address & !0b11;
        if self.code.holds(word) {
            self.named.insert(word);
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

    /// Records a constant that the code builds. It is a named address,
    /// and an indirect branch in real mode reaches each word at its real
    /// address.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
    /// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
    fn name_constant(&mut self, constant: u64) {
        self.name(constant);
        let code = self.code;
        self.real_named.extend(code.real_aliases(constant & !0b11));
    }

    /// Names each constant that the code of `segment` builds in a GPR,
    /// keeps the target of each `bclr` whose LR it builds, and keeps each
    /// constant that an `mtctr` copies.
    ///
    /// `index` is the index of `segment` in the code. `tocs` holds the
    /// TOC of each word of `segment`, as [`FunctionWords`] gives it.
    /// `every` holds each TOC that a descriptor gives. On each word, r2
    /// can hold the constants of the path and the TOCs of the word.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
    pub(super) fn code_constants(
        &mut self,
        index: usize,
        segment: &CodeSegment,
        tocs: &[FunctionToc],
        every: &Value,
    ) {
        let mut constants = Constants::default();
        for ((address, raw), &toc) in segment.addressed().zip(tocs) {
            constants.gprs[2] = toc.in_r2(constants.gprs[2].take(), every);
            let decoded = crate::decode::decode(raw);
            if let (Ok(PpuInstruction::Bclr { .. }), Some(lr)) = (&decoded, &constants.lr) {
                self.targets
                    .entry(lr.by)
                    .or_default()
                    .extend(lr.value.constants().iter().map(|&lr| lr & !0b11));
            }
            if let Ok(PpuInstruction::Mtctr { rs }) = decoded {
                self.ctr_writes.extend(
                    constants
                        .gpr(rs)
                        .map(|value| ((index, address), value.clone())),
                );
            }
            for &constant in constants
                .step(address, raw)
                .iter()
                .flat_map(Value::constants)
            {
                self.name_constant(constant);
            }
        }
    }

    /// Adds each target of each relative jump table that starts at an
    /// address of [`Self::tables`], and empties it.
    ///
    /// A table starts at a named address or at the word after a `bcctr`
    /// that does not link. Each entry is a 32-bit offset from the
    /// table's start, and its target is the word at the start plus the
    /// offset, with bits 62 and 63 at 0. The run of entries ends at the
    /// first entry whose target:
    /// - is no word of the code; or
    /// - is in the run itself.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
    fn jump_tables(&mut self) -> Result<(), CallerScanError> {
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
                .map(|target| target & !0b11)
                .filter(|word| !(start..=at).contains(word) && self.code.holds(*word))
                .collect();
            if targets.is_empty() {
                return Ok(());
            }
            if index == JUMP_TABLE_LIMIT {
                return Err(CallerScanError::JumpTableTooLong { start });
            }
            self.named.extend(targets);
        }
        Ok(())
    }
}
