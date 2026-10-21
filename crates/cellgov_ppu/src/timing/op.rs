//! What the issue model needs to know of one instruction: its units, the
//! registers it reads and writes, and the latency of each write.
//!
//! Latencies follow Table A-1 and Table A-8 of the handbook, with the
//! console's measured values where the capture times a term.
//! [CBE-Handbook p:715 s:A.1.2] the latency of an instruction is its
//! issue-to-issue latency to a dependent instruction.

use crate::instruction::ops::{Fp59Op, Fp63Op, VaOp, VxOp};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BO_ALWAYS;
use cellgov_ps3_abi::hw::ppu_issue as k;

/// A register the model tracks: GPRs, FPRs, VRs, the eight CR fields,
/// LR, CTR, XER and FPSCR.
pub(crate) type Reg = u8;

/// The number of registers the model tracks.
pub(crate) const REGS: usize = 108;

const fn gpr(n: u8) -> Reg {
    n
}
const fn fpr(n: u8) -> Reg {
    32 + n
}
const fn vr(n: u8) -> Reg {
    64 + n
}
const fn crf(n: u8) -> Reg {
    96 + n
}
const LR: Reg = 104;
const CTR: Reg = 105;
const XER: Reg = 106;
const FPSCR: Reg = 107;
/// An unused slot of a register list.
pub(crate) const NO_REG: Reg = u8::MAX;

/// The execution units an instruction occupies, one bit each.
pub(crate) mod unit {
    pub(crate) const FXU: u8 = 1;
    pub(crate) const LSU: u8 = 2;
    pub(crate) const BRU: u8 = 4;
    /// Vector-scalar type 1: VXU simple, complex and float, FPU
    /// arithmetic.
    pub(crate) const VSU1: u8 = 8;
    /// Vector-scalar type 2: VXU load, store and permute, FPU load and
    /// store.
    pub(crate) const VSU2: u8 = 16;
}

/// How a load's result latency is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadClass {
    /// A fixed-point load.
    Fixed,
    /// A floating-point load.
    Float,
    /// A vector load.
    Vector,
    /// `lwarx` or `ldarx`, which always miss the L1.
    ///
    /// [CBE-Handbook p:717 s:A.1.2] `lwarx ldarx`: forced to miss the
    /// data L1 cache.
    Reserve,
}

/// Where a branch takes its target from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    /// The instruction's own displacement.
    Direct,
    /// LR, predicted by the link stack.
    Link,
    /// CTR.
    Count,
}

/// What an instruction does beyond reading and writing registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// No effect beyond its register reads and writes.
    Plain,
    /// Holds its units for this many cycles after it issues.
    Unpipelined(u16),
    /// Reads `len` bytes at `ea`.
    Load { ea: u64, len: u8, class: LoadClass },
    /// Writes `len` bytes at `ea`. A conditional store's CR field waits
    /// for the line's ownership.
    Store { ea: u64, len: u8, conditional: bool },
    /// Zeroes the line at `ea` in the L2 and drops it from the L1.
    ZeroLine { ea: u64 },
    /// A branch with this `BO` field.
    ///
    /// `taken` is its direction when the state it executes from decides
    /// it. A fused pair's branch reads the CR that its compare writes.
    /// Its `taken` is `None`, and its direction comes from where the
    /// pair goes.
    Branch {
        bo: u8,
        link: bool,
        target: Target,
        taken: Option<bool>,
    },
    /// Issues once every older result is ready.
    Drain,
    /// Drains, then refetches the instructions after it.
    Refetch,
    /// An `sc`. `number` is r11 before the `sc` executes, or `None`
    /// for a hypervisor call.
    Syscall { number: Option<u64> },
}

/// One instruction as the issue model sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Op {
    /// The units it occupies, from [`mod@unit`].
    pub(crate) units: u8,
    /// A vector permute, which may pair with a younger load or store.
    pub(crate) permute: bool,
    /// Registers read as data.
    pub(crate) srcs: [Reg; 8],
    /// Registers read as an address.
    pub(crate) addr: [Reg; 2],
    /// Registers written, each with its latency.
    pub(crate) dsts: [(Reg, u16); 3],
    /// GPRs written from this one up, with the count and latency: the
    /// destinations of a load multiple or load string.
    pub(crate) range: Option<(Reg, u8, u16)>,
    /// What it does beyond its registers.
    pub(crate) kind: Kind,
    /// Microwords, or zero for an instruction issued whole.
    pub(crate) words: u8,
    /// Cycles a microcode sequence holds issue. Its write latencies
    /// count from its first microword.
    pub(crate) span: u8,
}

impl Op {
    const fn on(units: u8) -> Self {
        Self {
            units,
            permute: false,
            srcs: [NO_REG; 8],
            addr: [NO_REG; 2],
            dsts: [(NO_REG, 0); 3],
            range: None,
            kind: Kind::Plain,
            words: 0,
            span: 0,
        }
    }

    fn src(mut self, r: Reg) -> Self {
        if let Some(slot) = self.srcs.iter_mut().find(|s| **s == NO_REG) {
            *slot = r;
        }
        self
    }

    /// Reads GPR `ra` as an address unless it is the literal zero of
    /// `(RA|0)`.
    fn base(mut self, ra: u8) -> Self {
        if ra != 0 {
            if let Some(slot) = self.addr.iter_mut().find(|s| **s == NO_REG) {
                *slot = gpr(ra);
            }
        }
        self
    }

    fn index(mut self, rb: u8) -> Self {
        if let Some(slot) = self.addr.iter_mut().find(|s| **s == NO_REG) {
            *slot = gpr(rb);
        }
        self
    }

    fn dst(mut self, r: Reg, lat: u64) -> Self {
        if let Some(slot) = self.dsts.iter_mut().find(|s| s.0 == NO_REG) {
            *slot = (r, lat as u16);
        }
        self
    }

    fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }

    /// The CR0 write of an `Rc = 1` form the decoder issues whole.
    fn record_cr0(self, rc: bool) -> Self {
        if rc {
            self.dst(crf(0), k::PPU_COMPARE_LATENCY_CYCLES)
        } else {
            self
        }
    }

    /// A microcode sequence of `words` microwords that holds issue for
    /// `span` cycles.
    fn microcode(mut self, words: u8, span: u8) -> Self {
        self.words = words;
        self.span = span;
        self
    }

    /// The microcoded `Rc = 1` form of an operation of latency `lat`:
    /// the base operation, then a compare that waits for it.
    ///
    /// [CBE-Handbook p:726 s:A.1.3.1] a CR-recording instruction is the
    /// base operation followed by a dependent compare.
    fn cr_recording(self, rt: Reg, lat: u64) -> Self {
        let span = lat + k::PPU_COMPARE_LATENCY_CYCLES;
        self.dst(rt, lat).dst(crf(0), span).microcode(2, span as u8)
    }

    /// A shift or rotate by a register: `mt_shr`, two no-ops, then the
    /// root instruction, and with `Rc = 1` a compare after it.
    ///
    /// [CBE-Handbook p:728 s:A.1.3.1] indirect shifts and rotates: 4
    /// microwords, 5 when recording.
    fn indirect_shift(self, ra: Reg, rc: bool) -> Self {
        let root = 3 + k::PPU_FX_LATENCY_CYCLES;
        if rc {
            let span = root + k::PPU_COMPARE_LATENCY_CYCLES;
            self.dst(ra, root)
                .dst(crf(0), span)
                .microcode(5, span as u8)
        } else {
            self.dst(ra, root).microcode(4, 4)
        }
    }

    /// The registers this one writes.
    pub(crate) fn writes(&self) -> impl Iterator<Item = Reg> + '_ {
        let range = self
            .range
            .into_iter()
            .flat_map(|(first, n, _)| (first..first.saturating_add(n)).filter(|r| *r < 32));
        self.dsts
            .iter()
            .map(|d| d.0)
            .filter(|r| *r != NO_REG)
            .chain(range)
    }

    /// Whether this one reads a register `other` writes.
    pub(crate) fn reads_any_of(&self, other: &Op) -> bool {
        other
            .writes()
            .any(|w| self.srcs.contains(&w) || self.addr.contains(&w))
    }
}

fn fx(rt: Reg) -> Op {
    Op::on(unit::FXU).dst(rt, k::PPU_FX_LATENCY_CYCLES)
}

fn ea_d(state: &PpuState, ra: u8, imm: i16) -> u64 {
    let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
    base.wrapping_add(imm as i64 as u64)
}

fn ea_x(state: &PpuState, ra: u8, rb: u8) -> u64 {
    let base = if ra == 0 { 0 } else { state.gpr[ra as usize] };
    base.wrapping_add(state.gpr[rb as usize])
}

fn load(units: u8, ea: u64, len: u8, class: LoadClass) -> Op {
    Op::on(units).kind(Kind::Load { ea, len, class })
}

fn store(units: u8, ea: u64, len: u8) -> Op {
    Op::on(units).kind(Kind::Store {
        ea,
        len,
        conditional: false,
    })
}

/// A fixed-point load's write of `rt`: the L1 latency to a dependent
/// fixed-point operation. The pipeline replaces it on a miss.
fn int_load(ea: u64, len: u8, rt: u8) -> Op {
    load(unit::LSU, ea, len, LoadClass::Fixed).dst(gpr(rt), k::PPU_LOAD_FX_USE_CYCLES)
}

/// A load or store with update also writes RA through the FXU.
///
/// [CBE-Handbook p:716 s:A.1.2] load and store with update: the
/// hardware breaks it into the basic load or store and an add.
fn update(op: Op, ra: u8) -> Op {
    let mut op = op.dst(gpr(ra), k::PPU_FX_LATENCY_CYCLES);
    op.units |= unit::FXU;
    op
}

const INT_STORE: u8 = unit::FXU | unit::LSU;
const VSU_MEM: u8 = unit::LSU | unit::VSU2;

/// A load algebraic, which the microcode handles by byte.
///
/// `lha` takes the console's microword count; the other forms take the
/// handbook's.
///
/// [CBE-Handbook p:729 s:A.1.3.1] `lha` 7 microwords, `lhau lhaux lhax`
/// 8, `lwa` 13, `lwaux lwax` 12; latency 11.
fn load_algebraic(ea: u64, len: u8, rt: u8, words: u8) -> Op {
    load(unit::LSU | unit::FXU, ea, len, LoadClass::Fixed)
        .dst(gpr(rt), words as u64 + k::PPU_LOAD_FX_USE_CYCLES)
        .microcode(words, words)
}

/// A load or store multiple, or a load or store string, of `bytes` bytes
/// from register `first`.
///
/// [CBE-Handbook p:729 s:A.1.3.1] `lmw stmw`: 2 + 1 x words microwords;
/// `lswi stswi`: 1 x words + 2 x bytes.
fn multiple(ea: u64, first: u8, bytes: u64, store_side: bool, string: bool) -> Op {
    let words = bytes / 4;
    let tail = bytes % 4;
    let micro = if string { words + 2 * tail } else { 2 + words };
    let micro = micro.clamp(1, u8::MAX as u64) as u8;
    let regs = bytes.div_ceil(4).min(32) as u8;
    let len = bytes.min(u8::MAX as u64) as u8;
    if store_side {
        let mut op = store(INT_STORE, ea, len).microcode(micro, micro);
        for r in 0..regs.min(4) {
            op = op.src(gpr((first + r) % 32));
        }
        op
    } else {
        let mut op = load(unit::LSU | unit::FXU, ea, len, LoadClass::Fixed).microcode(micro, micro);
        op.range = Some((
            gpr(first),
            regs,
            (micro as u64 + k::PPU_LOAD_FX_USE_CYCLES) as u16,
        ));
        op
    }
}

/// The cycles a divide of `ra` by `rb` takes, by the handbook's formula.
///
/// [CBE-Handbook p:718 s:A.1.2] PPU cycles equal
/// `((1 setup) + (ceil((rb leading digits - ra leading digits) / 2) + 1
/// iterations) + (1 fixup)) x 2`, word minimum 10 and maximum 38,
/// doubleword minimum 10 and maximum 70; overflow cases complete in 10.
fn divide_cycles(ra: u64, rb: u64, word: bool, signed: bool) -> u64 {
    let (a, b, max) = if word {
        let (a, b) = (ra as u32, rb as u32);
        let (a, b) = if signed {
            (
                (a as i32).unsigned_abs() as u64,
                (b as i32).unsigned_abs() as u64,
            )
        } else {
            (a as u64, b as u64)
        };
        (a << 32, b << 32, k::PPU_DIVIDE_WORD_MAX_CYCLES)
    } else if signed {
        (
            (ra as i64).unsigned_abs(),
            (rb as i64).unsigned_abs(),
            k::PPU_DIVIDE_DOUBLEWORD_MAX_CYCLES,
        )
    } else {
        (ra, rb, k::PPU_DIVIDE_DOUBLEWORD_MAX_CYCLES)
    };
    if b == 0 || a < b {
        return k::PPU_DIVIDE_MIN_CYCLES;
    }
    let span = u64::from(b.leading_zeros() - a.leading_zeros());
    let iterations = span.div_ceil(2) + 1;
    ((1 + iterations + 1) * 2).clamp(k::PPU_DIVIDE_MIN_CYCLES, max)
}

/// A divide; `(word, signed)` names the form.
fn divide(state: &PpuState, rt: u8, ra: u8, rb: u8, oe: bool, rc: bool, form: (bool, bool)) -> Op {
    let (word, signed) = form;
    let lat = divide_cycles(state.gpr[ra as usize], state.gpr[rb as usize], word, signed)
        + k::PPU_DIVIDE_REFETCH_CYCLES;
    let op = Op::on(unit::FXU).src(gpr(ra)).src(gpr(rb));
    let op = if oe { op.dst(XER, lat) } else { op };
    if rc {
        op.cr_recording(gpr(rt), lat)
    } else {
        op.dst(gpr(rt), lat).kind(Kind::Unpipelined(lat as u16))
    }
}

fn multiply(ra: u8, rb: Option<u8>, rt: u8, lat: u64, rc: bool) -> Op {
    let op = Op::on(unit::FXU).src(gpr(ra));
    let op = match rb {
        Some(rb) => op.src(gpr(rb)),
        None => op,
    };
    if rc {
        op.cr_recording(gpr(rt), lat)
    } else {
        op.dst(gpr(rt), lat).kind(Kind::Unpipelined(lat as u16))
    }
}

/// A fixed-point operation of latency 2 that writes `ra`, microcoded
/// when it records.
///
/// [CBE-Handbook p:727 s:A.1.3.1] logical, count-leading-zeros,
/// extend-sign and rotate-immediate recording forms are microcoded.
fn logical(op: Op, ra: u8, rc: bool) -> Op {
    if rc {
        op.cr_recording(gpr(ra), k::PPU_FX_LATENCY_CYCLES)
    } else {
        op.dst(gpr(ra), k::PPU_FX_LATENCY_CYCLES)
    }
}

fn cmp(bf: u8, ra: u8, rb: Option<u8>) -> Op {
    let op = Op::on(unit::FXU).src(gpr(ra));
    let op = match rb {
        Some(rb) => op.src(gpr(rb)),
        None => op,
    };
    op.dst(crf(bf), k::PPU_COMPARE_LATENCY_CYCLES)
}

/// A branch: it reads the CR field `bi` names unless `BO` ignores the
/// condition, and reads and writes CTR when `BO` decrements it.
///
/// [PPC-Book1 p:20 s:2.4.1] `BO` bit 0 (value 16) ignores the condition
/// and bit 2 (value 4) leaves CTR alone.
fn branch(bo: u8, bi: u8, link: bool, target: Target, taken: Option<bool>) -> Op {
    let mut op = Op::on(unit::BRU).kind(Kind::Branch {
        bo,
        link,
        target,
        taken,
    });
    if bo & 0x10 == 0 {
        op = op.src(crf(bi / 4));
    }
    if bo & 0x04 == 0 {
        op = op.src(CTR).dst(CTR, k::PPU_BRANCH_UNIT_LATENCY_CYCLES);
    }
    match target {
        Target::Link => op = op.src(LR),
        Target::Count => op = op.src(CTR),
        Target::Direct => {}
    }
    if link {
        op = op.dst(LR, k::PPU_BRANCH_UNIT_LATENCY_CYCLES);
    }
    op
}

/// Whether a conditional branch with `bo` and `bi` branches from
/// `state`.
///
/// [PPC-Book1 p:20 s:2.4.1] the branch is taken when the CTR clause
/// (decrement, then test against zero) and the condition clause (CR bit
/// `bi` against `BO` bit 1) both hold; `BO` bits 0 and 2 drop them.
fn branches(state: &PpuState, bo: u8, bi: u8) -> bool {
    let ctr_ok = bo & 0x04 != 0 || ((state.ctr().wrapping_sub(1) != 0) != (bo & 0x02 != 0));
    let bit = (state.cr() >> (31 - u32::from(bi))) & 1 != 0;
    let cond_ok = bo & 0x10 != 0 || bit == (bo & 0x08 != 0);
    ctr_ok && cond_ok
}

fn cr_logical(bt: u8, ba: u8, bb: u8) -> Op {
    Op::on(unit::BRU)
        .src(crf(ba / 4))
        .src(crf(bb / 4))
        .src(crf(bt / 4))
        .dst(crf(bt / 4), k::PPU_BRANCH_UNIT_LATENCY_CYCLES)
}

/// A vector operation's latency class.
fn vx_class(op: VxOp) -> (u8, bool, u64) {
    use VxOp as V;
    match op {
        V::Vmrghb
        | V::Vmrghh
        | V::Vmrghw
        | V::Vmrglb
        | V::Vmrglh
        | V::Vmrglw
        | V::Vpkuhum
        | V::Vpkuwum
        | V::Vpkuhus
        | V::Vpkuwus
        | V::Vpkshus
        | V::Vpkswus
        | V::Vpkshss
        | V::Vpkswss
        | V::Vpkpx
        | V::Vupkhsb
        | V::Vupkhsh
        | V::Vupklsb
        | V::Vupklsh
        | V::Vupkhpx
        | V::Vupklpx
        | V::Vspltb
        | V::Vsplth
        | V::Vspltw
        | V::Vspltisb
        | V::Vspltish
        | V::Vspltisw
        | V::Vslo
        | V::Vsro => (unit::VSU2, true, k::PPU_VMX_PERMUTE_LATENCY_CYCLES),
        V::Vaddfp
        | V::Vsubfp
        | V::Vlogefp
        | V::Vexptefp
        | V::Vrfin
        | V::Vrfiz
        | V::Vrfip
        | V::Vrfim
        | V::Vcfux
        | V::Vcfsx
        | V::Vctuxs
        | V::Vctsxs => (unit::VSU1, false, k::PPU_VMX_FLOAT_LATENCY_CYCLES),
        V::Vrefp | V::Vrsqrtefp => (unit::VSU1, false, k::PPU_VMX_ESTIMATE_LATENCY_CYCLES),
        V::Vmuloub
        | V::Vmulouh
        | V::Vmulosb
        | V::Vmulosh
        | V::Vmuleub
        | V::Vmuleuh
        | V::Vmulesb
        | V::Vmulesh
        | V::Vsum4ubs
        | V::Vsum4shs
        | V::Vsum4sbs
        | V::Vsum2sws
        | V::Vsumsws => (unit::VSU1, false, k::PPU_VMX_COMPLEX_LATENCY_CYCLES),
        V::Vaddubm
        | V::Vmaxub
        | V::Vrlb
        | V::Vcmpequb
        | V::Vadduhm
        | V::Vmaxuh
        | V::Vrlh
        | V::Vcmpequh
        | V::Vadduwm
        | V::Vmaxuw
        | V::Vrlw
        | V::Vcmpequw
        | V::Vcmpeqfp
        | V::Vmaxsb
        | V::Vslb
        | V::Vmaxsh
        | V::Vslh
        | V::Vaddcuw
        | V::Vmaxsw
        | V::Vslw
        | V::Vsl
        | V::Vcmpgefp
        | V::Vaddubs
        | V::Vminub
        | V::Vsrb
        | V::Vcmpgtub
        | V::Vadduhs
        | V::Vminuh
        | V::Vsrh
        | V::Vcmpgtuh
        | V::Vadduws
        | V::Vminuw
        | V::Vsrw
        | V::Vcmpgtuw
        | V::Vsr
        | V::Vcmpgtfp
        | V::Vaddsbs
        | V::Vminsb
        | V::Vsrab
        | V::Vcmpgtsb
        | V::Vaddshs
        | V::Vminsh
        | V::Vsrah
        | V::Vcmpgtsh
        | V::Vaddsws
        | V::Vminsw
        | V::Vsraw
        | V::Vcmpgtsw
        | V::Vcmpbfp
        | V::Vsububm
        | V::Vavgub
        | V::Vand
        | V::Vmaxfp
        | V::Vsubuhm
        | V::Vavguh
        | V::Vandc
        | V::Vminfp
        | V::Vsubuwm
        | V::Vavguw
        | V::Vor
        | V::Vxor
        | V::Vavgsb
        | V::Vnor
        | V::Vavgsh
        | V::Vsubcuw
        | V::Vavgsw
        | V::Vsububs
        | V::Vsubuhs
        | V::Vsubuws
        | V::Vsubsbs
        | V::Vsubshs
        | V::Vsubsws => (unit::VSU1, false, k::PPU_VMX_SIMPLE_LATENCY_CYCLES),
    }
}

fn vector(units: u8, permute: bool, lat: u64, vt: u8, srcs: &[u8]) -> Op {
    let mut op = Op::on(units).dst(vr(vt), lat);
    op.permute = permute;
    for &s in srcs {
        op = op.src(vr(s));
    }
    op
}

fn fp(frt: u8, srcs: &[u8], lat: u64, rc: bool) -> Op {
    let mut op = Op::on(unit::VSU1).dst(fpr(frt), lat);
    for &s in srcs {
        op = op.src(fpr(s));
    }
    if rc {
        op = op.dst(crf(1), lat + k::PPU_FP_RECORD_EXTRA_CYCLES);
    }
    op
}

fn fp_unpipelined(frt: u8, srcs: &[u8], lat: u64, rc: bool) -> Op {
    let mut op = fp(frt, srcs, lat, rc).kind(Kind::Unpipelined(lat as u16));
    op.units = unit::VSU1 | unit::VSU2;
    op
}

/// The model's view of `insn` at the state it executes from: one op, or
/// two for a fused pair, in program order.
pub(crate) fn classify(insn: &I, state: &PpuState) -> ([Op; 2], usize) {
    let none = Op::on(0);
    let one = |op: Op| ([op, none], 1);
    let two = |a: Op, b: Op| ([a, b], 2);
    let g = |r: u8| state.gpr[r as usize];
    match *insn {
        I::Lwz { rt, ra, imm } => one(int_load(ea_d(state, ra, imm), 4, rt).base(ra)),
        I::Lbz { rt, ra, imm } => one(int_load(ea_d(state, ra, imm), 1, rt).base(ra)),
        I::Lhz { rt, ra, imm } => one(int_load(ea_d(state, ra, imm), 2, rt).base(ra)),
        I::Ld { rt, ra, imm } => one(int_load(ea_d(state, ra, imm), 8, rt).base(ra)),
        I::Lwzu { rt, ra, imm } => one(update(int_load(ea_d(state, ra, imm), 4, rt).base(ra), ra)),
        I::Lbzu { rt, ra, imm } => one(update(int_load(ea_d(state, ra, imm), 1, rt).base(ra), ra)),
        I::Lhzu { rt, ra, imm } => one(update(int_load(ea_d(state, ra, imm), 2, rt).base(ra), ra)),
        I::Ldu { rt, ra, imm } => one(update(int_load(ea_d(state, ra, imm), 8, rt).base(ra), ra)),
        I::Lha { rt, ra, imm } => {
            one(load_algebraic(ea_d(state, ra, imm), 2, rt, k::PPU_LHA_MICROWORDS).base(ra))
        }
        I::Lhau { rt, ra, imm } => one(update(
            load_algebraic(ea_d(state, ra, imm), 2, rt, 8).base(ra),
            ra,
        )),
        I::Lwa { rt, ra, imm } => one(load_algebraic(ea_d(state, ra, imm), 4, rt, 13).base(ra)),
        I::Lmw { rt, ra, imm } => {
            one(multiple(ea_d(state, ra, imm), rt, 4 * (32 - rt as u64), false, false).base(ra))
        }
        I::Stmw { rs, ra, imm } => {
            one(multiple(ea_d(state, ra, imm), rs, 4 * (32 - rs as u64), true, false).base(ra))
        }
        I::Stw { rs, ra, imm } => one(store(INT_STORE, ea_d(state, ra, imm), 4)
            .src(gpr(rs))
            .base(ra)),
        I::Stb { rs, ra, imm } => one(store(INT_STORE, ea_d(state, ra, imm), 1)
            .src(gpr(rs))
            .base(ra)),
        I::Sth { rs, ra, imm } => one(store(INT_STORE, ea_d(state, ra, imm), 2)
            .src(gpr(rs))
            .base(ra)),
        I::Std { rs, ra, imm } => one(store(INT_STORE, ea_d(state, ra, imm), 8)
            .src(gpr(rs))
            .base(ra)),
        I::Stwu { rs, ra, imm } => one(update(
            store(INT_STORE, ea_d(state, ra, imm), 4)
                .src(gpr(rs))
                .base(ra),
            ra,
        )),
        I::Stbu { rs, ra, imm } => one(update(
            store(INT_STORE, ea_d(state, ra, imm), 1)
                .src(gpr(rs))
                .base(ra),
            ra,
        )),
        I::Sthu { rs, ra, imm } => one(update(
            store(INT_STORE, ea_d(state, ra, imm), 2)
                .src(gpr(rs))
                .base(ra),
            ra,
        )),
        I::Stdu { rs, ra, imm } => one(update(
            store(INT_STORE, ea_d(state, ra, imm), 8)
                .src(gpr(rs))
                .base(ra),
            ra,
        )),
        I::Addi { rt, ra, .. } | I::Addis { rt, ra, .. } => {
            let op = fx(gpr(rt));
            one(if ra == 0 { op } else { op.src(gpr(ra)) })
        }
        I::Subfic { rt, ra, .. } | I::Addic { rt, ra, .. } => {
            one(fx(gpr(rt)).src(gpr(ra)).dst(XER, k::PPU_FX_LATENCY_CYCLES))
        }
        I::AddicDot { rt, ra, .. } => one(fx(gpr(rt))
            .src(gpr(ra))
            .dst(XER, k::PPU_FX_LATENCY_CYCLES)
            .record_cr0(true)),
        I::Mulli { rt, ra, .. } => one(multiply(ra, None, rt, k::PPU_MULLI_LATENCY_CYCLES, false)),
        I::Add { rt, ra, rb, oe, rc } | I::Subf { rt, ra, rb, oe, rc } => {
            one(add_form(rt, &[ra, rb], oe, rc))
        }
        I::Subfc {
            rt,
            ra,
            rb,
            oe: _,
            rc,
        } => one(add_form(rt, &[ra, rb], true, rc)),
        I::Subfe {
            rt,
            ra,
            rb,
            oe: _,
            rc,
        }
        | I::Adde {
            rt,
            ra,
            rb,
            oe: _,
            rc,
        } => one(add_form(rt, &[ra, rb], true, rc).src(XER)),
        I::Neg { rt, ra, oe, rc } => one(add_form(rt, &[ra], oe, rc)),
        I::Addze { rt, ra, oe: _, rc }
        | I::Subfze { rt, ra, oe: _, rc }
        | I::Subfme { rt, ra, oe: _, rc }
        | I::Addme { rt, ra, oe: _, rc } => one(add_form(rt, &[ra], true, rc).src(XER)),
        I::Mullw { rt, ra, rb, rc, .. }
        | I::Mulhw { rt, ra, rb, rc }
        | I::Mulhwu { rt, ra, rb, rc } => {
            one(multiply(ra, Some(rb), rt, k::PPU_MULLW_LATENCY_CYCLES, rc))
        }
        I::Mulld { rt, ra, rb, rc, .. }
        | I::Mulhd { rt, ra, rb, rc }
        | I::Mulhdu { rt, ra, rb, rc } => {
            one(multiply(ra, Some(rb), rt, k::PPU_MULLD_LATENCY_CYCLES, rc))
        }
        I::Ldarx { rt, ra, rb } => one(load(unit::LSU, ea_x(state, ra, rb), 8, LoadClass::Reserve)
            .dst(gpr(rt), k::PPU_LOAD_FX_USE_CYCLES)
            .base(ra)
            .index(rb)),
        I::Lwarx { rt, ra, rb } => one(load(unit::LSU, ea_x(state, ra, rb), 4, LoadClass::Reserve)
            .dst(gpr(rt), k::PPU_LOAD_FX_USE_CYCLES)
            .base(ra)
            .index(rb)),
        I::Stdcx { rs, ra, rb } | I::Stwcx { rs, ra, rb } => {
            let len = if matches!(insn, I::Stdcx { .. }) {
                8
            } else {
                4
            };
            one(Op::on(INT_STORE)
                .kind(Kind::Store {
                    ea: ea_x(state, ra, rb),
                    len,
                    conditional: true,
                })
                .src(gpr(rs))
                .base(ra)
                .index(rb)
                .dst(crf(0), k::PPU_L2_LOAD_USE_CYCLES))
        }
        // `ori 0,0,0` is the preferred no-op, which the shadow quickens to
        // `Nop`; both time the same.
        I::Ori {
            ra: 0,
            rs: 0,
            imm: 0,
        } => one(Op::on(unit::FXU)),
        I::Xori { ra, rs, .. }
        | I::Xoris { ra, rs, .. }
        | I::Ori { ra, rs, .. }
        | I::Oris { ra, rs, .. } => one(fx(gpr(ra)).src(gpr(rs))),
        I::Divw { rt, ra, rb, oe, rc } => one(divide(state, rt, ra, rb, oe, rc, (true, true))),
        I::Divwu { rt, ra, rb, oe, rc } => one(divide(state, rt, ra, rb, oe, rc, (true, false))),
        I::Divd { rt, ra, rb, oe, rc } => one(divide(state, rt, ra, rb, oe, rc, (false, true))),
        I::Divdu { rt, ra, rb, oe, rc } => one(divide(state, rt, ra, rb, oe, rc, (false, false))),
        I::And { ra, rs, rb, rc }
        | I::Andc { ra, rs, rb, rc }
        | I::Nor { ra, rs, rb, rc }
        | I::Xor { ra, rs, rb, rc }
        | I::Eqv { ra, rs, rb, rc }
        | I::Nand { ra, rs, rb, rc }
        | I::Orc { ra, rs, rb, rc }
        | I::Or { ra, rs, rb, rc } => {
            one(logical(Op::on(unit::FXU).src(gpr(rs)).src(gpr(rb)), ra, rc))
        }
        I::AndiDot { ra, rs, .. } | I::AndisDot { ra, rs, .. } => {
            one(logical(Op::on(unit::FXU).src(gpr(rs)), ra, true))
        }
        I::Slw { ra, rs, rb, rc }
        | I::Srw { ra, rs, rb, rc }
        | I::Sld { ra, rs, rb, rc }
        | I::Srd { ra, rs, rb, rc } => one(Op::on(unit::FXU)
            .src(gpr(rs))
            .src(gpr(rb))
            .indirect_shift(gpr(ra), rc)),
        I::Sraw { ra, rs, rb, rc } | I::Srad { ra, rs, rb, rc } => one(Op::on(unit::FXU)
            .src(gpr(rs))
            .src(gpr(rb))
            .dst(XER, 3 + k::PPU_FX_LATENCY_CYCLES)
            .indirect_shift(gpr(ra), rc)),
        I::Srawi { ra, rs, rc, .. } | I::Sradi { ra, rs, rc, .. } => one(logical(
            Op::on(unit::FXU)
                .src(gpr(rs))
                .dst(XER, k::PPU_FX_LATENCY_CYCLES),
            ra,
            rc,
        )),
        I::Cntlzw { ra, rs, rc }
        | I::Cntlzd { ra, rs, rc }
        | I::Extsh { ra, rs, rc }
        | I::Extsb { ra, rs, rc }
        | I::Extsw { ra, rs, rc } => one(logical(Op::on(unit::FXU).src(gpr(rs)), ra, rc)),
        I::Popcntb { ra, rs } => one(fx(gpr(ra)).src(gpr(rs))),
        I::Tw { ra, rb, .. } | I::Td { ra, rb, .. } => {
            one(Op::on(unit::FXU).src(gpr(ra)).src(gpr(rb)))
        }
        I::Mcrxr { bf } => one(Op::on(unit::BRU)
            .src(XER)
            .dst(crf(bf), k::PPU_BRANCH_UNIT_LATENCY_CYCLES)
            .dst(XER, k::PPU_BRANCH_UNIT_LATENCY_CYCLES)),
        I::Cmpwi { bf, ra, .. }
        | I::Cmplwi { bf, ra, .. }
        | I::Cmpdi { bf, ra, .. }
        | I::Cmpldi { bf, ra, .. } => one(cmp(bf, ra, None)),
        I::CmpwZero { bf, ra } => one(cmp(bf, ra, None)),
        I::Cmpw { bf, ra, rb }
        | I::Cmplw { bf, ra, rb }
        | I::Cmpd { bf, ra, rb }
        | I::Cmpld { bf, ra, rb } => one(cmp(bf, ra, Some(rb))),
        I::B { link, .. } => one(branch(PPC_BO_ALWAYS, 0, link, Target::Direct, Some(true))),
        I::Bc { bo, bi, link, .. } => one(branch(
            bo,
            bi,
            link,
            Target::Direct,
            Some(branches(state, bo, bi)),
        )),
        I::Bclr { bo, bi, link } => one(branch(
            bo,
            bi,
            link,
            Target::Link,
            Some(branches(state, bo, bi)),
        )),
        I::Bcctr { bo, bi, link } => one(branch(
            bo,
            bi,
            link,
            Target::Count,
            Some(branches(state, bo | 0x04, bi)),
        )),
        I::Mcrf { crfd, crfs } => one(Op::on(unit::BRU)
            .src(crf(crfs))
            .dst(crf(crfd), k::PPU_BRANCH_UNIT_LATENCY_CYCLES)),
        I::Crand { bt, ba, bb }
        | I::Crandc { bt, ba, bb }
        | I::Cror { bt, ba, bb }
        | I::Crorc { bt, ba, bb }
        | I::Crxor { bt, ba, bb }
        | I::Crnand { bt, ba, bb }
        | I::Crnor { bt, ba, bb }
        | I::Creqv { bt, ba, bb } => one(cr_logical(bt, ba, bb)),
        I::Lwzx { rt, ra, rb } | I::Lwbrx { rt, ra, rb } => {
            one(int_load(ea_x(state, ra, rb), 4, rt).base(ra).index(rb))
        }
        I::Lbzx { rt, ra, rb } => one(int_load(ea_x(state, ra, rb), 1, rt).base(ra).index(rb)),
        I::Lhzx { rt, ra, rb } | I::Lhbrx { rt, ra, rb } => {
            one(int_load(ea_x(state, ra, rb), 2, rt).base(ra).index(rb))
        }
        I::Ldx { rt, ra, rb } | I::Ldbrx { rt, ra, rb } => {
            one(int_load(ea_x(state, ra, rb), 8, rt).base(ra).index(rb))
        }
        I::Lwzux { rt, ra, rb } => one(update(
            int_load(ea_x(state, ra, rb), 4, rt).base(ra).index(rb),
            ra,
        )),
        I::Lbzux { rt, ra, rb } => one(update(
            int_load(ea_x(state, ra, rb), 1, rt).base(ra).index(rb),
            ra,
        )),
        I::Lhzux { rt, ra, rb } => one(update(
            int_load(ea_x(state, ra, rb), 2, rt).base(ra).index(rb),
            ra,
        )),
        I::Ldux { rt, ra, rb } => one(update(
            int_load(ea_x(state, ra, rb), 8, rt).base(ra).index(rb),
            ra,
        )),
        I::Lhax { rt, ra, rb } => one(load_algebraic(ea_x(state, ra, rb), 2, rt, 8)
            .base(ra)
            .index(rb)),
        I::Lhaux { rt, ra, rb } => one(update(
            load_algebraic(ea_x(state, ra, rb), 2, rt, 8)
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Lwax { rt, ra, rb } => one(load_algebraic(ea_x(state, ra, rb), 4, rt, 12)
            .base(ra)
            .index(rb)),
        I::Lwaux { rt, ra, rb } => one(update(
            load_algebraic(ea_x(state, ra, rb), 4, rt, 12)
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Stwx { rs, ra, rb } | I::Stwbrx { rs, ra, rb } => {
            one(store(INT_STORE, ea_x(state, ra, rb), 4)
                .src(gpr(rs))
                .base(ra)
                .index(rb))
        }
        I::Stdx { rs, ra, rb } | I::Sdbrx { rs, ra, rb } => {
            one(store(INT_STORE, ea_x(state, ra, rb), 8)
                .src(gpr(rs))
                .base(ra)
                .index(rb))
        }
        I::Stbx { rs, ra, rb } => one(store(INT_STORE, ea_x(state, ra, rb), 1)
            .src(gpr(rs))
            .base(ra)
            .index(rb)),
        I::Sthx { rs, ra, rb } | I::Sthbrx { rs, ra, rb } => {
            one(store(INT_STORE, ea_x(state, ra, rb), 2)
                .src(gpr(rs))
                .base(ra)
                .index(rb))
        }
        I::Stdux { rs, ra, rb } => one(update(
            store(INT_STORE, ea_x(state, ra, rb), 8)
                .src(gpr(rs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Stwux { rs, ra, rb } => one(update(
            store(INT_STORE, ea_x(state, ra, rb), 4)
                .src(gpr(rs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Sthux { rs, ra, rb } => one(update(
            store(INT_STORE, ea_x(state, ra, rb), 2)
                .src(gpr(rs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Stbux { rs, ra, rb } => one(update(
            store(INT_STORE, ea_x(state, ra, rb), 1)
                .src(gpr(rs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Lswi { rt, ra, nb } => {
            let bytes = if nb == 0 { 32 } else { nb as u64 };
            one(multiple(ea_d(state, ra, 0), rt, bytes, false, true).base(ra))
        }
        I::Stswi { rs, ra, nb } => {
            let bytes = if nb == 0 { 32 } else { nb as u64 };
            one(multiple(ea_d(state, ra, 0), rs, bytes, true, true).base(ra))
        }
        I::Lswx { rt, ra, rb } => {
            let bytes = state.xer() & 0x7F;
            one(multiple(ea_x(state, ra, rb), rt, bytes, false, true)
                .base(ra)
                .index(rb)
                .src(XER))
        }
        I::Stswx { rs, ra, rb } => {
            let bytes = state.xer() & 0x7F;
            one(multiple(ea_x(state, ra, rb), rs, bytes, true, true)
                .base(ra)
                .index(rb)
                .src(XER))
        }
        I::Mftb { rt } | I::Mftbu { rt } => {
            one(Op::on(unit::FXU).dst(gpr(rt), k::PPU_MFTB_LATENCY_CYCLES))
        }
        I::Mfcr { rt } | I::Mfocrf { rt, .. } => {
            let mut op = Op::on(unit::BRU)
                .dst(gpr(rt), k::PPU_MFCR_CYCLES)
                .kind(Kind::Unpipelined(k::PPU_MFCR_CYCLES as u16));
            for f in 0..8 {
                op = op.src(crf(f));
            }
            one(op)
        }
        I::Mtcrf { rs, crm } => {
            // [CBE-Handbook p:729 s:A.1.3.1] `mtcrf` with bit 11 zero: 9
            // microwords.
            let mut op = Op::on(unit::FXU).src(gpr(rs)).microcode(9, 9);
            for f in (0..8u8).filter(|f| crm & (0x80 >> f) != 0).take(3) {
                op = op.dst(crf(f), 9);
            }
            one(op)
        }
        I::Mtocrf { rs, crm } => {
            let field = crm.leading_zeros() as u8;
            one(Op::on(unit::FXU)
                .src(gpr(rs))
                .dst(crf(field.min(7)), k::PPU_COMPARE_LATENCY_CYCLES))
        }
        I::Mflr { rt } => one(Op::on(unit::BRU)
            .src(LR)
            .dst(gpr(rt), k::PPU_BRANCH_UNIT_LATENCY_CYCLES)),
        I::Mfctr { rt } => one(Op::on(unit::BRU)
            .src(CTR)
            .dst(gpr(rt), k::PPU_BRANCH_UNIT_LATENCY_CYCLES)),
        I::Mtlr { rs } => one(Op::on(unit::BRU)
            .src(gpr(rs))
            .dst(LR, k::PPU_BRANCH_UNIT_LATENCY_CYCLES)),
        I::Mtctr { rs } => one(Op::on(unit::BRU)
            .src(gpr(rs))
            .dst(CTR, k::PPU_BRANCH_UNIT_LATENCY_CYCLES)),
        I::Mfxer { rt } => one(fx(gpr(rt)).src(XER)),
        I::Mtxer { rs } => one(Op::on(unit::FXU)
            .src(gpr(rs))
            .dst(XER, k::PPU_FX_LATENCY_CYCLES)),
        I::Mfvrsave { rt } => one(fx(gpr(rt))),
        I::Mtvrsave { rs } => one(Op::on(unit::FXU).src(gpr(rs))),
        I::Rlwinm { ra, rs, rc, .. }
        | I::Rldicl { ra, rs, rc, .. }
        | I::Rldicr { ra, rs, rc, .. }
        | I::Rldic { ra, rs, rc, .. } => one(logical(Op::on(unit::FXU).src(gpr(rs)), ra, rc)),
        I::Rlwimi { ra, rs, rc, .. } | I::Rldimi { ra, rs, rc, .. } => {
            one(logical(Op::on(unit::FXU).src(gpr(rs)).src(gpr(ra)), ra, rc))
        }
        I::Rlwnm { ra, rs, rb, rc, .. }
        | I::Rldcl { ra, rs, rb, rc, .. }
        | I::Rldcr { ra, rs, rb, rc, .. } => one(Op::on(unit::FXU)
            .src(gpr(rs))
            .src(gpr(rb))
            .indirect_shift(gpr(ra), rc)),
        I::Vx { op, rc, vt, va, vb } => {
            let (units, permute, lat) = vx_class(op);
            let srcs: &[u8] = match op.shape() {
                crate::instruction::ops::VxShape::VdVaVb => &[va, vb],
                crate::instruction::ops::VxShape::VdVb
                | crate::instruction::ops::VxShape::VdVbUimm => &[vb],
                crate::instruction::ops::VxShape::VdSimm => &[],
            };
            let op = vector(units, permute, lat, vt, srcs);
            one(if rc { op.dst(crf(6), lat) } else { op })
        }
        I::Va { op, vt, va, vb, vc } => {
            let (units, permute, lat) = match op {
                VaOp::Vperm | VaOp::Vsldoi => (unit::VSU2, true, k::PPU_VMX_PERMUTE_LATENCY_CYCLES),
                VaOp::Vsel => (unit::VSU1, false, k::PPU_VMX_SIMPLE_LATENCY_CYCLES),
                VaOp::Vmaddfp | VaOp::Vnmsubfp => {
                    (unit::VSU1, false, k::PPU_VMX_FLOAT_LATENCY_CYCLES)
                }
                VaOp::Vmhaddshs
                | VaOp::Vmhraddshs
                | VaOp::Vmladduhm
                | VaOp::Vmsumubm
                | VaOp::Vmsummbm
                | VaOp::Vmsumuhm
                | VaOp::Vmsumuhs
                | VaOp::Vmsumshm
                | VaOp::Vmsumshs => (unit::VSU1, false, k::PPU_VMX_COMPLEX_LATENCY_CYCLES),
            };
            one(vector(units, permute, lat, vt, &[va, vb, vc]))
        }
        I::Vxor { vt, va, vb } => one(vector(
            unit::VSU1,
            false,
            k::PPU_VMX_SIMPLE_LATENCY_CYCLES,
            vt,
            &[va, vb],
        )),
        I::Vsldoi { vt, va, vb, .. } => one(vector(
            unit::VSU2,
            true,
            k::PPU_VMX_PERMUTE_LATENCY_CYCLES,
            vt,
            &[va, vb],
        )),
        I::Lvlx { vt, ra, rb }
        | I::Lvrx { vt, ra, rb }
        | I::Lvlxl { vt, ra, rb }
        | I::Lvrxl { vt, ra, rb }
        | I::Lvebx { vt, ra, rb }
        | I::Lvehx { vt, ra, rb }
        | I::Lvewx { vt, ra, rb }
        | I::Lvx { vt, ra, rb }
        | I::Lvxl { vt, ra, rb } => {
            one(
                load(VSU_MEM, ea_x(state, ra, rb) & !15, 16, LoadClass::Vector)
                    .dst(vr(vt), k::PPU_VMX_LOAD_LATENCY_CYCLES)
                    .base(ra)
                    .index(rb),
            )
        }
        I::Lvsl { vt, ra, rb } | I::Lvsr { vt, ra, rb } => one(Op::on(VSU_MEM)
            .dst(vr(vt), k::PPU_VMX_LOAD_LATENCY_CYCLES)
            .base(ra)
            .index(rb)),
        I::Stvlx { vs, ra, rb }
        | I::Stvrx { vs, ra, rb }
        | I::Stvlxl { vs, ra, rb }
        | I::Stvrxl { vs, ra, rb }
        | I::Stvebx { vs, ra, rb }
        | I::Stvehx { vs, ra, rb }
        | I::Stvewx { vs, ra, rb }
        | I::Stvx { vs, ra, rb }
        | I::Stvxl { vs, ra, rb } => one(store(VSU_MEM, ea_x(state, ra, rb) & !15, 16)
            .src(vr(vs))
            .base(ra)
            .index(rb)),
        I::Lfs { frt, ra, imm } => one(fp_load(ea_d(state, ra, imm), 4, frt).base(ra)),
        I::Lfd { frt, ra, imm } => one(fp_load(ea_d(state, ra, imm), 8, frt).base(ra)),
        I::Lfsu { frt, ra, imm } => one(update(fp_load(ea_d(state, ra, imm), 4, frt).base(ra), ra)),
        I::Lfdu { frt, ra, imm } => one(update(fp_load(ea_d(state, ra, imm), 8, frt).base(ra), ra)),
        I::Lfsx { frt, ra, rb } => one(fp_load(ea_x(state, ra, rb), 4, frt).base(ra).index(rb)),
        I::Lfdx { frt, ra, rb } => one(fp_load(ea_x(state, ra, rb), 8, frt).base(ra).index(rb)),
        I::Lfsux { frt, ra, rb } => one(update(
            fp_load(ea_x(state, ra, rb), 4, frt).base(ra).index(rb),
            ra,
        )),
        I::Lfdux { frt, ra, rb } => one(update(
            fp_load(ea_x(state, ra, rb), 8, frt).base(ra).index(rb),
            ra,
        )),
        I::Stfs { frs, ra, imm } => one(store(VSU_MEM, ea_d(state, ra, imm), 4)
            .src(fpr(frs))
            .base(ra)),
        I::Stfd { frs, ra, imm } => one(store(VSU_MEM, ea_d(state, ra, imm), 8)
            .src(fpr(frs))
            .base(ra)),
        I::Stfsu { frs, ra, imm } => one(update(
            store(VSU_MEM, ea_d(state, ra, imm), 4)
                .src(fpr(frs))
                .base(ra),
            ra,
        )),
        I::Stfdu { frs, ra, imm } => one(update(
            store(VSU_MEM, ea_d(state, ra, imm), 8)
                .src(fpr(frs))
                .base(ra),
            ra,
        )),
        I::Stfiwx { frs, ra, rb } | I::Stfsx { frs, ra, rb } => {
            one(store(VSU_MEM, ea_x(state, ra, rb), 4)
                .src(fpr(frs))
                .base(ra)
                .index(rb))
        }
        I::Stfdx { frs, ra, rb } => one(store(VSU_MEM, ea_x(state, ra, rb), 8)
            .src(fpr(frs))
            .base(ra)
            .index(rb)),
        I::Stfsux { frs, ra, rb } => one(update(
            store(VSU_MEM, ea_x(state, ra, rb), 4)
                .src(fpr(frs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Stfdux { frs, ra, rb } => one(update(
            store(VSU_MEM, ea_x(state, ra, rb), 8)
                .src(fpr(frs))
                .base(ra)
                .index(rb),
            ra,
        )),
        I::Fp63 {
            op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => one(fp63(op, frt, fra, frb, frc, rc)),
        I::Fp59 {
            op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => one(fp59(op, frt, fra, frb, frc, rc)),
        I::Li { rt, .. } => one(fx(gpr(rt))),
        I::Mr { ra, rs }
        | I::Slwi { ra, rs, .. }
        | I::Srwi { ra, rs, .. }
        | I::Clrlwi { ra, rs, .. }
        | I::Clrldi { ra, rs, .. }
        | I::Sldi { ra, rs, .. }
        | I::Srdi { ra, rs, .. } => one(fx(gpr(ra)).src(gpr(rs))),
        I::Nop => one(Op::on(unit::FXU)),
        I::LwzCmpwi {
            rt,
            ra_load,
            offset,
            bf,
            ..
        } => two(
            int_load(ea_d(state, ra_load, offset), 4, rt).base(ra_load),
            cmp(bf, rt, None),
        ),
        I::LiStw {
            rt,
            imm,
            ra_store,
            store_offset,
        } => {
            let base = if ra_store == rt {
                imm as i64 as u64
            } else {
                g(ra_store)
            };
            let ea = if ra_store == 0 { 0 } else { base }.wrapping_add(store_offset as i64 as u64);
            two(
                fx(gpr(rt)),
                store(INT_STORE, ea, 4).src(gpr(rt)).base(ra_store),
            )
        }
        I::MflrStw {
            rt,
            ra_store,
            store_offset,
        }
        | I::MflrStd {
            rt,
            ra_store,
            store_offset,
        } => {
            let len = if matches!(insn, I::MflrStd { .. }) {
                8
            } else {
                4
            };
            let base = if ra_store == rt {
                state.lr()
            } else {
                g(ra_store)
            };
            let ea = if ra_store == 0 { 0 } else { base }.wrapping_add(store_offset as i64 as u64);
            two(
                Op::on(unit::BRU)
                    .src(LR)
                    .dst(gpr(rt), k::PPU_BRANCH_UNIT_LATENCY_CYCLES),
                store(INT_STORE, ea, len).src(gpr(rt)).base(ra_store),
            )
        }
        I::LwzMtlr {
            rt,
            ra_load,
            offset,
        }
        | I::LdMtlr {
            rt,
            ra_load,
            offset,
        } => {
            let len = if matches!(insn, I::LdMtlr { .. }) {
                8
            } else {
                4
            };
            two(
                int_load(ea_d(state, ra_load, offset), len, rt).base(ra_load),
                Op::on(unit::BRU)
                    .src(gpr(rt))
                    .dst(LR, k::PPU_BRANCH_UNIT_LATENCY_CYCLES),
            )
        }
        I::StdStd {
            rs1,
            rs2,
            ra,
            offset1,
        } => {
            let ea = ea_d(state, ra, offset1);
            two(
                store(INT_STORE, ea, 8).src(gpr(rs1)).base(ra),
                store(INT_STORE, ea.wrapping_add(8), 8)
                    .src(gpr(rs2))
                    .base(ra),
            )
        }
        I::CmpwiBc { bf, ra, bo, bi, .. } => two(
            cmp(bf, ra, None),
            branch(bo, bi, false, Target::Direct, None),
        ),
        I::CmpwBc {
            bf, ra, rb, bo, bi, ..
        } => two(
            cmp(bf, ra, Some(rb)),
            branch(bo, bi, false, Target::Direct, None),
        ),
        I::Consumed => ([none, none], 0),
        I::Dcbz { ra, rb } => one(Op::on(unit::LSU)
            .kind(Kind::ZeroLine {
                ea: ea_x(state, ra, rb),
            })
            .base(ra)
            .index(rb)),
        I::Sc { lev } => one(Op::on(0).kind(Kind::Syscall {
            number: (lev == 0).then_some(state.gpr[11]),
        })),
        I::Sync { .. } => one(Op::on(unit::LSU).kind(Kind::Drain)),
        I::Eieio => one(Op::on(unit::LSU)),
        I::Isync => one(Op::on(unit::LSU).kind(Kind::Refetch)),
    }
}

/// An add-family operation: GPR at 2, CR0 at 1 when recording, XER at 2
/// when it writes CA or OV. The decoder issues these whole.
///
/// [CBE-Handbook p:717 s:A.1.2] `add. subf. subfic neg.`: 2 (gpr),
/// 1 (cr); the carrying and extended forms 2 (xer).
fn add_form(rt: u8, srcs: &[u8], xer: bool, rc: bool) -> Op {
    let mut op = fx(gpr(rt));
    for &s in srcs {
        op = op.src(gpr(s));
    }
    let op = if xer {
        op.dst(XER, k::PPU_FX_LATENCY_CYCLES)
    } else {
        op
    };
    op.record_cr0(rc)
}

fn fp_load(ea: u64, len: u8, frt: u8) -> Op {
    load(VSU_MEM, ea, len, LoadClass::Float).dst(fpr(frt), k::PPU_FP_LOAD_LATENCY_CYCLES)
}

fn fp63(op: Fp63Op, frt: u8, fra: u8, frb: u8, frc: u8, rc: bool) -> Op {
    use Fp63Op as F;
    let l = k::PPU_FP_LATENCY_CYCLES;
    match op {
        F::Fadd | F::Fsub => fp(frt, &[fra, frb], l, rc),
        F::Fmul => fp(frt, &[fra, frc], l, rc),
        F::Fmadd | F::Fmsub | F::Fnmadd | F::Fnmsub | F::Fsel => fp(frt, &[fra, frc, frb], l, rc),
        F::Frsp
        | F::Fctiw
        | F::Fctiwz
        | F::Fctid
        | F::Fctidz
        | F::Fcfid
        | F::Fmr
        | F::Fneg
        | F::Fabs
        | F::Fnabs
        | F::Frsqrte => fp(frt, &[frb], l, rc),
        F::Fdiv => fp_unpipelined(frt, &[fra, frb], k::PPU_FDIV_CYCLES, rc),
        F::Fsqrt => fp_unpipelined(frt, &[frb], k::PPU_FSQRT_CYCLES, rc),
        // [CBE-Handbook p:722 s:A.1.2] `fcmpu fcmpo`: latency 1.
        F::Fcmpu | F::Fcmpo => Op::on(unit::VSU1)
            .src(fpr(fra))
            .src(fpr(frb))
            .dst(crf(frt >> 2), k::PPU_COMPARE_LATENCY_CYCLES)
            .dst(FPSCR, k::PPU_COMPARE_LATENCY_CYCLES),
        F::Mffs => Op::on(unit::VSU1)
            .src(FPSCR)
            .dst(fpr(frt), k::PPU_MFFS_LATENCY_CYCLES)
            .kind(Kind::Drain),
        // [CBE-Handbook p:722 s:A.1.2] `mtfsfi mtfsf mtfsb0 mtfsb1`:
        // latency 1.
        F::Mtfsb0 | F::Mtfsb1 | F::Mtfsfi => {
            Op::on(unit::VSU1).dst(FPSCR, k::PPU_COMPARE_LATENCY_CYCLES)
        }
        F::Mtfsf => Op::on(unit::VSU1)
            .src(fpr(frb))
            .dst(FPSCR, k::PPU_COMPARE_LATENCY_CYCLES),
        // [CBE-Handbook p:729 s:A.1.3.1] `mcrfs`: 2 microwords.
        F::Mcrfs => Op::on(unit::VSU1)
            .src(FPSCR)
            .dst(crf(frt >> 2), 2)
            .microcode(2, 2),
    }
}

fn fp59(op: Fp59Op, frt: u8, fra: u8, frb: u8, frc: u8, rc: bool) -> Op {
    use Fp59Op as F;
    let l = k::PPU_FP_LATENCY_CYCLES;
    match op {
        F::Fadds | F::Fsubs => fp(frt, &[fra, frb], l, rc),
        F::Fmuls => fp(frt, &[fra, frc], l, rc),
        F::Fmadds | F::Fmsubs | F::Fnmadds | F::Fnmsubs => fp(frt, &[fra, frc, frb], l, rc),
        F::Fres => fp(frt, &[frb], l, rc),
        F::Fdivs => fp_unpipelined(frt, &[fra, frb], k::PPU_FDIV_CYCLES, rc),
        F::Fsqrts => fp_unpipelined(frt, &[frb], k::PPU_FSQRT_CYCLES, rc),
    }
}
