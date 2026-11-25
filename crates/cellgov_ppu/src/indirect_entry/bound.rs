//! The bound as it grows: the words that the file bytes, the code
//! constants and the relative jump tables name, in rounds until no round
//! names a new word.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::format::elf::ppc64_function_descriptor as descriptor;

use super::constants::{Constants, Value};
use super::ctr_flow::CtrFlow;
use super::join_flow::{Bounds, JoinFlow, JoinWords};
use super::lost::Origins;
use super::real_mode::{real_mode_entries, Switches};
use super::toc::{function_tocs, FunctionToc, FunctionWords};
use crate::caller_census::{CallerScanError, JoinDrop};
use crate::hypercall_cfg::{low_word, real_forms, Code, CodeSegment};
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

    /// The big-endian value of each 32-bit word that starts at a
    /// word-aligned address.
    fn words(&self) -> impl Iterator<Item = u32> + '_ {
        let first = (4 - self.vaddr % 4) % 4;
        (first as usize..)
            .step_by(4)
            .map_while(|offset| self.bytes.get(offset..)?.first_chunk::<4>().copied())
            .map(u32::from_be_bytes)
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

/// The mode of the code that reads a relative jump table and branches
/// through it: the bits of an address that the load and the `bcctr` use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Width {
    /// 64-bit mode, which uses all 64 bits.
    Full,
    /// 32-bit mode, which uses only the low-order 32 bits.
    Low,
}

impl Width {
    /// The words of `code` that a `bcctr` in this mode can enter with
    /// `target` in CTR: each word at the real address of the target word.
    ///
    /// The words hold the target word itself, which a `bcctr` with
    /// translation enters, and each word that a `bcctr` in real mode
    /// enters.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
    /// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
    /// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
    fn words(self, code: &Code, target: u64) -> impl Iterator<Item = u64> + '_ {
        let word = match self {
            Self::Full => target & !0b11,
            Self::Low => low_word(target),
        };
        code.real_aliases(word)
    }

    /// The address of the entry `index` of a relative jump table at
    /// `start`, which is `start` plus 4 times `index`.
    ///
    /// In 32-bit mode the sum wraps in the low-order 32 bits, and the
    /// high-order 32 bits stay those of `start`. For a start at the low
    /// word of a base, these bits hold only EA bits 0 and 1.
    ///
    /// [PPC-Book1 p:15 s:1.12.2] Effective address arithmetic wraps from 2^64 - 1 to 0 in 64-bit mode, and from 2^32 - 1 to 0 in 32-bit mode.
    fn entry(self, start: u64, index: u64) -> u64 {
        let at = start.wrapping_add(index * 4);
        match self {
            Self::Full => at,
            Self::Low => (start & !LOW_HALF) | (at & LOW_HALF),
        }
    }

    /// Whether `word` is one of the entries `0..=index` of a relative
    /// jump table at `start`, as [`Self::entry`] gives them.
    fn in_run(self, start: u64, index: u64, word: u64) -> bool {
        let offset = word.wrapping_sub(start);
        match self {
            Self::Full => offset <= index * 4,
            Self::Low => ((word ^ start) & !LOW_HALF) == 0 && (offset & LOW_HALF) <= index * 4,
        }
    }
}

/// The low-order 32 bits of an address.
const LOW_HALF: u64 = 0xFFFF_FFFF;

/// The result of [`indirect_entries`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct IndirectBound {
    /// The words that control can enter other than by a direct branch or
    /// a fall-through, with the real-mode entries.
    pub(crate) entries: BTreeSet<u64>,
    /// Each word at the real address of a doubleword or a 32-bit word of
    /// the file bytes, or of a constant that the code builds.
    ///
    /// A `bcctr` or a `bclr` in real mode can reach these words with a
    /// CTR or LR that the scan does not record. A branch in virtual mode
    /// reaches only the word at the named address itself.
    pub(crate) real_named: BTreeSet<u64>,
    /// Each word at the low-order 32 bits of a doubleword or a 32-bit
    /// word of the file bytes, or of a constant that the code builds.
    ///
    /// A `bcctr` or a `bclr` in 32-bit mode can reach these words with a
    /// CTR or LR that the scan does not record.
    pub(crate) low_named: BTreeSet<u64>,
    /// The address of each word that writes a constant into CTR or LR,
    /// with each address that an indirect branch reaches from it.
    ///
    /// A path of fall-throughs and direct branches goes from the word to
    /// the branch. The scan takes control that reaches the word in real
    /// mode to reach the address in real mode.
    pub(crate) targets: BTreeMap<u64, BTreeSet<u64>>,
    /// The address of each `bcctr` or `bclr` that can reach an address
    /// that the scan does not know.
    ///
    /// A path reaches the `bcctr` after one of these steps, with no CTR
    /// write between:
    /// - a decrement of a constant, because a loop can run it more than
    ///   once;
    /// - a call, because its routine can write CTR before it returns;
    /// - an `mtctr` of a GPR that can hold a constant that `JoinFlow`
    ///   dropped.
    ///
    /// A path reaches the `bclr` with an LR that can hold a constant that
    /// `JoinFlow` dropped.
    ///
    /// The set also holds each `rfid` that can leave MSR\[IR\] at 1 when a
    /// path reaches it with a lost SRR0. A lost SRR0 can hold a constant
    /// that `JoinFlow` dropped.
    pub(crate) lost_targets: BTreeSet<u64>,
    /// The joins that dropped the constants of each lost value that an
    /// `mtctr`, a `bclr` or an `rfid` takes, by the address of the word.
    ///
    /// The map is empty when [`Self::lost_targets`] is empty.
    pub(crate) lost_origins: BTreeMap<u64, LostOrigins>,
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
    /// Each word where an `rfid` or `mtmsrd` can put control into 32-bit
    /// mode, with MSR\[SF\] and MSR bit 1 at 0.
    pub(crate) mode32_switches: BTreeSet<u64>,
    /// The address of each `rfid` or `mtmsrd` that can put control into
    /// 32-bit mode at an address that the scan does not know.
    pub(crate) unbounded_mode32_switches: BTreeSet<u64>,
}

/// The joins that dropped the constants of one lost value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LostOrigins {
    /// Each join that the lost mark names.
    pub(crate) drops: BTreeSet<JoinDrop>,
    /// Whether the value can also hold a constant of a join that
    /// `drops` does not name.
    pub(crate) more: bool,
}

/// The most times [`indirect_entries`] raises the bounds of `JoinFlow`
/// and runs the bound again.
const REFINEMENTS: usize = 16;

/// The words of `code` that control can enter other than by a direct
/// branch or a fall-through, and the targets of the indirect branches.
///
/// `files` holds the file bytes of every segment of `elf`.
///
/// The scan repeats in rounds until a round names no new word, because
/// each of these depends on the other:
/// - an indirect branch in a function reaches each named word, in real
///   mode each word of [`IndirectBound::real_named`], and in 32-bit mode
///   each word of [`IndirectBound::low_named`];
/// - r2 on the words of a function names more words.
///
/// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR, so r2 at the target is r2 at the branch.
///
/// The named words only grow, so the rounds end.
///
/// When the rounds end with a lost target, the bound refines `JoinFlow`
/// and runs the rounds again from the named words it has:
/// - it takes the origins of each lost value that an `mtctr`, a `bclr`
///   or an `rfid` takes;
/// - it raises the bound of each join that dropped those constants, as
///   [`Bounds::raise`] states.
///
/// A raised join keeps each constant that the paths bring, so it drops
/// none. The refinement stops when no lost target remains, when it can
/// raise no bound, or after [`REFINEMENTS`] runs. A loop that steps a
/// constant brings more constants than any bound, so it stays lost.
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
        low_named: BTreeSet::new(),
        tables: BTreeSet::new(),
        targets: BTreeMap::new(),
        ctr_writes: BTreeMap::new(),
        lost_ctr_writes: BTreeMap::new(),
        lost_returns: BTreeMap::new(),
        lost_rfids: BTreeMap::new(),
    };
    for file in files {
        for value in file.doublewords() {
            bound.name(value);
        }
        for value in file.words() {
            // [PPC-Book1 p:38 s:3.3.2] `lwa` fills RT bits 0:31 with a copy of bit 0 of the loaded word.
            bound.name(u64::from(value));
            bound.name(i64::from(value.cast_signed()).cast_unsigned());
        }
    }
    bound.tables.extend(tables_after_bcctr(code));
    let functions = function_tocs(files, code);
    let every = Value::sorted(functions.values().flatten().copied().collect());
    let mut words = FunctionWords::new(code, &functions);
    let join_words = JoinWords::new(code);
    let mut bounds = Bounds::default();
    let mut refinements = 0;
    let mut lost_origins = BTreeMap::new();
    let lost_targets = loop {
        bound.rounds(&mut words, &join_words, &every, &bounds)?;
        let lost_writes: BTreeSet<(usize, u64)> = bound.lost_ctr_writes.keys().copied().collect();
        let mut flow = CtrFlow::new(code, &bound.ctr_writes, &lost_writes);
        flow.run();
        let mut lost_targets = flow.bcctr_targets(&mut bound.targets);
        lost_targets.extend(bound.lost_returns.keys());
        lost_targets.extend(bound.lost_rfids.keys());
        if lost_targets.is_empty() {
            break lost_targets;
        }
        // The last round named no new word, so this run repeats its flow,
        // with the same drop ids, and counts each drop.
        let drops = JoinFlow::run(&join_words, &words.tocs, &every, &bounds, true).drops();
        let origins = bound.lost_values().map(|(_, origins)| origins);
        if refinements == REFINEMENTS || !bounds.raise(origins, &drops) {
            lost_origins = bound.lost_origins(&drops);
            break lost_targets;
        }
        refinements += 1;
    };
    let mut entries = real_mode_entries(elf, code);
    entries.extend(bound.named);
    let mut indirect = IndirectBound {
        entries,
        real_named: bound.real_named,
        low_named: bound.low_named,
        targets: bound.targets,
        lost_targets,
        lost_origins,
        ..IndirectBound::default()
    };
    let switches = Switches::find(code, &words.tocs, &every, &indirect);
    indirect.real_mode_switches = switches.targets;
    indirect.unbounded_switches = switches.unbounded;
    indirect.mode32_switches = switches.mode32;
    indirect.unbounded_mode32_switches = switches.unbounded_mode32;
    Ok(indirect)
}

/// The address of the word after each `bcctr` of `code` that does not
/// link, in each [`Width`]. A relative jump table can start there.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` branches to the address in CTR.
fn tables_after_bcctr(code: &Code) -> impl Iterator<Item = (u64, Width)> + '_ {
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
        .flat_map(|start| [(start, Width::Full), (start, Width::Low)])
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
    /// The [`IndirectBound::low_named`].
    low_named: BTreeSet<u64>,
    /// The addresses that a relative jump table can start at, with the
    /// mode that reads it, which the next call to [`Self::jump_tables`]
    /// reads. A later round can add an address again.
    tables: BTreeSet<(u64, Width)>,
    /// The targets of the `bclr` words, from the TOCs of the latest
    /// round.
    targets: BTreeMap<u64, BTreeSet<u64>>,
    /// The constants that each `mtctr` copies into CTR on the paths to
    /// it, by segment index and address, from the TOCs of the latest
    /// round.
    ctr_writes: BTreeMap<(usize, u64), Value>,
    /// Each `mtctr` that can also copy a constant that [`JoinFlow`]
    /// dropped, by segment index and address, with the origins of the
    /// GPR, from the TOCs of the latest round.
    lost_ctr_writes: BTreeMap<(usize, u64), Origins>,
    /// The address of each `bclr` whose LR can hold a constant that
    /// [`JoinFlow`] dropped, with the origins of LR, from the TOCs of the
    /// latest round.
    lost_returns: BTreeMap<u64, Origins>,
    /// The address of each `rfid` that [`Constants::translated_rfid_lost`]
    /// gives, with the origins of SRR0, from the TOCs of the latest round.
    lost_rfids: BTreeMap<u64, Origins>,
}

/// Joins `origins` into the origins of `key` in `lost`.
fn note<K: Ord>(lost: &mut BTreeMap<K, Origins>, key: K, origins: &Origins) {
    lost.entry(key)
        .and_modify(|own| *own = own.union(origins))
        .or_insert_with(|| origins.clone());
}

impl Bound<'_> {
    /// Runs the rounds of the bound until a round names no new word.
    ///
    /// `bounds` holds the raised bounds of `JoinFlow`. The origins that
    /// the last round records name the drops of its `JoinFlow` run.
    fn rounds(
        &mut self,
        words: &mut FunctionWords,
        join_words: &JoinWords<'_>,
        every: &Value,
        bounds: &Bounds,
    ) -> Result<(), CallerScanError> {
        let code = self.code;
        loop {
            let before = self.named_lens();
            let reached: BTreeSet<u64> = self
                .named
                .iter()
                .chain(&self.real_named)
                .chain(&self.low_named)
                .copied()
                .collect();
            words.grow(&reached);
            self.targets.clear();
            self.ctr_writes.clear();
            self.lost_ctr_writes.clear();
            self.lost_returns.clear();
            self.lost_rfids.clear();
            for (index, segment) in code.segments().iter().enumerate() {
                self.code_constants(index, segment, words.tocs.segment(index), every, None);
            }
            let joins = JoinFlow::run(join_words, &words.tocs, every, bounds, false);
            for (index, segment) in code.segments().iter().enumerate() {
                let tocs = words.tocs.segment(index);
                self.code_constants(index, segment, tocs, every, Some(&joins));
            }
            self.jump_tables()?;
            if self.named_lens() == before {
                return Ok(());
            }
        }
    }

    /// The address of each `mtctr`, `bclr` and `rfid` that takes a lost
    /// value, with the origins of that value.
    fn lost_values(&self) -> impl Iterator<Item = (u64, &Origins)> + '_ {
        self.lost_ctr_writes
            .iter()
            .map(|(&(_, address), origins)| (address, origins))
            .chain(
                self.lost_returns
                    .iter()
                    .map(|(&address, origins)| (address, origins)),
            )
            .chain(
                self.lost_rfids
                    .iter()
                    .map(|(&address, origins)| (address, origins)),
            )
    }

    /// The [`IndirectBound::lost_origins`], from the drops `drops` of the
    /// run that recorded the lost values.
    fn lost_origins(&self, drops: &[(usize, JoinDrop)]) -> BTreeMap<u64, LostOrigins> {
        let mut lost: BTreeMap<u64, LostOrigins> = BTreeMap::new();
        for (address, origins) in self.lost_values() {
            let own = lost.entry(address).or_default();
            let unknown = origins
                .drops()
                .iter()
                .any(|&id| drops.get(id as usize).is_none());
            own.more |= origins.has_more() || unknown;
            own.drops.extend(
                origins
                    .drops()
                    .iter()
                    .filter_map(|&id| drops.get(id as usize))
                    .map(|&(_, drop)| drop),
            );
        }
        lost
    }

    /// The sizes of the sets of named words, which grow until a round
    /// names no new word.
    fn named_lens(&self) -> [usize; 3] {
        [
            self.named.len(),
            self.real_named.len(),
            self.low_named.len(),
        ]
    }

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

    /// Records a named address: a doubleword or a 32-bit word of the file
    /// bytes, or a constant that the code builds. For this address:
    /// - an indirect branch can reach it;
    /// - an indirect branch in real mode can reach each word at its real
    ///   address, also when no segment holds the address itself;
    /// - an indirect branch in 32-bit mode can reach the word at its
    ///   low-order 32 bits;
    /// - when the address is 4-aligned, a relative jump table can start
    ///   at each address with the real address of one of these two:
    ///   - the named address, which 64-bit code reads;
    ///   - its low-order 32 bits, which 32-bit code reads.
    ///
    /// Real-mode code reads at the real address. A table start needs a
    /// word of the file bytes.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
    /// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
    /// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
    /// [PPC-Book1 p:15 s:1.12.2] In 32-bit mode the high-order 32 bits of a computed effective address are set to 0 for a data access.
    fn name(&mut self, address: u64) {
        self.enter(address);
        let code = self.code;
        self.real_named.extend(code.real_aliases(address & !0b11));
        let low = low_word(address);
        if code.holds(low) {
            self.low_named.insert(low);
        }
        if !address.is_multiple_of(4) {
            return;
        }
        let files = self.files;
        let starts = [(address, Width::Full), (low, Width::Low)]
            .into_iter()
            .flat_map(|(read, width)| real_forms(read).map(move |start| (start, width)))
            .filter(|&(start, _)| files.iter().any(|file| file.read::<4>(start).is_some()));
        self.tables.extend(starts);
    }

    /// Does these steps for the code of `segment`:
    /// - names each constant that the code builds in a GPR;
    /// - keeps the target of each `bclr` whose LR the code builds;
    /// - keeps each constant that an `mtctr` copies;
    /// - enters each word that an `rfid` can enter with MSR\[IR\] at 1;
    /// - keeps each `rfid` that can enter a word with MSR\[IR\] at 1 from
    ///   an SRR0 that can hold a constant that [`JoinFlow`] dropped.
    ///
    /// `index` is the index of `segment` in the code. `tocs` holds the
    /// TOC of each word of `segment`, as [`FunctionWords`] gives it.
    /// `every` holds each TOC that a descriptor gives. On each word, r2
    /// can hold the constants of the path and the TOCs of the word.
    ///
    /// Without `joins`, the scan follows each straight-line path. With
    /// `joins`, each join word starts with what [`JoinFlow`] gives it.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
    /// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
    pub(super) fn code_constants(
        &mut self,
        index: usize,
        segment: &CodeSegment,
        tocs: &[FunctionToc],
        every: &Value,
        joins: Option<&JoinFlow<'_>>,
    ) {
        let mut constants = Constants::default();
        for (word, ((address, raw), &toc)) in segment.addressed().zip(tocs).enumerate() {
            if let Some(entered) = joins.and_then(|joins| joins.entered((index, word))) {
                constants = entered;
            }
            constants.enter_toc(toc, every);
            let decoded = crate::decode::decode(raw);
            if let Ok(PpuInstruction::Bclr { .. }) = decoded {
                for (&by, lr) in &constants.lr {
                    self.targets
                        .entry(by)
                        .or_default()
                        .extend(lr.constants().iter().map(|&lr| lr & !0b11));
                }
                if let Some(origins) = &constants.lr_lost {
                    note(&mut self.lost_returns, address, origins);
                }
            }
            if let Ok(PpuInstruction::Mtctr { rs }) = decoded {
                if let Some(value) = constants.gpr(rs) {
                    self.ctr_writes
                        .entry((index, address))
                        .and_modify(|own| *own = own.clone().union(value))
                        .or_insert_with(|| value.clone());
                }
                if let Some(origins) = constants.lost(rs) {
                    note(&mut self.lost_ctr_writes, (index, address), origins);
                }
            }
            for target in constants.translated_rfid_targets(raw) {
                self.enter(target);
            }
            if let Some(origins) = constants.translated_rfid_lost(raw) {
                note(&mut self.lost_rfids, address, origins);
            }
            for &constant in constants
                .step(address, raw)
                .iter()
                .flat_map(Value::constants)
            {
                self.name(constant);
            }
        }
    }

    /// Adds each target of each relative jump table that starts at an
    /// address of [`Self::tables`], and empties it.
    ///
    /// A table starts where [`Self::name`] states, or at the word after a
    /// `bcctr` that does not link. Each entry is a 32-bit offset from the
    /// table's start. Its targets are the words that [`Width::words`]
    /// gives for the start plus the offset. The run of entries ends at
    /// the first entry that has no target outside the run itself.
    ///
    /// When real-mode code reads a table, the EA bits 0 and 1 of the
    /// start can differ from those of the base. The start plus the offset
    /// then has the real address of the target that the code computes.
    /// The scan reads each entry at each address with the real address of
    /// the entry, because real-mode code reads the entry there.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
    /// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
    fn jump_tables(&mut self) -> Result<(), CallerScanError> {
        let tables = std::mem::take(&mut self.tables);
        for (start, width) in tables {
            self.jump_table(start, width)?;
        }
        Ok(())
    }

    /// Adds each target of the relative jump table at `start`, which code
    /// in the mode `width` reads.
    fn jump_table(&mut self, start: u64, width: Width) -> Result<(), CallerScanError> {
        let code = self.code;
        for index in 0..=JUMP_TABLE_LIMIT {
            let at = width.entry(start, index);
            let targets: Vec<u64> = self
                .files
                .iter()
                .flat_map(|file| real_forms(at).filter_map(move |form| file.read::<4>(form)))
                .map(|bytes| start.wrapping_add_signed(i64::from(i32::from_be_bytes(bytes))))
                .flat_map(|target| width.words(code, target))
                .filter(|&word| !width.in_run(start, index, word))
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
