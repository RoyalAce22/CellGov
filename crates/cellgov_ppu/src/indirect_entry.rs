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
//! An indirect branch ignores bits 62 and 63 of its target. So for a
//! doubleword or a built constant, the bound holds the word at the
//! address with those bits at 0. A jump-table entry adds its target only
//! when that target is itself the address of a word.
//!
//! A function reaches each word that control can get to from its first
//! word with r2 unchanged. On each word that a function reaches, r2 can
//! hold the TOC that the function's descriptor gives. On a word that
//! functions with two different TOCs reach, r2 can hold each TOC that a
//! descriptor gives. The bound does not hold an address that the code
//! computes in another way, such as from a value it loads at run time.
//!
//! The scan also keeps these addresses:
//! - the target of each `bclr` whose LR holds a constant that a
//!   straight-line path builds;
//! - the target of each `bcctr` from each constant that a path to it
//!   leaves in CTR;
//! - each `bcctr` that a path reaches with a CTR that the scan cannot
//!   prove: a CTR after a call, or after a decrement of a constant that
//!   the scan records.
//!
//! The CTR paths go across direct branches as well as fall-throughs, so
//! a branch to a later word brings its CTR to that word.
//!
//! A second scan finds where an `rfid`, `mtmsr` or `mtmsrd` can put
//! control into real mode. It forgets each constant at a word that
//! control can enter other than by a fall-through, so a constant that it
//! keeps is the value on every path to the word.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use cellgov_ps3_abi::format::elf::{ppc64_function_descriptor as descriptor, ELF64_E_ENTRY};
use cellgov_ps3_abi::hw::ppc_isa::PPC_BO_BIT2;
use cellgov_ps3_abi::hw::ppu_interrupt::INTERRUPT_VECTOR_REAL_ADDRESSES;
use cellgov_ps3_abi::hw::ppu_mmu::REAL_MODE_IGNORED_EA;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_BIT1, MSR_IR, MSR_PR, MSR_SF, SPR_SRR0, SPR_SRR1};

use crate::caller_census::{written_gprs, CallerScanError};
use crate::hypercall_cfg::{branch_target, Code, CodeSegment};
use crate::instruction::{spr_number, PpuInstruction};
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

    /// The code address, the TOC and the environment of a function
    /// descriptor that starts at each word-aligned address.
    fn descriptors(&self) -> impl Iterator<Item = [u64; 3]> + '_ {
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

/// The TOCs that the function descriptors in `files` give each word of
/// `code` that they name as a function's first word.
///
/// A descriptor counts only when its environment is zero, as each LV2
/// descriptor's is. Data that only looks like a descriptor also counts,
/// because the scan cannot tell it from a descriptor. An extra TOC can
/// only add words to the bound.
///
/// [OFW-4.93 lv2_kernel a:0x80000000002765e4-0x8000000000276600] A call through a descriptor loads LR from its first doubleword and r2 from its second.
fn function_tocs(files: &[FileSegment<'_>], code: &Code) -> BTreeMap<u64, BTreeSet<u64>> {
    let mut tocs: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    for [entry, toc, environment] in files.iter().flat_map(FileSegment::descriptors) {
        if environment == 0 && code.holds(entry) {
            tocs.entry(entry).or_default().insert(toc);
        }
    }
    tocs
}

/// The TOCs that r2 can hold on one word, from the functions that reach
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FunctionToc {
    /// No function that a descriptor names reaches the word.
    Outside,
    /// Each function that reaches the word has this TOC.
    Toc(u64),
    /// Functions with two different TOCs reach the word, so r2 can hold
    /// each TOC that a descriptor gives.
    Every,
}

impl FunctionToc {
    /// The TOC of a word that the functions of `self` and of `other`
    /// reach.
    fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Outside, toc) | (toc, Self::Outside) => toc,
            (Self::Toc(one), Self::Toc(other)) if one == other => self,
            _ => Self::Every,
        }
    }

    /// The constants that r2 can hold on a word with this TOC, when the
    /// straight-line path to the word leaves `path` in r2.
    ///
    /// `every` holds each TOC that a descriptor gives.
    fn in_r2(self, path: Option<Value>, every: &Value) -> Option<Value> {
        let tocs = match self {
            Self::Outside => return path,
            Self::Toc(toc) => Value::One(toc),
            Self::Every => every.clone(),
        };
        Some(match path {
            Some(path) => path.union(&tocs),
            None => tocs,
        })
    }
}

/// The constants that a register can hold on a straight-line path.
///
/// A register holds more than one constant only when the path builds it
/// from r2 on a word where r2 holds more than one: a word that functions
/// with two different TOCs reach, or a word whose TOC differs from the
/// constant that the path leaves in r2.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    /// One constant.
    One(u64),
    /// The constants, in ascending order, when there are none or two or
    /// more.
    Many(Rc<[u64]>),
}

impl Value {
    /// The value that holds each constant of `values`.
    ///
    /// A value from an empty `values` holds no constant.
    fn sorted(mut values: Vec<u64>) -> Self {
        values.sort_unstable();
        values.dedup();
        match *values.as_slice() {
            [one] => Self::One(one),
            _ => Self::Many(values.into()),
        }
    }

    /// The constants, in ascending order.
    fn constants(&self) -> &[u64] {
        match self {
            Self::One(one) => std::slice::from_ref(one),
            Self::Many(many) => many,
        }
    }

    /// The value that holds `step` of each constant.
    fn map(&self, step: impl Fn(u64) -> u64) -> Self {
        match self {
            Self::One(one) => Self::One(step(*one)),
            Self::Many(many) => Self::sorted(many.iter().map(|&constant| step(constant)).collect()),
        }
    }

    /// The value that holds each constant of `self` and of `other`.
    fn union(self, other: &Self) -> Self {
        if self.holds_all(other) {
            self
        } else if other.holds_all(&self) {
            other.clone()
        } else {
            Self::sorted([self.constants(), other.constants()].concat())
        }
    }

    /// Whether `self` holds each constant of `other`.
    fn holds_all(&self, other: &Self) -> bool {
        if let (Self::Many(one), Self::Many(two)) = (self, other) {
            if Rc::ptr_eq(one, two) {
                return true;
            }
        }
        other
            .constants()
            .iter()
            .all(|constant| self.constants().binary_search(constant).is_ok())
    }
}

/// The [`FunctionToc`] of each word of the code, in one vector for each
/// code segment.
#[derive(Debug)]
struct WordTocs {
    segments: Vec<(u64, Vec<FunctionToc>)>,
}

impl WordTocs {
    fn new(code: &Code) -> Self {
        Self {
            segments: code
                .segments()
                .iter()
                .map(|segment| {
                    (
                        segment.vaddr,
                        vec![FunctionToc::Outside; segment.words.len()],
                    )
                })
                .collect(),
        }
    }

    /// The TOC of each word of the code segment at `index`.
    fn segment(&self, index: usize) -> &[FunctionToc] {
        &self.segments[index].1
    }

    /// The TOC that each segment keeps for its word at `address`.
    fn at(&mut self, address: u64) -> impl Iterator<Item = &mut FunctionToc> + '_ {
        self.segments.iter_mut().filter_map(move |(vaddr, tocs)| {
            let offset = address.checked_sub(*vaddr)?;
            if offset % 4 != 0 {
                return None;
            }
            tocs.get_mut(usize::try_from(offset / 4).ok()?)
        })
    }

    /// The TOC of the word at `address`.
    fn get(&mut self, address: u64) -> FunctionToc {
        self.at(address)
            .fold(FunctionToc::Outside, |toc, known| toc.join(*known))
    }

    /// Joins `toc` into the word at `address`, and returns whether the
    /// word's TOC changed.
    fn join(&mut self, address: u64, toc: FunctionToc) -> bool {
        let mut changed = false;
        for known in self.at(address) {
            let joined = known.join(toc);
            changed |= joined != *known;
            *known = joined;
        }
        changed
    }
}

/// The TOC of each word of the code that is in a function that a
/// descriptor names.
///
/// The words of a function are the words that control can reach from
/// its first word with the steps of [`FunctionSteps`]. The steps also
/// enter the first word of another function. So a word in the middle of
/// a function keeps that function's TOC when a descriptor names it.
///
/// The walk errs wide: a word that gets a TOC in error can only add
/// words to the bound. An indirect branch reaches each named word, so
/// the walk goes on as the named words grow.
struct FunctionWords<'a> {
    code: &'a Code,
    tocs: WordTocs,
    /// The words whose TOC changed and whose steps the walk has not
    /// taken since.
    pending: Vec<u64>,
    /// The join of the TOCs of the indirect branches that the walk
    /// reached.
    indirect: FunctionToc,
    /// The named words that already have [`Self::indirect`].
    carried: BTreeSet<u64>,
}

impl<'a> FunctionWords<'a> {
    /// The walk before its first step, with the TOCs that `functions`
    /// gives each function's first word.
    fn new(code: &'a Code, functions: &BTreeMap<u64, BTreeSet<u64>>) -> Self {
        let mut tocs = WordTocs::new(code);
        for (&first, first_tocs) in functions {
            for &toc in first_tocs {
                tocs.join(first, FunctionToc::Toc(toc));
            }
        }
        Self {
            code,
            tocs,
            pending: functions.keys().copied().collect(),
            indirect: FunctionToc::Outside,
            carried: BTreeSet::new(),
        }
    }

    /// Walks the steps from each word whose TOC changed, with an
    /// indirect branch that reaches each word of `named`.
    fn grow(&mut self, named: &BTreeSet<u64>) {
        self.carry(named);
        while let Some(at) = self.pending.pop() {
            let toc = self.tocs.get(at);
            let code = self.code;
            for raw in code.words_at(at) {
                let steps = FunctionSteps::of(at, raw);
                if steps.next {
                    if let Some(next) = at.checked_add(4) {
                        self.reach(next, toc);
                    }
                }
                if let Some(target) = steps.target {
                    self.reach(target, toc);
                }
                if steps.indirect && self.indirect.join(toc) != self.indirect {
                    self.indirect = self.indirect.join(toc);
                    self.carried.clear();
                    self.carry(named);
                }
            }
        }
    }

    /// Gives [`Self::indirect`] to each word of `named` that does not
    /// have it yet.
    fn carry(&mut self, named: &BTreeSet<u64>) {
        if self.indirect == FunctionToc::Outside {
            return;
        }
        for &word in named {
            if self.carried.insert(word) {
                self.reach(word, self.indirect);
            }
        }
    }

    /// Joins `toc` into the word at `address`.
    fn reach(&mut self, address: u64, toc: FunctionToc) {
        if self.tocs.join(address, toc) {
            self.pending.push(address);
        }
    }
}

/// The steps that control can take from one word with r2 unchanged.
///
/// A direct branch that links steps to its target and to the next word,
/// where the call returns. The target gets the TOC of its caller, as
/// well as its own TOC when it is the first word of a function.
///
/// [PPC-Book1 p:24 s:2.4] The registers that a branch can alter are CTR and LR, so r2 at the target of a call is r2 at the call.
#[derive(Debug, PartialEq, Eq)]
struct FunctionSteps {
    /// Control can go to the next word.
    next: bool,
    /// The target of a direct branch.
    target: Option<u64>,
    /// The word is an indirect branch that does not link.
    indirect: bool,
}

impl FunctionSteps {
    /// The steps from the word `raw` at `address`.
    ///
    /// A word that does not decode can cause an exception that a handler
    /// emulates, so control can go to the next word.
    fn of(address: u64, raw: u32) -> Self {
        let Ok(decoded) = crate::decode::decode(raw) else {
            return Self::to_next(true);
        };
        let direct = |displacement: i64, aa: bool| Some(branch_target(address, displacement, aa));
        match decoded {
            PpuInstruction::B { offset, aa, link } => Self {
                next: link,
                target: direct(i64::from(offset), aa),
                indirect: false,
            },
            PpuInstruction::Bc {
                bo,
                offset,
                aa,
                link,
                ..
            } => Self {
                next: falls_through(bo, link),
                target: direct(i64::from(offset), aa),
                indirect: false,
            },
            PpuInstruction::Bclr { bo, link, .. } | PpuInstruction::Bcctr { bo, link, .. } => {
                Self {
                    next: falls_through(bo, link),
                    target: None,
                    indirect: !link,
                }
            }
            _ => Self::to_next(!never_falls_through(&decoded)),
        }
    }

    /// The steps of a word that is not a branch.
    fn to_next(next: bool) -> Self {
        Self {
            next,
            target: None,
            indirect: false,
        }
    }
}

/// The result of [`indirect_entries`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct IndirectBound {
    /// The words that control can enter other than by a direct branch or
    /// a fall-through, with the real-mode entries.
    pub(crate) entries: BTreeSet<u64>,
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
/// - an indirect branch in a function reaches each named word;
/// - r2 on the words of a function names more words.
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
        let before = bound.named.len();
        words.grow(&bound.named);
        bound.targets.clear();
        bound.ctr_writes.clear();
        for (index, segment) in code.segments().iter().enumerate() {
            bound.code_constants(index, segment, words.tocs.segment(index), &every);
        }
        bound.jump_tables()?;
        if bound.named.len() == before {
            break;
        }
    }
    let mut flow = CtrFlow::new(code, &bound.ctr_writes);
    flow.run();
    let lost_targets = flow.bcctr_targets(&mut bound.targets);
    let mut entries = real_mode_entries(elf, code);
    entries.extend(bound.named);
    let switches = Switches::find(code, &words.tocs, &every, &entries, &bound.targets);
    Ok(IndirectBound {
        entries,
        targets: bound.targets,
        lost_targets,
        real_mode_switches: switches.targets,
        unbounded_switches: switches.unbounded,
    })
}

/// The real-mode switches of the code.
#[derive(Debug, Default)]
struct Switches {
    /// The [`IndirectBound::real_mode_switches`].
    targets: BTreeSet<u64>,
    /// The [`IndirectBound::unbounded_switches`].
    unbounded: BTreeSet<u64>,
}

impl Switches {
    /// The switches of `code`, from the constants that each path to a
    /// switch builds.
    ///
    /// `tocs` holds the TOC of each word, and `every` holds each TOC that
    /// a descriptor gives. `entries` and `targets` are the entries and the
    /// indirect-branch targets of the bound.
    ///
    /// A switch target is also a word that control enters other than by
    /// a fall-through. So the scan repeats until no switch adds a target.
    /// The targets are words of the code, so the repeats end.
    fn find(
        code: &Code,
        tocs: &WordTocs,
        every: &Value,
        entries: &BTreeSet<u64>,
        targets: &BTreeMap<u64, BTreeSet<u64>>,
    ) -> Self {
        let mut joins = joins(code, entries, targets);
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
    ///
    /// A call can change each register before it returns, so the scan
    /// also forgets each constant after a branch that links. A `b` or a
    /// `bc` that links to the next word calls nothing, so the scan keeps
    /// the constants there.
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
                constants.step(address, raw);
                if calls(address, raw) {
                    constants = Constants::default();
                }
            }
        }
        switches
    }
}

/// The words of `code` that control can enter other than by a
/// fall-through, apart from the switch targets:
/// - each word of `entries`;
/// - each target of a direct branch;
/// - each target that `targets` gives.
///
/// Real mode ignores EA bits 0 and 1, so the set also holds each word at
/// the real address of each of these. The set omits the word after a
/// direct branch as a target of that branch. The branch reaches that
/// word with the constants of the fall-through.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
fn joins(
    code: &Code,
    entries: &BTreeSet<u64>,
    targets: &BTreeMap<u64, BTreeSet<u64>>,
) -> BTreeSet<u64> {
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
    entries
        .iter()
        .chain(targets.values().flatten())
        .flat_map(|&address| code.real_aliases(address))
        .chain(direct)
        .collect()
}

/// Whether the word `raw` at `address` is a branch that links, other
/// than a `b` or a `bc` that links to the next word.
///
/// A `bc` that links to the next word, such as `bcl 20,31,$+4`, reaches
/// the next word on the taken and on the fall-through path, so it runs
/// no routine.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:24 s:2.4] A taken `bc` sets NIA to its target, and a `bc` sets LR whether or not it branches.
fn calls(address: u64, raw: u32) -> bool {
    match crate::decode::decode(raw) {
        Ok(PpuInstruction::B { link, .. } | PpuInstruction::Bc { link, .. }) => {
            link && FunctionSteps::of(address, raw).target != address.checked_add(4)
        }
        Ok(PpuInstruction::Bclr { link, .. } | PpuInstruction::Bcctr { link, .. }) => link,
        _ => false,
    }
}

/// Where one `rfid`, `mtmsr` or `mtmsrd` can put control into real mode.
#[derive(Debug, PartialEq, Eq)]
enum Switch {
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
struct Bound<'a> {
    code: &'a Code,
    files: &'a [FileSegment<'a>],
    /// The words of the code that the image names, and the targets of
    /// the relative jump tables.
    named: BTreeSet<u64>,
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
    fn code_constants(
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
                self.name(constant);
            }
        }
    }

    /// Adds each target of each relative jump table that starts at an
    /// address of [`Self::tables`], and empties it.
    ///
    /// A table starts at a named address or at the word after a `bcctr`
    /// that does not link. Each entry is a 32-bit offset from the
    /// table's start. The run of entries ends at the first offset that:
    /// - reaches no word of the code; or
    /// - reaches back into the run itself.
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
                .filter(|target| !(start..=at).contains(target) && self.code.holds(*target))
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

/// The constants that the code builds in its GPRs, LR, SRR0 and SRR1 on
/// one straight-line path.
#[derive(Debug, Default)]
struct Constants {
    gprs: [Option<Value>; 32],
    lr: Option<Written>,
    srr0: Option<Value>,
    srr1: Option<Value>,
}

/// A constant in LR.
#[derive(Debug, Clone)]
struct Written {
    value: Value,
    /// The address of the word that wrote the constant.
    by: u64,
}

impl Constants {
    /// Steps over the word `raw` at `address`, and returns the constants
    /// it writes into a GPR.
    ///
    /// A word that does not fall through ends the straight-line path, so
    /// the step forgets every constant.
    fn step(&mut self, address: u64, raw: u32) -> Option<Value> {
        let Ok(decoded) = crate::decode::decode(raw) else {
            *self = Self::default();
            return None;
        };
        if let Some((gpr, value)) = self.written(decoded) {
            self.gprs[usize::from(gpr)].clone_from(&value);
            return value;
        }
        match decoded {
            PpuInstruction::Mtlr { rs } => self.lr = self.written_by(rs, address),
            PpuInstruction::B { link, .. } => self.after_branch(address, 0b10100, link),
            PpuInstruction::Bc { bo, link, .. }
            | PpuInstruction::Bclr { bo, link, .. }
            | PpuInstruction::Bcctr { bo, link, .. } => self.after_branch(address, bo, link),
            _ if never_falls_through(&decoded) => *self = Self::default(),
            PpuInstruction::Sc { .. } => {
                self.forget_srrs();
                self.forget_written(&decoded);
            }
            PpuInstruction::PpePrivileged { raw } => match MsrWord::of(raw) {
                Some(MsrWord::MtSrr0 { rs }) => self.srr0 = self.gpr(rs).cloned(),
                Some(MsrWord::MtSrr1 { rs }) => self.srr1 = self.gpr(rs).cloned(),
                _ => self.forget_written(&decoded),
            },
            _ => self.forget_written(&decoded),
        }
        None
    }

    /// Forgets the constants in SRR0 and SRR1.
    ///
    /// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt sets SRR0 and SRR1.
    fn forget_srrs(&mut self) {
        self.srr0 = None;
        self.srr1 = None;
    }

    /// Where the word `raw` at `address` can put control into real mode.
    ///
    /// These words give no address:
    /// - an `mtmsr` or `mtmsrd` with L = 1, which sets only MSR\[EE\]
    ///   and MSR\[RI\];
    /// - an `hrfid`, because the scan does not follow HSRR0 and HSRR1.
    ///
    /// The address depends on the MSR value only through MSR\[SF\] and
    /// MSR bit 1, so the step uses one MSR value of each kind. An MSR
    /// value that the scan does not know can clear IR and PR with SF at 0
    /// or at 1.
    ///
    /// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0 under the new MSR.
    /// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and MSR\[SF\] to RS bit 0 | RS bit 1.
    /// [PPC-Book3 p:91 s:8.3] `mtmsr` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and leaves MSR bits 0:31 unchanged.
    fn real_mode_switch(&self, address: u64, raw: u32) -> Switch {
        let Some((msr, next, sets_sf)) = self.msr_write(address, raw) else {
            return Switch::None;
        };
        let kinds: BTreeSet<u64> = match msr {
            Some(msr) => msr
                .constants()
                .iter()
                .filter(|&&msr| msr & (MSR_IR | MSR_PR) == 0)
                .map(|&msr| msr & (MSR_SF | MSR_BIT1))
                .collect(),
            None => BTreeSet::from([0, MSR_SF]),
        };
        if kinds.is_empty() {
            return Switch::None;
        }
        let Some(next) = next else {
            return Switch::Unbounded;
        };
        Switch::To(
            kinds
                .iter()
                .flat_map(|&msr| {
                    next.constants().iter().map(move |&next| {
                        if sets_sf {
                            fetch_address(next, msr)
                        } else {
                            next
                        }
                    })
                })
                .collect(),
        )
    }

    /// The MSR constants that the word `raw` at `address` sets MSR\[IR\]
    /// from, the address of the next fetch, and whether the word also
    /// sets MSR\[SF\], or `None` for a word that does not set MSR\[IR\].
    ///
    /// The MSR constants or the address are `None` when the scan does
    /// not know them.
    fn msr_write(&self, address: u64, raw: u32) -> Option<(Option<&Value>, Option<Value>, bool)> {
        let next = || address.checked_add(4).map(Value::One);
        Some(match MsrWord::of(raw)? {
            MsrWord::Rfid => (
                self.srr1.as_ref(),
                self.srr0.as_ref().map(|srr0| srr0.map(|srr0| srr0 & !0b11)),
                true,
            ),
            MsrWord::Mtmsrd { rs } => (self.gpr(rs), next(), true),
            MsrWord::Mtmsr { rs } => (self.gpr(rs), next(), false),
            MsrWord::MtSrr0 { .. } | MsrWord::MtSrr1 { .. } => return None,
        })
    }

    /// Forgets the constant in each GPR that `decoded` writes.
    fn forget_written(&mut self, decoded: &PpuInstruction) {
        let written = written_gprs(decoded);
        for (gpr, value) in self.gprs.iter_mut().enumerate() {
            if written >> gpr & 1 == 1 {
                *value = None;
            }
        }
    }

    /// The GPR that `decoded` writes and the constant it writes there,
    /// for a word that builds a constant.
    ///
    /// [PPC-Book1 p:51 s:3.3.8] `addi` and `addis` add EXTS(SI) or EXTS(SI || 0x0000) to RA, or to 0 when RA is 0.
    /// [PPC-Book1 p:66 s:3.3.11] `ori` and `oris` OR RS with UI or UI || 0x0000 and place the result into RA.
    /// [PPC-Book1 p:72 s:3.3.12.1] `rldicl` and `rldicr` rotate RS left by SH and AND it with MASK(MB, 63) or MASK(0, ME).
    /// [PPC-Book1 p:82 s:3.3.13] `mflr`, a form of `mfspr`, copies LR into RT.
    fn written(&self, decoded: PpuInstruction) -> Option<(u8, Option<Value>)> {
        Some(match decoded {
            PpuInstruction::Addi { rt, ra, imm } => (
                rt,
                self.stepped_base(ra, |value| value.wrapping_add_signed(i64::from(imm))),
            ),
            PpuInstruction::Addis { rt, ra, imm } => (
                rt,
                self.stepped_base(ra, |value| value.wrapping_add_signed(i64::from(imm) << 16)),
            ),
            PpuInstruction::Ori { ra, rs, imm } => {
                (ra, self.stepped_gpr(rs, |value| value | u64::from(imm)))
            }
            PpuInstruction::Oris { ra, rs, imm } => (
                ra,
                self.stepped_gpr(rs, |value| value | u64::from(imm) << 16),
            ),
            PpuInstruction::Rldicl { ra, rs, sh, mb, .. } => (
                ra,
                self.stepped_gpr(rs, |value| {
                    value.rotate_left(u32::from(sh)) & (u64::MAX >> mb)
                }),
            ),
            PpuInstruction::Rldicr { ra, rs, sh, me, .. } => (
                ra,
                self.stepped_gpr(rs, |value| {
                    value.rotate_left(u32::from(sh)) & (u64::MAX << (63 - me))
                }),
            ),
            PpuInstruction::Or { ra, rs, rb, .. } if rs == rb => (ra, self.gpr(rs).cloned()),
            PpuInstruction::Mflr { rt } => (rt, self.lr.as_ref().map(|lr| lr.value.clone())),
            _ => return None,
        })
    }

    fn gpr(&self, gpr: u8) -> Option<&Value> {
        self.gprs.get(usize::from(gpr))?.as_ref()
    }

    /// `step` of each constant in `gpr`.
    fn stepped_gpr(&self, gpr: u8, step: impl Fn(u64) -> u64) -> Option<Value> {
        self.gpr(gpr).map(|value| value.map(step))
    }

    /// `step` of each constant in `ra`, or of 0 when `ra` is 0.
    fn stepped_base(&self, ra: u8, step: impl Fn(u64) -> u64) -> Option<Value> {
        if ra == 0 {
            Some(Value::One(step(0)))
        } else {
            self.stepped_gpr(ra, step)
        }
    }

    /// The constants in `gpr`, which the word at `address` copies into LR.
    ///
    /// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr`, forms of `mtspr`, copy RS into LR or CTR.
    fn written_by(&self, gpr: u8, address: u64) -> Option<Written> {
        self.gpr(gpr).map(|value| Written {
            value: value.clone(),
            by: address,
        })
    }

    /// Steps over a branch at `address` with the BO field `bo`, which
    /// writes LR when `link` is set.
    ///
    /// The routine that a branch with `link` calls can write SRR0 and
    /// SRR1, so the step forgets them.
    ///
    /// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
    fn after_branch(&mut self, address: u64, bo: u8, link: bool) {
        if !falls_through(bo, link) {
            *self = Self::default();
            return;
        }
        if link {
            self.forget_srrs();
            self.lr = address.checked_add(4).map(|value| Written {
                value: Value::One(value),
                by: address,
            });
        }
    }
}

/// What the paths to one word bring in CTR.
///
/// A path can also bring a CTR that no word of the scan writes, such as
/// a value that the code loads. The bound takes such a CTR to hold a
/// named address, so the state does not record it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Ctr {
    /// The constants that the paths bring in CTR, by the address of the
    /// word that wrote each one.
    known: BTreeMap<u64, Value>,
    /// Whether a path brings a value that the scan cannot prove.
    lost: bool,
}

impl Ctr {
    /// CTR after the word at `by` writes the constants `value`.
    fn written(by: u64, value: Value) -> Self {
        Self {
            known: BTreeMap::from([(by, value)]),
            lost: false,
        }
    }

    /// CTR after a step that can leave it at any value.
    fn lost() -> Self {
        Self {
            known: BTreeMap::new(),
            lost: true,
        }
    }

    /// CTR after a step that keeps no constant of it.
    ///
    /// Each constant becomes a value that the scan cannot prove. A CTR
    /// that no word of the scan writes stays out of the state.
    fn unproved(&self) -> Self {
        Self {
            known: BTreeMap::new(),
            lost: self.lost || !self.known.is_empty(),
        }
    }

    /// Joins `other` into `self`, and returns whether `self` changed.
    fn join(&mut self, other: &Self) -> bool {
        let mut changed = other.lost && !self.lost;
        self.lost |= other.lost;
        for (&by, value) in &other.known {
            match self.known.get_mut(&by) {
                Some(own) if own.holds_all(value) => {}
                Some(own) => {
                    *own = own.clone().union(value);
                    changed = true;
                }
                None => {
                    self.known.insert(by, value.clone());
                    changed = true;
                }
            }
        }
        changed
    }
}

/// What the paths to each word of the code bring in CTR.
///
/// A path goes from a word to these words:
/// - the next word, when the word can fall through;
/// - each word at the real address of the target of a direct branch
///   that is not a call.
///
/// So a branch to a later word brings its CTR there. The flow does not
/// follow CTR into the routine that a call runs, or to the target of an
/// indirect branch, an `rfid` or an interrupt. Such a path brings a CTR
/// that the bound takes to hold a named address. The word after a call
/// gets a CTR that the scan cannot prove. The word after an `sc` gets
/// the CTR before the `sc`, so the flow takes the handler to return
/// with CTR unchanged.
///
/// The flow joins the CTR of each path to a word, and goes on until no
/// word changes. Each state only grows, from finite sets of constants,
/// so the flow ends.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
struct CtrFlow<'a> {
    code: &'a Code,
    /// The constants that each `mtctr` copies, as
    /// [`Bound::code_constants`] keeps them.
    writes: &'a BTreeMap<(usize, u64), Value>,
    /// What the paths to each word bring in CTR, in one vector for each
    /// code segment.
    at: Vec<Vec<Ctr>>,
}

impl<'a> CtrFlow<'a> {
    /// The flow before its first step, with no CTR on any word.
    fn new(code: &'a Code, writes: &'a BTreeMap<(usize, u64), Value>) -> Self {
        Self {
            code,
            writes,
            at: code
                .segments()
                .iter()
                .map(|segment| vec![Ctr::default(); segment.words.len()])
                .collect(),
        }
    }

    /// Steps from each word until no word changes.
    fn run(&mut self) {
        let code = self.code;
        let mut queued: Vec<Vec<bool>> = code
            .segments()
            .iter()
            .map(|segment| vec![true; segment.words.len()])
            .collect();
        let mut pending: Vec<(usize, usize)> = code
            .segments()
            .iter()
            .enumerate()
            .flat_map(|(index, segment)| (0..segment.words.len()).map(move |word| (index, word)))
            .rev()
            .collect();
        while let Some((index, word)) = pending.pop() {
            queued[index][word] = false;
            let segment = &code.segments()[index];
            let address = segment.vaddr + (word as u64) * 4;
            let (next, taken) = self.steps((index, word), address, segment.words[word]);
            let next = next.and_then(|ctr| Some((address.checked_add(4)?, ctr)));
            let taken = taken.into_iter().flat_map(|(target, ctr)| {
                code.real_aliases(target)
                    .map(move |alias| (alias, ctr.clone()))
            });
            for (to, ctr) in next.into_iter().chain(taken) {
                for (to_index, to_word) in positions(code, to) {
                    if self.at[to_index][to_word].join(&ctr) && !queued[to_index][to_word] {
                        queued[to_index][to_word] = true;
                        pending.push((to_index, to_word));
                    }
                }
            }
        }
    }

    /// The CTR that the word `raw` at `address` passes to the next word,
    /// and the target of a direct branch with the CTR that the branch
    /// passes there.
    ///
    /// `position` holds the index of the segment and of the word there. A
    /// word that does not decode passes no CTR, as the straight-line scan
    /// forgets each constant there.
    ///
    /// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr`, forms of `mtspr`, copy RS into LR or CTR.
    fn steps(
        &self,
        position: (usize, usize),
        address: u64,
        raw: u32,
    ) -> (Option<Ctr>, Option<(u64, Ctr)>) {
        let (index, word) = position;
        let ctr = &self.at[index][word];
        let Ok(decoded) = crate::decode::decode(raw) else {
            return (None, None);
        };
        match decoded {
            PpuInstruction::Mtctr { .. } => {
                let written = self.writes.get(&(index, address)).cloned();
                let ctr = written.map_or_else(Ctr::default, |value| Ctr::written(address, value));
                (Some(ctr), None)
            }
            PpuInstruction::B { offset, aa, link } => {
                let target = branch_target(address, i64::from(offset), aa);
                let next = branched(address, raw, 0b10100, link, ctr);
                (next, Some((target, taken(address, raw, 0b10100, ctr))))
            }
            PpuInstruction::Bc {
                bo,
                offset,
                aa,
                link,
                ..
            } => {
                let target = branch_target(address, i64::from(offset), aa);
                let next = branched(address, raw, bo, link, ctr);
                (next, Some((target, taken(address, raw, bo, ctr))))
            }
            PpuInstruction::Bclr { bo, link, .. } | PpuInstruction::Bcctr { bo, link, .. } => {
                (branched(address, raw, bo, link, ctr), None)
            }
            _ if never_falls_through(&decoded) => (None, None),
            _ => (Some(ctr.clone()), None),
        }
    }

    /// Adds the target of each `bcctr` to `targets`, by the word that
    /// wrote each constant that the paths to it bring in CTR, and returns
    /// the [`IndirectBound::lost_targets`].
    ///
    /// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
    fn bcctr_targets(&self, targets: &mut BTreeMap<u64, BTreeSet<u64>>) -> BTreeSet<u64> {
        let mut lost = BTreeSet::new();
        for (segment, ctrs) in self.code.segments().iter().zip(&self.at) {
            for ((address, raw), ctr) in segment.addressed().zip(ctrs) {
                if !matches!(crate::decode::decode(raw), Ok(PpuInstruction::Bcctr { .. })) {
                    continue;
                }
                for (&by, value) in &ctr.known {
                    targets
                        .entry(by)
                        .or_default()
                        .extend(value.constants().iter().map(|&ctr| ctr & !0b11));
                }
                if ctr.lost {
                    lost.insert(address);
                }
            }
        }
        lost
    }
}

/// The CTR that a direct branch at `address` passes to its target. The
/// branch is the word `raw`, with the BO field `bo`, and `ctr` is what
/// the paths to the branch bring.
///
/// A decrement leaves CTR at a value that the scan cannot prove, because
/// a loop can run it more than once. The flow does not follow CTR into
/// the routine that a call runs. The bound takes the routine to start
/// with a CTR that no word of the scan writes, as at an indirect entry.
///
/// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
fn taken(address: u64, raw: u32, bo: u8, ctr: &Ctr) -> Ctr {
    if calls(address, raw) {
        return Ctr::default();
    }
    if bo & PPC_BO_BIT2 == 0 {
        ctr.unproved()
    } else {
        ctr.clone()
    }
}

/// The CTR that a branch at `address` passes to the next word, or `None`
/// when it cannot fall through. The branch is the word `raw`, with the
/// BO field `bo`, and it writes LR when `link` is set. `ctr` is what the
/// paths to the branch bring.
///
/// A `bc` with BO 1a00t that does not link falls through only when the
/// decremented CTR is 0. A `bcctr` then reaches address 0 in either
/// mode, and the `bc` is the word that wrote CTR. The taken path brings
/// its own CTR to the target.
///
/// Each other decrement leaves CTR at a value that the scan cannot prove.
/// A call leaves CTR at any value, because its routine can write CTR
/// before it returns.
///
/// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
fn branched(address: u64, raw: u32, bo: u8, link: bool, ctr: &Ctr) -> Option<Ctr> {
    if !falls_through(bo, link) {
        return None;
    }
    Some(if calls(address, raw) {
        Ctr::lost()
    } else if bo & PPC_BO_BIT2 != 0 {
        ctr.clone()
    } else if bo & 0b10110 == 0b10000 && !link {
        Ctr::written(address, Value::One(0))
    } else {
        ctr.unproved()
    })
}

/// The index of the segment and of the word of each segment that holds a
/// word at `address`.
fn positions(code: &Code, address: u64) -> impl Iterator<Item = (usize, usize)> + '_ {
    code.segments()
        .iter()
        .enumerate()
        .filter_map(move |(index, segment)| {
            let offset = address.checked_sub(segment.vaddr)?;
            if offset % 4 != 0 {
                return None;
            }
            let word = usize::try_from(offset / 4).ok()?;
            (word < segment.words.len()).then_some((index, word))
        })
}

/// A privileged word that writes SRR0 or SRR1, or that sets MSR\[IR\].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MsrWord {
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
    fn of(raw: u32) -> Option<Self> {
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
fn fetch_address(address: u64, msr: u64) -> u64 {
    if msr & (MSR_SF | MSR_BIT1) != 0 {
        address
    } else {
        address & u64::from(u32::MAX)
    }
}

/// Whether control never goes from `decoded` to the next word, for a
/// word that is not a branch. These words never fall through:
/// - `rfid` and `hrfid`;
/// - a trap that traps for all operand values.
///
/// The bound errs wide, so a word that causes an exception which a
/// handler can emulate keeps the path. A word that the PPE does not
/// implement is such a word.
///
/// An `rfid` with SRR0 at the next word falls through. The reset there
/// is safe, because no constant is seen to cross an `rfid`.
///
/// [PPC-Book3 p:119 s:Appendix G] `rfid` and `hrfid` are XL-form words with primary opcode 19 and extended opcodes 18 and 274.
/// [PPC-Book3 p:13 s:2.3] `rfid` and `hrfid` fetch the next instruction from SRR0 or HSRR0.
/// [OFW-4.93 lv2_kernel a:0x8000000000000720-0x8000000000000744] This `rfid` returns to the next word, which r5 names, and the words after it build no constant.
/// [CBE-Handbook p:738 s:A.2.4.1] The PPE does not implement `popcntb`.
/// [CBE-Handbook p:739 s:A.2.4.1] An attempt to execute `popcntb`, `mcrxr` or another word that the PPE does not implement causes an illegal instruction exception.
/// [PPC-Book3 p:92 s:8.4] An `lq` with RT = RA invokes the illegal instruction error handler.
fn never_falls_through(decoded: &PpuInstruction) -> bool {
    match *decoded {
        PpuInstruction::PpePrivileged { raw } => {
            matches!((raw >> 26, (raw >> 1) & 0x3FF), (19, 18 | 274))
        }
        PpuInstruction::Tw { to, ra, rb } | PpuInstruction::Td { to, ra, rb } => {
            traps_always(to, ra == rb)
        }
        PpuInstruction::Twi { to, .. } | PpuInstruction::Tdi { to, .. } => traps_always(to, false),
        _ => false,
    }
}

/// Whether a trap with the TO field `to` traps for all operand values.
///
/// `same_operands` is set when the trap compares a register with itself.
/// Two operands are equal, or else each comparison puts them in one
/// order. So the trap traps always when TO has the equal bit and one of
/// these is true:
/// - the operands are the same register;
/// - TO has both signed order bits;
/// - TO has both unsigned order bits.
///
/// [PPC-Book1 p:62 s:3.3.10] TO bits 0 to 4 select signed less than, signed greater than, equal, unsigned less than and unsigned greater than.
fn traps_always(to: u8, same_operands: bool) -> bool {
    let equal = to & 0b00100 != 0;
    let signed_orders = to & 0b11000 == 0b11000;
    let unsigned_orders = to & 0b00011 == 0b00011;
    equal && (same_operands || signed_orders || unsigned_orders)
}

#[cfg(test)]
#[path = "tests/indirect_entry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/indirect_entry_partial_constant_tests.rs"]
mod partial_constant_tests;

#[cfg(test)]
#[path = "tests/indirect_entry_toc_tests.rs"]
mod toc_tests;

#[cfg(test)]
#[path = "tests/indirect_entry_path_end_tests.rs"]
mod path_end_tests;

#[cfg(test)]
#[path = "tests/indirect_entry_function_toc_tests.rs"]
mod function_toc_tests;

#[cfg(test)]
#[path = "tests/indirect_entry_toc_candidate_tests.rs"]
mod toc_candidate_tests;
