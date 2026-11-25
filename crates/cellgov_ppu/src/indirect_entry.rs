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
//! On each word of a function that a descriptor names, and on each word
//! of a routine with no descriptor that a direct call from the function
//! reaches, r2 holds the TOC that the descriptor gives. The bound gives
//! no TOC to a word that functions with two different TOCs reach. The
//! bound does not hold an address that the code computes in another
//! way, such as from a value it loads at run time.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::format::elf::{ppc64_function_descriptor as descriptor, ELF64_E_ENTRY};
use cellgov_ps3_abi::hw::ppu_interrupt::INTERRUPT_VECTOR_REAL_ADDRESSES;
use cellgov_ps3_abi::hw::ppu_mmu::REAL_MODE_IGNORED_EA;

use crate::caller_census::{written_gprs, CallerScanError};
use crate::hypercall_cfg::{branch_target, Code, CodeSegment};
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

/// The TOC that a function descriptor in `files` gives each word of
/// `code` that it names as a function's first word.
///
/// A descriptor counts only when its environment is zero, as each LV2
/// descriptor's is. A word that two descriptors give different TOCs
/// gets [`FunctionToc::Disputed`].
///
/// [OFW-4.93 lv2_kernel a:0x80000000002765e4-0x8000000000276600] A call through a descriptor loads LR from its first doubleword and r2 from its second.
fn function_tocs(files: &[FileSegment<'_>], code: &Code) -> BTreeMap<u64, FunctionToc> {
    let mut tocs: BTreeMap<u64, FunctionToc> = BTreeMap::new();
    for [entry, toc, environment] in files.iter().flat_map(FileSegment::descriptors) {
        if environment != 0 || !code.holds(entry) {
            continue;
        }
        let known = tocs.entry(entry).or_insert(FunctionToc::Outside);
        *known = known.join(FunctionToc::Toc(toc));
    }
    tocs
}

/// The TOC that r2 holds on one word, from the functions that reach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FunctionToc {
    /// No function that a descriptor names reaches the word.
    Outside,
    /// Each function that reaches the word has this TOC.
    Toc(u64),
    /// Functions with two different TOCs reach the word.
    Disputed,
}

impl FunctionToc {
    /// The TOC of a word that the functions of `self` and of `other`
    /// reach.
    fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Outside, toc) | (toc, Self::Outside) => toc,
            (Self::Toc(one), Self::Toc(other)) if one == other => self,
            _ => Self::Disputed,
        }
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
/// its first word with the steps of [`FunctionSteps`]. The steps do not
/// enter the first word of another function.
///
/// The walk errs wide: a word that gets a TOC in error can only add
/// words to the bound. An indirect branch reaches each named word, so
/// the walk goes on as the named words grow.
struct FunctionWords<'a> {
    code: &'a Code,
    /// Each function's first word, with its TOC.
    functions: &'a BTreeMap<u64, FunctionToc>,
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
    fn new(code: &'a Code, functions: &'a BTreeMap<u64, FunctionToc>) -> Self {
        let mut tocs = WordTocs::new(code);
        for (&first, &toc) in functions {
            tocs.join(first, toc);
        }
        Self {
            code,
            functions,
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

    /// Joins `toc` into the word at `address`, unless the word is the
    /// first word of a function.
    fn reach(&mut self, address: u64, toc: FunctionToc) {
        if !self.functions.contains_key(&address) && self.tocs.join(address, toc) {
            self.pending.push(address);
        }
    }
}

/// The steps that control can take from one word with r2 unchanged.
///
/// A direct branch that links steps to its target and to the next word,
/// where the call returns. The walk does not enter a target that is the
/// first word of a function, so only a routine with no descriptor gets
/// the TOC of its caller.
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

/// The words of `code` that control can enter other than by a direct
/// branch or a fall-through.
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
) -> Result<BTreeSet<u64>, CallerScanError> {
    let mut bound = Bound {
        code,
        files,
        named: BTreeSet::new(),
        tables: BTreeSet::new(),
    };
    for file in files {
        for value in file.doublewords() {
            bound.name(value);
        }
    }
    bound.tables.extend(tables_after_bcctr(code));
    let functions = function_tocs(files, code);
    let mut words = FunctionWords::new(code, &functions);
    loop {
        let before = bound.named.len();
        words.grow(&bound.named);
        for (index, segment) in code.segments().iter().enumerate() {
            bound.code_constants(segment, &functions, words.tocs.segment(index));
        }
        bound.jump_tables()?;
        if bound.named.len() == before {
            break;
        }
    }
    let mut entries = real_mode_entries(elf, code);
    entries.extend(bound.named);
    Ok(entries)
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
}

impl Bound<'_> {
    /// Adds `address` to the bound when it is a word of the code.
    fn enter(&mut self, address: u64) {
        if self.code.holds(address) {
            self.named.insert(address);
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

    /// Names each constant that the code of `segment` builds in a GPR.
    ///
    /// `tocs` holds the TOC of each word of `segment`, as
    /// [`FunctionWords`] gives it. r2 gets the TOC at these words:
    /// - the first word of a function, which `functions` holds;
    /// - another word of a function, when the path holds no constant in
    ///   r2.
    fn code_constants(
        &mut self,
        segment: &CodeSegment,
        functions: &BTreeMap<u64, FunctionToc>,
        tocs: &[FunctionToc],
    ) {
        let mut constants = Constants::default();
        for ((address, raw), &toc) in segment.addressed().zip(tocs) {
            if let FunctionToc::Toc(toc) = toc {
                if constants.gprs[2].is_none() || functions.contains_key(&address) {
                    constants.gprs[2] = Some(toc);
                }
            }
            if let Some(value) = constants.step(address, raw) {
                self.name(value);
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

/// The constants that the code builds in its GPRs and in LR on one
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
            _ if never_falls_through(&decoded) => *self = Self::default(),
            _ => self.forget_written(&decoded),
        }
        None
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
