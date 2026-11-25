//! The bound as it grows: the words that the file bytes, the code
//! constants and the relative jump tables name, in rounds until no round
//! names a new word.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::format::elf::ppc64_function_descriptor as descriptor;

use super::constants::{Constants, Value};
use super::ctr_flow::{CtrFlow, CtrWrites};
use super::join_flow::{Bounds, EntryWords, JoinFlow, JoinWords};
use super::lost::Origins;
use super::progression::Residue;
use super::reach::Reach;
use super::real_mode::{real_mode_entries, Switches};
use super::toc::{function_tocs, FunctionToc, FunctionWords};
use crate::caller_census::{CallerScanError, JoinDrop};
use crate::hypercall_cfg::{
    branches_in_mode32, branches_in_real_mode, low_word, real_forms, return_aliases, Code,
    CodeSegment,
};
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

    /// Each value that a load of 1, 2, 4 or 8 bytes at any address of the
    /// file bytes gives, in either byte order.
    ///
    /// A halfword or a word gives one value with zero extension and one
    /// with sign extension. The list can hold a value more than once.
    ///
    /// [PPC-Book1 p:34 s:3.3.2] `lbz` puts the byte into RT bits 56:63 and sets RT bits 0:55 to 0.
    /// [PPC-Book1 p:38 s:3.3.2] `lwa` fills RT bits 0:31 with a copy of bit 0 of the loaded word.
    fn loads(&self) -> impl Iterator<Item = u64> + '_ {
        (0..self.bytes.len()).flat_map(move |offset| {
            let bytes = &self.bytes[offset..];
            let byte = bytes.first().map(|&byte| u64::from(byte));
            let half = bytes.first_chunk::<2>().map(|half| {
                let [be, le] = [u16::from_be_bytes(*half), u16::from_le_bytes(*half)];
                [be, le].map(|value| {
                    [
                        u64::from(value),
                        i64::from(value.cast_signed()).cast_unsigned(),
                    ]
                })
            });
            let word = bytes.first_chunk::<4>().map(|word| {
                let [be, le] = [u32::from_be_bytes(*word), u32::from_le_bytes(*word)];
                [be, le].map(|value| {
                    [
                        u64::from(value),
                        i64::from(value.cast_signed()).cast_unsigned(),
                    ]
                })
            });
            let double = bytes
                .first_chunk::<8>()
                .map(|double| [[u64::from_be_bytes(*double), u64::from_le_bytes(*double)]]);
            half.into_iter()
                .flatten()
                .chain(word.into_iter().flatten())
                .chain(double.into_iter().flatten())
                .flatten()
                .chain(byte)
        })
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
    /// - a decrement, because a loop can run it more than once;
    /// - a call, because its routine can write CTR before it returns;
    /// - an `mtctr` of a GPR that can hold a constant that `JoinFlow`
    ///   dropped.
    ///
    /// A path reaches the `bclr` with an LR that can hold a constant that
    /// `JoinFlow` dropped, or with a copy of a CTR of the steps above that
    /// an `mtlr` put into LR.
    ///
    /// A dropped constant that is a TOC of a descriptor, and that no word
    /// stepped, does not make the `bcctr` or the `bclr` lost. The bound
    /// names each such TOC, because it is a doubleword of the file bytes.
    /// A value of a progression does not make the branch lost either, once
    /// the bound names each word of its classes.
    ///
    /// The set also holds each `rfid` that can leave MSR\[IR\] at 1 when a
    /// path reaches it with a lost SRR0. A lost SRR0 can hold a constant
    /// that `JoinFlow` dropped. It also holds each `hrfid` that a path
    /// reaches with a lost mark in the SPR cell, which holds HSRR0.
    pub(crate) lost_targets: BTreeSet<u64>,
    /// The address of each `bcctr` and `bclr` that the paths reach only
    /// with constants in CTR or LR.
    ///
    /// Such a branch reaches only the [`Self::targets`] of the words that
    /// wrote those constants. Each other `bcctr` and `bclr` can reach an
    /// address that the image names.
    pub(crate) closed_branches: BTreeSet<u64>,
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
    /// The address of each `rfid`, `hrfid`, `mtmsr` or `mtmsrd` that can
    /// put control into real mode at an address that the scan does not
    /// know.
    pub(crate) unbounded_switches: BTreeSet<u64>,
    /// Each word where an `rfid` or `mtmsrd` can put control into 32-bit
    /// mode, with MSR\[SF\] and MSR bit 1 at 0.
    pub(crate) mode32_switches: BTreeSet<u64>,
    /// The address of each `rfid`, `hrfid` or `mtmsrd` that can put
    /// control into 32-bit mode at an address that the scan does not know.
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
/// The rounds step only the words of a [`Reach`] set. It starts at the
/// ELF entry, the first word of each interrupt vector and each value that
/// a load of the file bytes gives. Each round adds the words that the named words
/// reach. After the rounds, the bound adds the words that the targets and
/// the switches reach, and runs the rounds again until the set stops
/// growing. The set only grows, so this ends.
///
/// The rounds start with no real-address word and no low-order word of a
/// named address among the entry words. When the rounds end with a
/// `bcctr` or `bclr` that can enter such words, the bound adds them and
/// runs the rounds again. A word at a real address of a target that the
/// bound holds is an entry word as soon as the bound finds the target.
///
/// When the rounds end with a lost target, the bound tries these steps in
/// order. The first step that changes something runs the rounds again,
/// from the named words that the bound has:
/// 1. Raise the bound of each join that dropped the constants of a lost
///    value that an `mtctr`, a `bclr` or an `rfid` takes, as
///    [`Bounds::raise`] states. A raised join keeps each constant that
///    the paths bring. The bound skips this step after [`REFINEMENTS`]
///    raises. A loop that steps a constant brings more constants than
///    any bound, so this step leaves its value lost.
/// 2. Name each word of the classes of a lost value that keeps one
///    progression, as [`JoinFlow::progression`] gives them.
/// 3. Add every word to the reach set, because a lost target can enter
///    any word.
///
/// When no step changes anything, the bound returns the lost targets that
/// remain.
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
        real_mode: real_mode_entries(elf, code),
        named: BTreeSet::new(),
        real_named: BTreeSet::new(),
        low_named: BTreeSet::new(),
        tables: BTreeSet::new(),
        bcctrs: unlinked_bcctrs(code).collect(),
        targets: BTreeMap::new(),
        ctr_writes: BTreeMap::new(),
        lost_ctr_writes: BTreeMap::new(),
        ranged_ctr_writes: BTreeSet::new(),
        lost_returns: BTreeMap::new(),
        lost_rfids: BTreeMap::new(),
        closed_bclrs: BTreeSet::new(),
        open_ctr_writes: BTreeSet::new(),
        progressions: false,
        deferred: false,
        residues: BTreeSet::new(),
        loaded: EntryWords::new(code),
        loaded_real: EntryWords::new(code),
        real_branches: false,
        low_branches: false,
        switched: BTreeSet::new(),
        target_aliases: BTreeSet::new(),
        return_aliases: return_aliases(code, |_| true).collect(),
    };
    // A real-mode branch to a loaded value enters the word at its real
    // address too.
    for value in files.iter().flat_map(FileSegment::loads) {
        let word = value & !0b11;
        bound.loaded.add(code, [word]);
        bound.loaded_real.add(code, code.real_aliases(word));
    }
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
    let functions = function_tocs(files, code);
    let every = Value::sorted(functions.values().flatten().copied().collect());
    let mut words = FunctionWords::new(code, &functions);
    let join_words = JoinWords::new(code);
    let mut reach = Reach::new(code);
    reach.grow(bound.real_mode.iter().copied());
    reach.grow(bound.return_aliases.iter().copied());
    // The bound does not name the value of an unaligned load, a byte
    // load, a halfword load or a byte-reversed load. Each can be a code
    // address, so the reach set starts at each such value.
    reach.grow(files.iter().flat_map(FileSegment::loads));
    let mut bounds = Bounds::default();
    let mut refinements = 0;
    loop {
        let mut indirect = bound.reached(&mut reach, &mut words, &join_words, &every, &bounds)?;
        if bound.open_alias_gates(&indirect) {
            continue;
        }
        if indirect.lost_targets.is_empty() {
            return Ok(indirect);
        }
        // The last round named no new word, so this run repeats its flow,
        // with the same drop ids, and counts each drop.
        let entries = bound.entry_words();
        let drops = JoinFlow::run(
            &join_words,
            &words.tocs,
            &every,
            &bounds,
            &reach,
            &entries,
            true,
        )
        .drops();
        let origins = || bound.lost_values().map(|(_, origins)| origins);
        if refinements < REFINEMENTS && bounds.raise(origins(), &drops) {
            refinements += 1;
            continue;
        }
        if bound.take_progressions() {
            continue;
        }
        if reach.fill() {
            continue;
        }
        indirect.lost_origins = bound.lost_origins(&drops);
        return Ok(indirect);
    }
}

/// The index of the segment, the index of the word there and the address
/// of each `bcctr` of `code` that does not link. A relative jump table can
/// start at the word after it.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` branches to the address in CTR.
fn unlinked_bcctrs(code: &Code) -> impl Iterator<Item = (usize, usize, u64)> + '_ {
    code.segments()
        .iter()
        .enumerate()
        .flat_map(|(index, segment)| {
            segment
                .addressed()
                .enumerate()
                .map(move |(word, (address, raw))| (index, word, address, raw))
        })
        .filter(|&(_, _, _, raw)| {
            matches!(
                crate::decode::decode(raw),
                Ok(PpuInstruction::Bcctr { link: false, .. })
            )
        })
        .map(|(index, word, address, _)| (index, word, address))
}

/// The bound as it grows.
pub(super) struct Bound<'a> {
    code: &'a Code,
    files: &'a [FileSegment<'a>],
    /// The real-mode entries: the ELF entry and the first word of each
    /// interrupt vector.
    real_mode: BTreeSet<u64>,
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
    /// Each `bcctr` that does not link, as [`unlinked_bcctrs`] gives it,
    /// that the reach set does not hold yet. When the set takes one, its
    /// next word starts a relative jump table.
    bcctrs: Vec<(usize, usize, u64)>,
    /// The targets of the `bclr` words, from the TOCs of the latest
    /// round.
    targets: BTreeMap<u64, BTreeSet<u64>>,
    /// The constants that each `mtctr` copies into CTR on the paths to
    /// it, by segment index and address, from the TOCs of the latest
    /// round. For a GPR for which [`JoinFlow::holds_only_tocs`] holds,
    /// the constants also hold each TOC that a descriptor gives.
    ctr_writes: BTreeMap<(usize, u64), Value>,
    /// Each `mtctr` that can also copy a constant that [`JoinFlow`]
    /// dropped, by segment index and address, with the origins of the
    /// GPR, from the TOCs of the latest round.
    ///
    /// The map leaves out a GPR in these cases:
    /// - [`JoinFlow::holds_only_tocs`] holds for it;
    /// - [`Self::progressions`] is true and [`JoinFlow::progression`]
    ///   gives classes for it.
    lost_ctr_writes: BTreeMap<(usize, u64), Origins>,
    /// The address of each `bclr` whose LR can hold a constant that
    /// [`JoinFlow`] dropped, with the origins of LR, from the TOCs of the
    /// latest round.
    ///
    /// The map leaves out an LR in these cases:
    /// - [`JoinFlow::holds_only_tocs`] holds for it;
    /// - [`Self::progressions`] is true and [`JoinFlow::progression`]
    ///   gives classes for it.
    lost_returns: BTreeMap<u64, Origins>,
    /// Each `mtctr` that can also copy a lost value for which
    /// [`JoinFlow::progression`] gives classes, by segment index and
    /// address, from the TOCs of the latest round. The bound names each
    /// word of those classes.
    ranged_ctr_writes: BTreeSet<(usize, u64)>,
    /// The address of each `rfid` that [`Constants::translated_rfid_lost`]
    /// gives, with the origins of SRR0, and of each `hrfid` that
    /// [`Constants::translated_hrfid_lost`] gives, with the origins of the
    /// SPR cell, from the TOCs of the latest round.
    lost_rfids: BTreeMap<u64, Origins>,
    /// The address of each `bclr` that the paths reach only with LR
    /// constants, from the [`JoinFlow`] run of the latest round.
    closed_bclrs: BTreeSet<u64>,
    /// The address of each `mtctr` whose GPR can also hold a value with
    /// no constant and no mark, from the [`JoinFlow`] run of the latest
    /// round.
    open_ctr_writes: BTreeSet<u64>,
    /// Whether an `mtctr` or a `bclr` takes a lost value that keeps one
    /// progression as the classes of [`JoinFlow::progression`].
    ///
    /// The bound sets it only when one of these is true:
    /// - the refinement of the joins can raise no more bounds;
    /// - the refinement ran [`REFINEMENTS`] times.
    ///
    /// A raised join keeps each constant, which names fewer words than a
    /// class. The named words only grow, so the bound does not name the
    /// words of a class before that.
    progressions: bool,
    /// Whether the latest round took as lost a value that keeps one
    /// progression, because [`Self::progressions`] was false.
    deferred: bool,
    /// Each class whose words the bound names.
    residues: BTreeSet<Residue>,
    /// The word at each value that an unaligned, byte, halfword or
    /// byte-reversed load of the file bytes gives, which an indirect
    /// branch can enter.
    loaded: EntryWords,
    /// Each word at the real address of a value of [`Self::loaded`],
    /// which an indirect branch in real mode can enter.
    loaded_real: EntryWords,
    /// Whether a `bcctr` or a `bclr` can run in real mode with a target
    /// that the bound does not hold, as [`branches_in_real_mode`] gives
    /// it.
    ///
    /// While it is false, the entry words and the switch scan leave out
    /// [`Self::real_named`] and [`Self::loaded_real`]. When the bound
    /// sets it, the rounds run again.
    real_branches: bool,
    /// Whether a `bcctr` or a `bclr` can run in 32-bit mode, or an
    /// unbounded 32-bit switch can put control at any word.
    ///
    /// While it is false, the entry words and the switch scan leave out
    /// [`Self::low_named`]. When the bound sets it, the rounds run again.
    low_branches: bool,
    /// Each word where a real-mode switch or a 32-bit switch of a
    /// latest scan puts control.
    switched: BTreeSet<u64>,
    /// Each word at a real address of a target of [`Self::targets`] of a
    /// latest round, other than the target word itself.
    ///
    /// A `bcctr` or `bclr` whose targets the bound holds can run in real
    /// mode, and then enters these words. So the entry words hold them
    /// whatever [`Self::real_branches`] is. The set only grows.
    target_aliases: BTreeSet<u64>,
    /// Each word that [`return_aliases`] gives: a word at a real address
    /// of a return point, other than the return point itself.
    ///
    /// A `bclr` in real mode that returns from a call enters these words,
    /// also when the bound takes the `bclr` as closed.
    return_aliases: BTreeSet<u64>,
}

/// How an `mtctr` or a `bclr` takes a lost value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LostUse {
    /// The branch can reach an address that the scan does not know.
    Lost,
    /// The value is a constant of the GPR or of LR, or a TOC that a
    /// descriptor gives, as [`JoinFlow::holds_only_tocs`] states.
    Tocs,
    /// The value is in a class whose words the bound names, as
    /// [`JoinFlow::progression`] gives it.
    Progression,
}

/// Joins `origins` into the origins of `key` in `lost`.
fn note<K: Ord>(lost: &mut BTreeMap<K, Origins>, key: K, origins: &Origins) {
    lost.entry(key)
        .and_modify(|own| *own = own.union(origins))
        .or_insert_with(|| origins.clone());
}

impl Bound<'_> {
    /// Runs the rounds, the CTR flow and the switch scan until `reach`
    /// stops growing, and returns the bound without its lost origins.
    ///
    /// After the rounds, `reach` takes the words that each target and
    /// each switch reaches. When it grows, the rounds run again.
    fn reached(
        &mut self,
        reach: &mut Reach<'_>,
        words: &mut FunctionWords,
        join_words: &JoinWords<'_>,
        every: &Value,
        bounds: &Bounds,
    ) -> Result<IndirectBound, CallerScanError> {
        let code = self.code;
        loop {
            self.rounds(reach, words, join_words, every, bounds)?;
            let lost_writes: BTreeSet<(usize, u64)> =
                self.lost_ctr_writes.keys().copied().collect();
            let writes = CtrWrites {
                constants: &self.ctr_writes,
                lost: &lost_writes,
                ranged: &self.ranged_ctr_writes,
            };
            let mut flow = CtrFlow::new(code, writes, reach);
            flow.run();
            let mut lost_targets = flow.bcctr_targets(&mut self.targets);
            lost_targets.extend(flow.bclr_targets(&mut self.targets));
            lost_targets.extend(self.lost_returns.keys());
            lost_targets.extend(self.lost_rfids.keys());
            let mut closed_branches = flow.closed_bcctrs(&self.open_ctr_writes);
            closed_branches.extend(&self.closed_bclrs);
            let mut entries = self.real_mode.clone();
            entries.extend(&self.named);
            // The switch scan takes a word of `real_named` or `low_named`
            // as a join only when its flag is set, as the entry words do.
            let gated = |open: bool, words: &BTreeSet<u64>| {
                if open {
                    words.clone()
                } else {
                    BTreeSet::new()
                }
            };
            let mut indirect = IndirectBound {
                entries,
                real_named: gated(self.real_branches, &self.real_named),
                low_named: gated(self.low_branches, &self.low_named),
                targets: self.targets.clone(),
                lost_targets,
                closed_branches,
                ..IndirectBound::default()
            };
            let switches = Switches::find(code, &words.tocs, every, &indirect, reach);
            indirect.real_named.clone_from(&self.real_named);
            indirect.low_named.clone_from(&self.low_named);
            let starts = indirect
                .targets
                .values()
                .flatten()
                .chain(&switches.targets)
                .chain(&switches.mode32)
                .copied();
            let grew = reach.grow(starts);
            let switched = self.switch(&switches);
            let aliased = self.alias_targets();
            if grew || switched || aliased {
                continue;
            }
            indirect.real_mode_switches = switches.targets;
            indirect.unbounded_switches = switches.unbounded;
            indirect.mode32_switches = switches.mode32;
            indirect.unbounded_mode32_switches = switches.unbounded_mode32;
            return Ok(indirect);
        }
    }

    /// Runs the rounds of the bound until a round names no new word and
    /// `reach` takes no new word.
    ///
    /// Each round first adds to `reach` the words that the named words and
    /// the targets of the last round reach. The rounds run at least once.
    ///
    /// `bounds` holds the raised bounds of `JoinFlow`. The origins that
    /// the last round records name the drops of its `JoinFlow` run.
    fn rounds(
        &mut self,
        reach: &mut Reach<'_>,
        words: &mut FunctionWords,
        join_words: &JoinWords<'_>,
        every: &Value,
        bounds: &Bounds,
    ) -> Result<(), CallerScanError> {
        let code = self.code;
        let mut stepped = None;
        loop {
            let before = self.named_lens();
            let starts = self
                .named
                .iter()
                .chain(&self.real_named)
                .chain(&self.low_named)
                .chain(self.targets.values().flatten())
                .copied();
            let grew = reach.grow(starts);
            if !grew && stepped == Some(before) {
                return Ok(());
            }
            stepped = Some(before);
            self.tables_after_reached_bcctrs(reach);
            let named: BTreeSet<u64> = self
                .named
                .iter()
                .chain(&self.real_named)
                .chain(&self.low_named)
                .copied()
                .collect();
            words.grow(&named);
            self.targets.clear();
            self.ctr_writes.clear();
            self.lost_ctr_writes.clear();
            self.ranged_ctr_writes.clear();
            self.lost_returns.clear();
            self.lost_rfids.clear();
            self.closed_bclrs.clear();
            self.open_ctr_writes.clear();
            self.deferred = false;
            // The round names no new word when it is the last one, so the
            // run that counts the drops gets the same entries.
            let entries = self.entry_words();
            for (index, segment) in code.segments().iter().enumerate() {
                let tocs = words.tocs.segment(index);
                self.code_constants(index, segment, tocs, every, None, reach);
            }
            let joins = JoinFlow::run(
                join_words,
                &words.tocs,
                every,
                bounds,
                reach,
                &entries,
                false,
            );
            for (index, segment) in code.segments().iter().enumerate() {
                let tocs = words.tocs.segment(index);
                self.code_constants(index, segment, tocs, every, Some(&joins), reach);
            }
            self.jump_tables()?;
        }
    }

    /// Adds a relative jump table in each [`Width`] at the word after each
    /// `bcctr` of [`Self::bcctrs`] that `reach` holds, and drops that
    /// `bcctr` from the list.
    fn tables_after_reached_bcctrs(&mut self, reach: &Reach<'_>) {
        let tables = &mut self.tables;
        self.bcctrs.retain(|&(index, word, address)| {
            if !reach.contains(index, word) {
                return true;
            }
            if let Some(start) = address.checked_add(4) {
                tables.extend([(start, Width::Full), (start, Width::Low)]);
            }
            false
        });
    }

    /// The words that control can enter from a path that [`JoinFlow`]
    /// does not follow.
    ///
    /// These are the words:
    /// - each named word;
    /// - each real-mode entry;
    /// - each word where a switch of [`Self::switched`] puts control;
    /// - each word at address 0 or its real alias, which a `bcctr` reaches
    ///   after a `bdnz` falls through;
    /// - each word that [`Self::loaded`] holds;
    /// - when [`Self::real_branches`] is set, each word of
    ///   [`Self::real_named`] and of [`Self::loaded_real`];
    /// - when [`Self::low_branches`] is set, each word of
    ///   [`Self::low_named`];
    /// - each word of [`Self::target_aliases`];
    /// - each word of [`Self::return_aliases`].
    ///
    /// Only an indirect branch in real mode reaches a word at the real
    /// address of a named address. A branch whose targets the bound holds
    /// reaches only the real addresses of those targets, which
    /// [`Self::target_aliases`] holds. Only an indirect branch in 32-bit
    /// mode, or an unbounded 32-bit switch, reaches the word at the
    /// low-order 32 bits of a named address. An interrupt and a bounded
    /// switch enter only the words that the list names on their own.
    ///
    /// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
    /// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
    fn entry_words(&self) -> EntryWords {
        let code = self.code;
        let mut entries = if self.real_branches {
            self.loaded_real.clone()
        } else {
            self.loaded.clone()
        };
        let real = self.real_named.iter().filter(|_| self.real_branches);
        let low = self.low_named.iter().filter(|_| self.low_branches);
        entries.add(
            code,
            self.named
                .iter()
                .chain(real)
                .chain(low)
                .chain(&self.real_mode)
                .chain(&self.switched)
                .chain(&self.target_aliases)
                .chain(&self.return_aliases)
                .copied()
                .chain(code.real_aliases(0)),
        );
        entries
    }

    /// Adds to [`Self::switched`] each word where `switches` puts control,
    /// and returns whether the set grew.
    ///
    /// A real-mode switch puts control at each word at the real address of
    /// its target. The set only grows, so the rounds that it starts again
    /// end.
    fn switch(&mut self, switches: &Switches) -> bool {
        let code = self.code;
        let before = self.switched.len();
        self.switched.extend(
            switches
                .targets
                .iter()
                .flat_map(|&target| code.real_aliases(target))
                .chain(switches.mode32.iter().copied()),
        );
        self.switched.len() != before
    }

    /// Adds to [`Self::target_aliases`] each word at a real address of a
    /// target of [`Self::targets`], other than the target word itself,
    /// and returns whether the set grew.
    ///
    /// The bound does not know which branch reaches a target, or whether
    /// that branch runs in real mode, so it takes each target. The set
    /// only grows, so the rounds that it starts again end.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
    /// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
    fn alias_targets(&mut self) -> bool {
        let code = self.code;
        let before = self.target_aliases.len();
        for &target in self.targets.values().flatten() {
            self.target_aliases
                .extend(code.real_aliases(target).filter(|&alias| alias != target));
        }
        self.target_aliases.len() != before
    }

    /// Sets each of [`Self::real_branches`] and [`Self::low_branches`] that
    /// `indirect` shows a branch for, and returns whether it set one.
    ///
    /// The bound starts with both flags unset, so it reaches the least
    /// set of entry words that the branches it finds need. No round clears
    /// a flag, so the bound runs again at most twice.
    fn open_alias_gates(&mut self, indirect: &IndirectBound) -> bool {
        let code = self.code;
        let real = !self.real_branches && branches_in_real_mode(code, indirect, &self.real_mode);
        let low = !self.low_branches
            && (!indirect.unbounded_mode32_switches.is_empty()
                || branches_in_mode32(code, indirect));
        self.real_branches |= real;
        self.low_branches |= low;
        real || low
    }

    /// Sets [`Self::progressions`] when the latest round deferred a lost
    /// value that keeps one progression, and returns whether it did.
    fn take_progressions(&mut self) -> bool {
        if self.progressions || !self.deferred {
            return false;
        }
        self.progressions = true;
        true
    }

    /// How an `mtctr` or a `bclr` takes a lost value with the mark
    /// `origins`, from the [`JoinFlow`] run `joins` that gave the mark.
    ///
    /// For [`LostUse::Progression`], the bound names each word of each
    /// class of the mark. Each word that a branch to a value of a class
    /// can enter, in each mode, is such a word.
    fn lost_use(&mut self, joins: Option<&JoinFlow<'_>>, origins: &Origins) -> LostUse {
        let Some(joins) = joins else {
            return LostUse::Lost;
        };
        if joins.holds_only_tocs(origins) {
            return LostUse::Tocs;
        }
        let Some(residues) = joins.progression(origins) else {
            return LostUse::Lost;
        };
        if !self.progressions {
            self.deferred = true;
            return LostUse::Lost;
        }
        let code = self.code;
        for residue in residues {
            if self.residues.insert(residue) {
                self.named.extend(residue.words(code));
            }
        }
        LostUse::Progression
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
    /// - keeps each `mtctr` and each `bclr` that takes a lost value, as
    ///   [`Self::lost_use`] takes its mark;
    /// - enters each word that an `rfid` can enter with MSR\[IR\] at 1;
    /// - keeps each `rfid` that can enter a word with MSR\[IR\] at 1 from
    ///   an SRR0 that can hold a constant that [`JoinFlow`] dropped;
    /// - keeps each `hrfid` that a path reaches with a lost mark in the
    ///   SPR cell, which holds HSRR0.
    ///
    /// `index` is the index of `segment` in the code. `tocs` holds the
    /// TOC of each word of `segment`, as [`FunctionWords`] gives it.
    /// `every` holds each TOC that a descriptor gives. On each word, r2
    /// can hold the constants of the path and the TOCs of the word.
    ///
    /// Without `joins`, the scan follows each straight-line path. With
    /// `joins`, each join word starts with what [`JoinFlow`] gives it.
    ///
    /// The scan steps only the words that `reach` holds. A word that
    /// `reach` does not hold names nothing, and the next word starts with
    /// no constant.
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
        reach: &Reach<'_>,
    ) {
        let mut constants = Constants::default();
        for (word, ((address, raw), &toc)) in segment.addressed().zip(tocs).enumerate() {
            if !reach.contains(index, word) {
                constants = Constants::default();
                continue;
            }
            if let Some(entered) = joins.and_then(|joins| joins.entered((index, word))) {
                constants = entered;
            }
            if joins.is_some_and(|joins| joins.enters_unheld((index, word))) {
                constants.enter_unheld();
            }
            constants.enter_toc(toc, every);
            let decoded = crate::decode::decode(raw);
            if let Ok(PpuInstruction::Bclr { .. }) = decoded {
                let only_constants = !constants.lr.is_empty()
                    && constants.lr_lost.is_none()
                    && !constants.lr_is_unheld();
                if joins.is_some() && only_constants {
                    self.closed_bclrs.insert(address);
                }
                for (&by, lr) in &constants.lr {
                    self.targets
                        .entry(by)
                        .or_default()
                        .extend(lr.constants().iter().map(|&lr| lr & !0b11));
                }
                if let Some(origins) = &constants.lr_lost {
                    if self.lost_use(joins, origins) == LostUse::Lost {
                        note(&mut self.lost_returns, address, origins);
                    }
                }
            }
            if let Ok(PpuInstruction::Mtctr { rs }) = decoded {
                if joins.is_some() && constants.unheld_gprs() & (1 << rs) != 0 {
                    self.open_ctr_writes.insert(address);
                }
                if let Some(value) = constants.gpr(rs) {
                    self.ctr_writes
                        .entry((index, address))
                        .and_modify(|own| *own = own.clone().union(value))
                        .or_insert_with(|| value.clone());
                }
                if let Some(origins) = constants.lost(rs) {
                    self.lost_ctr_write(index, address, joins, origins, every);
                }
            }
            for target in constants.translated_rfid_targets(raw) {
                self.enter(target);
            }
            if let Some(origins) = constants.translated_rfid_lost(raw) {
                note(&mut self.lost_rfids, address, origins);
            }
            if let Some(origins) = constants.translated_hrfid_lost(raw) {
                note(&mut self.lost_rfids, address, &origins);
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

    /// Keeps the `mtctr` at `address` of the segment at `index`, which
    /// copies a GPR with the lost mark `origins`, as [`Self::lost_use`]
    /// takes the mark.
    ///
    /// `every` holds each TOC that a descriptor gives. The bound keeps a
    /// dropped TOC in CTR as a constant, and keeps the `mtctr` of a
    /// progression as a ranged write. So a decrement or a stepped copy of
    /// CTR makes the `bcctr` lost, as for a kept constant.
    ///
    /// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
    fn lost_ctr_write(
        &mut self,
        index: usize,
        address: u64,
        joins: Option<&JoinFlow<'_>>,
        origins: &Origins,
        every: &Value,
    ) {
        match self.lost_use(joins, origins) {
            LostUse::Lost => note(&mut self.lost_ctr_writes, (index, address), origins),
            LostUse::Tocs => {
                self.ctr_writes
                    .entry((index, address))
                    .and_modify(|own| *own = own.clone().union(every))
                    .or_insert_with(|| every.clone());
            }
            LostUse::Progression => {
                self.ranged_ctr_writes.insert((index, address));
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
