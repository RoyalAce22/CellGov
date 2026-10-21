//! The SPU's issue timing: the cycle each instruction issues in.
//!
//! The cycle comes from:
//!
//! - the CBE instruction table
//! - the dual-issue rule
//! - register dependencies
//! - branch prediction by hint
//! - the flushes the console measures
//!
//! [CBE-Handbook p:771 s:B.1.3.2] the SPU issues in program order; an instruction issues once its register dependencies are met, and an even/odd pair in one doubleword-aligned fetch group dual-issues.
//!
//! The model is a pure function of the instruction stream: the cycle an
//! instruction issues in depends only on the instructions before it, so
//! no host time and no scheduling choice enter. One thing the console has
//! and the model leaves out is local-store contention with DMA: the
//! `spu_issue_runout` capture times a loop of 64 lqa at 71 cycles an
//! iteration beside sixteen 16 KB gets, against 67 without them.
//!
//! [CBE-Handbook p:66 s:3.1.1.3] DMA occupies at most one of every eight local-store cycles, so its impact on loads, stores and instruction fetch can be ignored except for highly optimized code.

use crate::instruction::SpuInstruction;
use crate::state::SpuState;
use cellgov_ps3_abi::hw::spu_isa;
use cellgov_ps3_abi::hw::spu_issue::{
    branch_miss_cycles, SpuIssue, SpuPipe, SPU_CHANNEL_RETRY_WINDOW_CYCLES, SPU_FETCH_LINES_HELD,
    SPU_FETCH_LINE_BYTES, SPU_HINTED_NOT_TAKEN_CYCLES, SPU_HINT_READY_CYCLES,
    SPU_HINT_STALL_END_INSNS, SPU_HINT_STALL_MIN_INSNS, SPU_ISSUE_TABLE,
    SPU_PREFETCH_WINDOW_CYCLES, SPU_RUNOUT_CYCLES,
};

/// Inline prefetch: which fetch lines past the issuing one the SPU
/// holds.
///
/// [CBE-Handbook p:774 s:B.1.4] the prefetcher uses empty slots on the LS schedule; long sequences of loads and stores can prevent it, and the SPU runs out of instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fetch {
    /// The fetch line of the last instruction issued, or `None` after a
    /// branch or a flush.
    line: Option<u32>,
    /// Bit `i` set: line `line + 1 + i` is held.
    held: u32,
    /// The line a prefetch request is for, and the cycle its window
    /// opened.
    request: Option<(u32, u64)>,
    /// The last cycle an instruction issued in, and whether a load, a
    /// store or a hint took the local store in it.
    last: Option<(u64, bool)>,
}

impl Fetch {
    /// The state before the first instruction, which issues as a branch
    /// target does.
    const fn new() -> Self {
        Self {
            line: None,
            held: 0,
            request: None,
            last: None,
        }
    }

    /// Fetch again from the next instruction issued, as after a branch.
    fn redirect(&mut self) {
        self.line = None;
        self.request = None;
    }

    /// The lines held once every cycle before `cycle` has run: a request
    /// whose window holds a cycle the local store was free in is
    /// fetched.
    fn held_before(&self, cycle: u64) -> u32 {
        let (Some(line), Some((want, opened))) = (self.line, self.request) else {
            return self.held;
        };
        let free = match self.last {
            // The last issue cycle has ended, and every cycle between it
            // and `cycle` issued nothing.
            Some((at, busy)) if at < cycle => {
                let first = if busy { at + 1 } else { at };
                (first < cycle).then_some(first.max(opened))
            }
            Some(_) => None,
            None => (opened < cycle).then_some(opened),
        };
        match free {
            Some(at) if at < cycle && at <= opened + SPU_PREFETCH_WINDOW_CYCLES => {
                self.held | 1 << (want - line - 1)
            }
            _ => self.held,
        }
    }

    /// Whether the instruction at `pc` issues from a line inline prefetch
    /// never fetched, at `cycle`.
    fn runs_out(&self, pc: u32, cycle: u64) -> bool {
        self.line
            .is_some_and(|line| pc / SPU_FETCH_LINE_BYTES == line + 1)
            && self.held_before(cycle) & 1 == 0
    }

    /// Record an instruction at `pc` issued at `cycle`, taking the local
    /// store if `uses_ls`.
    fn issue(&mut self, pc: u32, cycle: u64, uses_ls: bool) {
        let line = pc / SPU_FETCH_LINE_BYTES;
        let held = self.held_before(cycle);
        if self.request.is_some() && (held != self.held || self.window_closed(cycle)) {
            self.request = None;
        }
        self.held = held;
        match self.line {
            Some(at) if at == line => {}
            Some(at) if at + 1 == line => {
                // A runout fetches every line it was missing.
                if self.held & 1 == 0 {
                    self.held = (1 << SPU_FETCH_LINES_HELD) - 1;
                }
                self.held >>= 1;
                self.line = Some(line);
                self.request = Some((line + SPU_FETCH_LINES_HELD, cycle));
            }
            _ => {
                self.line = Some(line);
                self.held = (1 << SPU_FETCH_LINES_HELD) - 1;
                self.request = None;
            }
        }
        self.last = match self.last {
            Some((at, busy)) if at == cycle => Some((at, busy || uses_ls)),
            _ => Some((cycle, uses_ls)),
        };
    }

    /// Whether the open request's window ended before `cycle`.
    fn window_closed(&self, cycle: u64) -> bool {
        self.request
            .is_some_and(|(_, opened)| cycle > opened + SPU_PREFETCH_WINDOW_CYCLES)
    }
}

/// A loaded branch hint: the branch it names and where it predicts the
/// branch goes.
///
/// [CBE-Handbook p:694 s:24.3.3.2] a hint loads the branch-target buffer with a target address and the address of the branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hint {
    branch: u32,
    target: u32,
}

/// A hint issued and not yet loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingHint {
    hint: Hint,
    /// The cycle from which a branch can use it.
    ready: u64,
    /// Whether its branch waits for it: the hint stall.
    ///
    /// [CBE-Handbook p:695 s:24.3.3.4] hint stall is set up by hints whose branches are at least eight, but less than 32, instructions ahead.
    stallable: bool,
}

/// What an instruction does to the issue beyond its own slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    Branch,
    Hint(Hint),
    Sync,
    /// A channel instruction, with the channel group its retry rule
    /// compares.
    Channel(u8),
}

/// The issue cycle [`SpuPipeline::plan`] chose for one instruction, and
/// what [`SpuPipeline::retire`] needs to record it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IssueSlot {
    /// The cycle the instruction issues in.
    pub(crate) cycle: u64,
    pc: u32,
    issue: SpuIssue,
    dest: Option<u8>,
    kind: Kind,
    /// The target a hint predicts, for a branch a hint names.
    predicted: Option<u32>,
    /// Whether the instruction takes the local store as it issues.
    uses_ls: bool,
}

/// The SPU's issue state between instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuPipeline {
    /// One cycle past the last issue: the cycles the unit has run.
    elapsed: u64,
    /// The cycle a flush, a stall or a charge holds the next issue to.
    floor: u64,
    /// The address and cycle of an even-pipeline instruction at the
    /// start of a fetch group, which the next instruction can pair with.
    pair: Option<(u32, u64)>,
    /// The cycle each register's last result becomes available.
    ready: [u64; crate::state::SPU_REG_COUNT],
    /// The group and issue cycle of the last channel instruction.
    channel: Option<(u8, u64)>,
    /// The loaded hint.
    hint: Option<Hint>,
    /// A hint issued and not yet loaded.
    pending: Option<PendingHint>,
    /// Inline prefetch.
    fetch: Fetch,
}

impl Default for SpuPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl SpuPipeline {
    /// The issue state of an SPU that has run nothing: no hint loaded.
    ///
    /// [CBE-Handbook p:694 s:24.3.3.3] the branch-target buffer is invalid at SPU program start.
    pub const fn new() -> Self {
        Self {
            elapsed: 0,
            floor: 0,
            pair: None,
            ready: [0; crate::state::SPU_REG_COUNT],
            channel: None,
            hint: None,
            pending: None,
            fetch: Fetch::new(),
        }
    }

    /// The cycles the unit has run: one past its last issue, and past
    /// any charge since.
    pub(crate) fn elapsed(&self) -> u64 {
        self.elapsed
    }

    /// The earliest cycle the next instruction can issue in.
    pub(crate) fn next_issue(&self) -> u64 {
        self.floor.max(self.elapsed)
    }

    /// The cycle the instruction `insn`, the word `raw` at `state.pc`,
    /// issues in after every instruction retired so far.
    pub(crate) fn plan(&self, state: &SpuState, raw: u32, insn: &SpuInstruction) -> IssueSlot {
        let pc = state.pc;
        let issue = spu_isa::row_for(raw)
            .and_then(|(row, _)| SPU_ISSUE_TABLE[row])
            .unwrap_or(SpuIssue {
                pipe: SpuPipe::Even,
                latency: 2,
                stall: 0,
            });
        let (dest, sources) = operands(insn);
        let operands_ready = sources
            .iter()
            .flatten()
            .map(|&r| self.ready[usize::from(r)])
            .max()
            .unwrap_or(0);
        // [CBE-Handbook p:771 s:B.1.3.2] dual issue: the first of a fetch group on the even pipeline and the second on the odd.
        let mut cycle = match self.pair {
            Some((addr, at))
                if pc == addr.wrapping_add(4)
                    && issue.pipe == SpuPipe::Odd
                    && operands_ready <= at =>
            {
                at
            }
            _ => self.next_issue().max(operands_ready),
        };
        if self.fetch.runs_out(pc, cycle) {
            cycle += SPU_RUNOUT_CYCLES;
        }
        let kind = kind(state, insn);
        if let Kind::Channel(group) = kind {
            if let Some((last, at)) = self.channel {
                if last == group && cycle - at <= SPU_CHANNEL_RETRY_WINDOW_CYCLES {
                    cycle += branch_miss_cycles(pc);
                }
            }
        }
        let mut predicted = None;
        if kind == Kind::Branch {
            match self.pending {
                Some(p) if p.hint.branch == pc && (cycle >= p.ready || p.stallable) => {
                    cycle = cycle.max(p.ready);
                    predicted = Some(p.hint.target);
                }
                // A pending hint that is ready replaces the loaded one.
                // [CBE-Handbook p:696 s:24.3.3.5] only one hint is active at a time.
                Some(p) if cycle >= p.ready => {
                    predicted = (p.hint.branch == pc).then_some(p.hint.target);
                }
                _ => {
                    predicted = self.hint.filter(|h| h.branch == pc).map(|h| h.target);
                }
            }
        }
        IssueSlot {
            cycle,
            pc,
            issue,
            dest,
            kind,
            predicted,
            uses_ls: uses_ls(insn),
        }
    }

    /// Record `slot`'s instruction as retired.
    ///
    /// `taken` is one of:
    ///
    /// - the address a taken branch went to
    /// - `None`, for a branch not taken and for every other instruction
    pub(crate) fn retire(&mut self, slot: IssueSlot, taken: Option<u32>) {
        let t = slot.cycle;
        self.elapsed = self.elapsed.max(t + 1);
        self.floor = t + 1 + u64::from(slot.issue.stall);
        self.fetch.issue(slot.pc, t, slot.uses_ls);
        if taken.is_some() {
            self.fetch.redirect();
        }
        // A hint loads once it is ready; a branch that used it issued no
        // earlier.
        if let Some(p) = self.pending.filter(|p| t >= p.ready) {
            self.hint = Some(p.hint);
            self.pending = None;
        }
        if let Some(d) = slot.dest {
            self.ready[usize::from(d)] = t + u64::from(slot.issue.latency);
        }
        self.pair = (slot.pc.is_multiple_of(8)
            && slot.issue.pipe == SpuPipe::Even
            && slot.issue.stall == 0
            && slot.kind == Kind::Plain)
            .then_some((slot.pc, t));
        match slot.kind {
            Kind::Plain => {}
            Kind::Branch => {
                // [CBE-Handbook p:691 s:24.3] a correctly predicted branch executes in one cycle; a mispredicted one costs 18 to 19.
                self.floor = match (taken, slot.predicted) {
                    (Some(to), Some(hinted)) if to == hinted => t + 1,
                    (None, None) => t + 1,
                    (None, Some(_)) => t + SPU_HINTED_NOT_TAKEN_CYCLES,
                    (Some(to), _) => t + branch_miss_cycles(to),
                };
            }
            Kind::Hint(hint) => {
                let ahead = (hint.branch.wrapping_sub(slot.pc) as i32) / 4;
                self.pending = Some(PendingHint {
                    hint,
                    ready: t + SPU_HINT_READY_CYCLES,
                    stallable: (SPU_HINT_STALL_MIN_INSNS as i32..SPU_HINT_STALL_END_INSNS as i32)
                        .contains(&ahead),
                });
            }
            Kind::Sync => {
                // [CBE-Handbook p:694 s:24.3.3.3] a hint stays loaded until another replaces it or a sync invalidates it.
                // The sync flushes the pipeline: the SPU fetches the next
                // instruction again.
                self.floor = t + branch_miss_cycles(slot.pc.wrapping_add(4));
                self.clear_hints();
            }
            Kind::Channel(group) => self.channel = Some((group, t)),
        }
    }

    /// Hold the next issue `cycles` past the last: a cost the unit pays
    /// outside the issue rules, such as a getllar's round trip.
    pub(crate) fn charge(&mut self, cycles: u64) {
        self.elapsed += cycles;
        self.floor = self.floor.max(self.elapsed);
        self.pair = None;
    }

    /// Send the next issue to `target`, as the entry to an interrupt does.
    ///
    /// The redirect flushes the pipeline as a mispredicted branch does.
    pub(crate) fn redirect(&mut self, target: u32) {
        let last = self.elapsed.saturating_sub(1);
        self.floor = self.floor.max(last + branch_miss_cycles(target));
        self.pair = None;
        self.fetch.redirect();
    }

    /// Resume after a park on a blocking channel.
    ///
    /// The access issues again once its channel count is not zero, long
    /// after the park. At that cycle:
    ///
    /// - the channel access before it is outside the retry window
    /// - any hint issued before the park is loaded
    ///
    /// [CBE-Handbook p:764 s:B.1.2 Table B-1] note 2: a rdch or wrch for a blocking channel is not issued unless the channel count is greater than zero.
    pub(crate) fn resume_after_park(&mut self) {
        self.channel = None;
        if let Some(p) = self.pending.take() {
            self.hint = Some(p.hint);
        }
    }

    /// Drop every hint and fetch again from the next instruction, as a
    /// sync or a restart does.
    pub(crate) fn clear_hints(&mut self) {
        self.hint = None;
        self.pending = None;
        self.fetch.redirect();
    }
}

/// The group a channel's retry rule compares.
///
/// The three event channels form one group. Every other channel forms
/// its own.
///
/// [CBE-Handbook p:764 s:B.1.2 Table B-1] note 4: the retry applies to two accesses to one channel, or to SPU_RdEventStat, SPU_WrEventMask and SPU_WrEventAck.
fn channel_group(channel: u8) -> u8 {
    use cellgov_ps3_abi::hw::spu;
    match channel {
        spu::SPU_RD_EVENT_STAT | spu::SPU_WR_EVENT_MASK | spu::SPU_WR_EVENT_ACK => {
            spu::SPU_RD_EVENT_STAT
        }
        other => other,
    }
}

fn kind(state: &SpuState, insn: &SpuInstruction) -> Kind {
    use SpuInstruction as I;
    let pc = state.pc;
    let word = |offset: i32| state.insn_addr(pc.wrapping_add((offset as u32) << 2));
    match *insn {
        I::Br { .. }
        | I::Brsl { .. }
        | I::Bra { .. }
        | I::Brasl { .. }
        | I::Brz { .. }
        | I::Brnz { .. }
        | I::Brhz { .. }
        | I::Brhnz { .. }
        | I::Bi { .. }
        | I::Iret { .. }
        | I::Bisl { .. }
        | I::Bisled { .. }
        | I::Biz { .. }
        | I::Binz { .. }
        | I::Bihz { .. }
        | I::Bihnz { .. } => Kind::Branch,
        // [SPU-ISA p:192 s:8] hbr with the P bit set is an inline-prefetch hint and names no branch.
        I::Hbr { p: true, .. } => Kind::Plain,
        I::Hbr { p: false, ra, ro } => Kind::Hint(Hint {
            branch: word(i32::from(ro)),
            target: state.insn_addr(state.reg_word(ra)),
        }),
        I::Hbra { ro, target } => Kind::Hint(Hint {
            branch: word(i32::from(ro)),
            target: state.insn_addr((target as u32) << 2),
        }),
        I::Hbrr { ro, offset } => Kind::Hint(Hint {
            branch: word(i32::from(ro)),
            target: word(offset),
        }),
        I::Sync { .. } => Kind::Sync,
        I::Rdch { channel, .. } | I::Wrch { channel, .. } | I::Rchcnt { channel, .. } => {
            Kind::Channel(channel_group(channel))
        }
        _ => Kind::Plain,
    }
}

/// Whether `insn` takes the local store as it issues: a load, a store,
/// or a hint, which fetches its target.
///
/// [CBE-Handbook p:66 s:3.1.1.3 Table 3-1] SPU loads and stores and hint fetches share priority 3, above inline fetch.
///
/// [CBE-Handbook p:775 s:B.1.4] hbrp arbitrates for the LS as a hint does, then leaves a hole wide enough for an instruction prefetch.
fn uses_ls(insn: &SpuInstruction) -> bool {
    use SpuInstruction as I;
    matches!(
        insn,
        I::Lqd { .. }
            | I::Lqx { .. }
            | I::Lqa { .. }
            | I::Lqr { .. }
            | I::Stqd { .. }
            | I::Stqx { .. }
            | I::Stqa { .. }
            | I::Stqr { .. }
            | I::Hbr { p: false, .. }
            | I::Hbra { .. }
            | I::Hbrr { .. }
    )
}

/// The register `insn` writes and the registers it reads.
fn operands(insn: &SpuInstruction) -> (Option<u8>, [Option<u8>; 3]) {
    use SpuInstruction as I;
    let none = [None; 3];
    match *insn {
        // Loads.
        I::Lqd { rt, ra, .. } => (Some(rt), [Some(ra), None, None]),
        I::Lqx { rt, ra, rb } => (Some(rt), [Some(ra), Some(rb), None]),
        I::Lqa { rt, .. } | I::Lqr { rt, .. } => (Some(rt), none),
        // Stores read RT.
        I::Stqd { rt, ra, .. } => (None, [Some(rt), Some(ra), None]),
        I::Stqx { rt, ra, rb } => (None, [Some(rt), Some(ra), Some(rb)]),
        I::Stqa { rt, .. } | I::Stqr { rt, .. } => (None, [Some(rt), None, None]),

        // Constants; iohl ORs into RT.
        I::Il { rt, .. }
        | I::Ila { rt, .. }
        | I::Ilh { rt, .. }
        | I::Ilhu { rt, .. }
        | I::Fsmbi { rt, .. } => (Some(rt), none),
        I::Iohl { rt, .. } => (Some(rt), [Some(rt), None, None]),

        // Two sources; these also read RT.
        I::Addx { rt, ra, rb }
        | I::Cgx { rt, ra, rb }
        | I::Sfx { rt, ra, rb }
        | I::Bgx { rt, ra, rb }
        | I::Mpyhha { rt, ra, rb }
        | I::Mpyhhau { rt, ra, rb }
        | I::Dfma { rt, ra, rb }
        | I::Dfms { rt, ra, rb }
        | I::Dfnms { rt, ra, rb }
        | I::Dfnma { rt, ra, rb } => (Some(rt), [Some(rt), Some(ra), Some(rb)]),

        // Two sources.
        I::A { rt, ra, rb }
        | I::Sf { rt, ra, rb }
        | I::Avgb { rt, ra, rb }
        | I::Absdb { rt, ra, rb }
        | I::Sumb { rt, ra, rb }
        | I::Mpy { rt, ra, rb }
        | I::Mpyu { rt, ra, rb }
        | I::Mpyh { rt, ra, rb }
        | I::Mpys { rt, ra, rb }
        | I::Mpyhh { rt, ra, rb }
        | I::Mpyhhu { rt, ra, rb }
        | I::Cg { rt, ra, rb }
        | I::Bg { rt, ra, rb }
        | I::Ah { rt, ra, rb }
        | I::Sfh { rt, ra, rb }
        | I::And { rt, ra, rb }
        | I::Or { rt, ra, rb }
        | I::Nor { rt, ra, rb }
        | I::Andc { rt, ra, rb }
        | I::Orc { rt, ra, rb }
        | I::Xor { rt, ra, rb }
        | I::Nand { rt, ra, rb }
        | I::Eqv { rt, ra, rb }
        | I::Rotqby { rt, ra, rb }
        | I::Shlqbi { rt, ra, rb }
        | I::Rotqbi { rt, ra, rb }
        | I::Rotqmbi { rt, ra, rb }
        | I::Shlqby { rt, ra, rb }
        | I::Shlqbybi { rt, ra, rb }
        | I::Rotqbybi { rt, ra, rb }
        | I::Rotqmby { rt, ra, rb }
        | I::Rotqmbybi { rt, ra, rb }
        | I::Shl { rt, ra, rb }
        | I::Rot { rt, ra, rb }
        | I::Rotm { rt, ra, rb }
        | I::Rotma { rt, ra, rb }
        | I::Shlh { rt, ra, rb }
        | I::Roth { rt, ra, rb }
        | I::Rothm { rt, ra, rb }
        | I::Rotmah { rt, ra, rb }
        | I::Cbx { rt, ra, rb }
        | I::Chx { rt, ra, rb }
        | I::Cwx { rt, ra, rb }
        | I::Cdx { rt, ra, rb }
        | I::Ceq { rt, ra, rb }
        | I::Ceqb { rt, ra, rb }
        | I::Ceqh { rt, ra, rb }
        | I::Cgtb { rt, ra, rb }
        | I::Cgth { rt, ra, rb }
        | I::Cgt { rt, ra, rb }
        | I::Clgtb { rt, ra, rb }
        | I::Clgth { rt, ra, rb }
        | I::Clgt { rt, ra, rb }
        | I::Fa { rt, ra, rb }
        | I::Fs { rt, ra, rb }
        | I::Fm { rt, ra, rb }
        | I::Fi { rt, ra, rb }
        | I::Fceq { rt, ra, rb }
        | I::Fcmeq { rt, ra, rb }
        | I::Fcgt { rt, ra, rb }
        | I::Fcmgt { rt, ra, rb }
        | I::Dfa { rt, ra, rb }
        | I::Dfs { rt, ra, rb }
        | I::Dfm { rt, ra, rb } => (Some(rt), [Some(ra), Some(rb), None]),

        // Three sources.
        I::Mpya { rt, ra, rb, rc }
        | I::Selb { rt, ra, rb, rc }
        | I::Shufb { rt, ra, rb, rc }
        | I::Fma { rt, ra, rb, rc }
        | I::Fms { rt, ra, rb, rc }
        | I::Fnms { rt, ra, rb, rc } => (Some(rt), [Some(ra), Some(rb), Some(rc)]),

        // One source, with or without an immediate.
        I::Ai { rt, ra, .. }
        | I::Mpyi { rt, ra, .. }
        | I::Mpyui { rt, ra, .. }
        | I::Ahi { rt, ra, .. }
        | I::Sfhi { rt, ra, .. }
        | I::Sfi { rt, ra, .. }
        | I::Ori { rt, ra, .. }
        | I::Andi { rt, ra, .. }
        | I::Andbi { rt, ra, .. }
        | I::Andhi { rt, ra, .. }
        | I::Orbi { rt, ra, .. }
        | I::Orhi { rt, ra, .. }
        | I::Xorbi { rt, ra, .. }
        | I::Xorhi { rt, ra, .. }
        | I::Xori { rt, ra, .. }
        | I::Shlqbyi { rt, ra, .. }
        | I::Rotqbyi { rt, ra, .. }
        | I::Rotqmbyi { rt, ra, .. }
        | I::Shlqbii { rt, ra, .. }
        | I::Rotqbii { rt, ra, .. }
        | I::Rotqmbii { rt, ra, .. }
        | I::Shli { rt, ra, .. }
        | I::Rotmi { rt, ra, .. }
        | I::Rotmai { rt, ra, .. }
        | I::Roti { rt, ra, .. }
        | I::Shlhi { rt, ra, .. }
        | I::Rothi { rt, ra, .. }
        | I::Rothmi { rt, ra, .. }
        | I::Rotmahi { rt, ra, .. }
        | I::Cbd { rt, ra, .. }
        | I::Chd { rt, ra, .. }
        | I::Cwd { rt, ra, .. }
        | I::Cdd { rt, ra, .. }
        | I::Ceqi { rt, ra, .. }
        | I::Ceqbi { rt, ra, .. }
        | I::Ceqhi { rt, ra, .. }
        | I::Cgtbi { rt, ra, .. }
        | I::Cgthi { rt, ra, .. }
        | I::Clgtbi { rt, ra, .. }
        | I::Clgthi { rt, ra, .. }
        | I::Clgti { rt, ra, .. }
        | I::Cgti { rt, ra, .. }
        | I::Csflt { rt, ra, .. }
        | I::Cflts { rt, ra, .. }
        | I::Cuflt { rt, ra, .. }
        | I::Cfltu { rt, ra, .. } => (Some(rt), [Some(ra), None, None]),
        I::Xsbh { rt, ra }
        | I::Xshw { rt, ra }
        | I::Xswd { rt, ra }
        | I::Clz { rt, ra }
        | I::Cntb { rt, ra }
        | I::Fsmb { rt, ra }
        | I::Fsmh { rt, ra }
        | I::Fsm { rt, ra }
        | I::Gbb { rt, ra }
        | I::Gb { rt, ra }
        | I::Gbh { rt, ra }
        | I::Orx { rt, ra }
        | I::Frest { rt, ra }
        | I::Frsqest { rt, ra }
        | I::Frds { rt, ra }
        | I::Fesd { rt, ra } => (Some(rt), [Some(ra), None, None]),

        // Branches write a link register and read a condition and a
        // target.
        I::Br { .. } | I::Bra { .. } => (None, none),
        I::Brsl { rt, .. } | I::Brasl { rt, .. } => (Some(rt), none),
        I::Brz { rt, .. } | I::Brnz { rt, .. } | I::Brhz { rt, .. } | I::Brhnz { rt, .. } => {
            (None, [Some(rt), None, None])
        }
        I::Bi { ra, .. } | I::Iret { ra, .. } => (None, [Some(ra), None, None]),
        I::Bisl { rt, ra, .. } | I::Bisled { rt, ra, .. } => (Some(rt), [Some(ra), None, None]),
        I::Biz { rt, ra, .. }
        | I::Binz { rt, ra, .. }
        | I::Bihz { rt, ra, .. }
        | I::Bihnz { rt, ra, .. } => (None, [Some(rt), Some(ra), None]),

        // Channels and special-purpose registers.
        I::Rdch { rt, .. } | I::Rchcnt { rt, .. } | I::Mfspr { rt, .. } => (Some(rt), none),
        I::Wrch { rt, .. } | I::Mtspr { rt, .. } => (None, [Some(rt), None, None]),

        // Hints, halts and control.
        I::Hbr { p, ra, .. } => (None, [(!p).then_some(ra), None, None]),
        I::Hbra { .. } | I::Hbrr { .. } => (None, none),
        I::Heq { ra, rb } | I::Hgt { ra, rb } | I::Hlgt { ra, rb } => {
            (None, [Some(ra), Some(rb), None])
        }
        I::Heqi { ra, .. } | I::Hgti { ra, .. } | I::Hlgti { ra, .. } => {
            (None, [Some(ra), None, None])
        }
        I::Nop { .. } | I::Lnop | I::Sync { .. } | I::Dsync | I::Stop { .. } | I::Stopd => {
            (None, none)
        }
        I::Fscrwr { ra } => (None, [Some(ra), None, None]),
        I::Fscrrd { rt } => (Some(rt), none),
    }
}

#[cfg(test)]
#[path = "tests/timing_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/timing_runout_tests.rs"]
mod runout_tests;
