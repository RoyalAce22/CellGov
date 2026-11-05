//! The registers the issue model tracks, the units an instruction
//! occupies, and the [`Op`] that describes one instruction.

use cellgov_ps3_abi::hw::ppu_issue as k;

/// A register the model tracks: GPRs, FPRs, VRs, the eight CR fields,
/// LR, CTR, XER and FPSCR.
pub(crate) type Reg = u8;

/// The number of registers the model tracks.
pub(crate) const REGS: usize = 108;

pub(super) const fn gpr(n: u8) -> Reg {
    n
}
pub(super) const fn fpr(n: u8) -> Reg {
    32 + n
}
pub(in crate::timing) const fn is_fpr(r: Reg) -> bool {
    r >= 32 && r < 64
}
pub(super) const fn vr(n: u8) -> Reg {
    64 + n
}
const fn is_vr(r: Reg) -> bool {
    r >= 64 && r < 96
}
pub(super) const fn crf(n: u8) -> Reg {
    96 + n
}
pub(super) const LR: Reg = 104;
pub(super) const CTR: Reg = 105;
pub(in crate::timing) const XER: Reg = 106;
pub(super) const FPSCR: Reg = 107;
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
    /// Reloads the line at `ea` into the L2 and the L1.
    Touch { ea: u64 },
    /// Drops the line at `ea` from the L1 and the L2.
    Flush { ea: u64 },
    /// Writes the line at `ea` back and keeps it valid.
    StoreBack { ea: u64 },
    /// A branch with this `BO` field.
    ///
    /// `taken` is its direction from the state it executes from. A fused
    /// pair's branch reads the CR that its compare writes.
    Branch {
        bo: u8,
        link: bool,
        target: Target,
        taken: bool,
    },
    /// Issues once every older result is ready.
    Drain,
    /// Drains, then refetches the instructions after it.
    Refetch,
    /// An `mtvscr`: drains, then holds its units for
    /// `PPU_MTVSCR_HOLD_CYCLES` and the younger vector-scalar
    /// instructions for `PPU_MTVSCR_YOUNGER_VSU_CYCLES`.
    VscrWrite,
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
    /// The destinations of a load multiple or load string: the first GPR,
    /// the count and the latency. The GPRs after the first wrap from r31
    /// to r0.
    pub(crate) range: Option<(Reg, u8, u16)>,
    /// The data sources of a store multiple or store string: the first
    /// GPR, the count and the microwords before the first read. The GPRs
    /// after the first wrap from r31 to r0.
    pub(crate) src_range: Option<(Reg, u8, u8)>,
    /// What it does beyond its registers.
    pub(crate) kind: Kind,
    /// Microwords, or zero for an instruction issued whole.
    pub(crate) words: u8,
    /// Cycles a microcode sequence holds issue. Its write latencies
    /// count from its first microword.
    pub(crate) span: u8,
}

impl Op {
    pub(super) const fn on(units: u8) -> Self {
        Self {
            units,
            permute: false,
            srcs: [NO_REG; 8],
            addr: [NO_REG; 2],
            dsts: [(NO_REG, 0); 3],
            range: None,
            src_range: None,
            kind: Kind::Plain,
            words: 0,
            span: 0,
        }
    }

    /// Reads `r` as data. A register read twice takes one slot, so
    /// `or rA,rS,rS` and `mr rA,rS` read the same set.
    pub(super) fn src(mut self, r: Reg) -> Self {
        if self.srcs.contains(&r) {
            return self;
        }
        if let Some(slot) = self.srcs.iter_mut().find(|s| **s == NO_REG) {
            *slot = r;
        }
        self
    }

    /// Reads GPR `ra` as an address unless it is the literal zero of
    /// `(RA|0)`.
    pub(super) fn base(mut self, ra: u8) -> Self {
        if ra != 0 {
            if let Some(slot) = self.addr.iter_mut().find(|s| **s == NO_REG) {
                *slot = gpr(ra);
            }
        }
        self
    }

    pub(super) fn index(mut self, rb: u8) -> Self {
        if let Some(slot) = self.addr.iter_mut().find(|s| **s == NO_REG) {
            *slot = gpr(rb);
        }
        self
    }

    pub(super) fn dst(mut self, r: Reg, lat: u64) -> Self {
        if let Some(slot) = self.dsts.iter_mut().find(|s| s.0 == NO_REG) {
            *slot = (r, lat as u16);
        }
        self
    }

    pub(super) fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }

    /// The CR0 write of an `Rc = 1` form the decoder issues whole.
    pub(super) fn record_cr0(self, rc: bool) -> Self {
        if rc {
            self.dst(crf(0), k::PPU_COMPARE_LATENCY_CYCLES)
        } else {
            self
        }
    }

    /// A microcode sequence of `words` microwords that holds issue for
    /// `span` cycles.
    pub(super) fn microcode(mut self, words: u8, span: u8) -> Self {
        self.words = words;
        self.span = span;
        self
    }

    /// The microcoded `Rc = 1` form of an operation of latency `lat`:
    /// the base operation, then a compare that waits for it.
    ///
    /// [CBE-Handbook p:726 s:A.1.3.1] a CR-recording instruction is the
    /// base operation followed by a dependent compare.
    pub(super) fn cr_recording(self, rt: Reg, lat: u64) -> Self {
        let span = lat + k::PPU_COMPARE_LATENCY_CYCLES;
        self.dst(rt, lat).dst(crf(0), span).microcode(2, span as u8)
    }

    /// A shift or rotate by a register: `mt_shr`, two no-ops, then the
    /// root instruction, and with `Rc = 1` a compare after it.
    ///
    /// [CBE-Handbook p:728 s:A.1.3.1] indirect shifts and rotates: 4
    /// microwords, 5 when recording.
    pub(super) fn indirect_shift(self, ra: Reg, rc: bool) -> Self {
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

    /// The GPRs a load multiple or load string writes.
    pub(crate) fn range_targets(&self) -> impl Iterator<Item = Reg> {
        self.range
            .into_iter()
            .flat_map(|(first, n, _)| gpr_run(first, n))
    }

    /// The registers this one writes.
    pub(crate) fn writes(&self) -> impl Iterator<Item = Reg> + '_ {
        self.dsts
            .iter()
            .map(|d| d.0)
            .filter(|r| *r != NO_REG)
            .chain(self.range_targets())
    }

    /// Whether this one is a pipelined fixed-point instruction issued
    /// whole, whose writes land in issue order.
    pub(crate) fn writes_in_order(&self) -> bool {
        self.units == unit::FXU && self.words == 0 && self.kind == Kind::Plain
    }

    /// The registers this one reads as data, each with the cycles after
    /// issue it reads that register.
    ///
    /// A store multiple or store string reads its `i`th source `lead + i`
    /// cycles after issue. Its first `lead` microwords read no source, and
    /// each later microword reads one word.
    ///
    /// [CBE-Handbook p:729 s:A.1.3.1] `lmw stmw`: 2 + 1 x words microwords.
    pub(crate) fn reads(&self) -> impl Iterator<Item = (Reg, u64)> + '_ {
        let run = self
            .src_range
            .into_iter()
            .flat_map(|(first, n, lead)| gpr_run(first, n).zip(u64::from(lead)..));
        self.srcs
            .iter()
            .filter(|r| **r != NO_REG)
            .map(|&r| (r, 0))
            .chain(run)
    }

    /// Whether this one is an `mfvscr`, the one drain that writes a VR.
    pub(crate) fn is_vscr_read(&self) -> bool {
        self.kind == Kind::Drain && self.writes().any(is_vr)
    }

    /// Whether this one reads a register `other` writes.
    pub(crate) fn reads_any_of(&self, other: &Op) -> bool {
        other
            .writes()
            .any(|w| self.reads().any(|(r, _)| r == w) || self.addr.contains(&w))
    }
}

/// `n` GPRs from `first` up, with r0 after r31.
///
/// [PPC-Book1 p:48 s:3.3.6] a load string's target register advances as
/// `r + 1 (mod 32)`.
fn gpr_run(first: Reg, n: u8) -> impl Iterator<Item = Reg> {
    (0..n.min(32)).map(move |i| (first + i) % 32)
}
