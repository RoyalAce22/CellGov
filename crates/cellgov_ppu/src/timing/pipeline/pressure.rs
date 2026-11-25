//! The sibling-pressure accounting: the stall the sibling thread adds to
//! an instruction, and the averages of this thread's own work that the
//! sibling reads.

use super::{
    issues_to_vsu, unit, Core, Kind, Op, Tables, KERNEL_WINDOW_CYCLES, PRESSURE_WINDOW_CYCLES,
    PRESSURE_WINDOW_INSTRUCTIONS, SHARE_ONE, STALL_WINDOW_CYCLES,
};
use cellgov_exec::PpeIssuePressure;
use cellgov_ps3_abi::hw::ppu_issue as k;

/// Whether `op` holds the FXU or the VSU longer than a multiply does: a
/// divide or a square root. Its cycles count in full toward a sibling's
/// stall, and toward the multiply hold a sibling repeats.
///
/// [CBE-Handbook p:301 s:10.2.3.1] an instruction that depends on a
/// divide in progress is a very long-latency stall condition. The captures
/// below are the evidence that the sibling waits through it.
///
/// The `ppu_issue_shared` capture: a sibling of divides adds about 13
/// cycles to each of a thread's instructions, and one of dependent
/// `mulld` about 5. The `ppu_issue_chain` capture's sibling runs one
/// `divd` to each `bdnz`, and still adds 13 or more: the divide's cycles
/// count, whatever the instructions between.
pub(super) fn divides(op: &Op) -> bool {
    let long = matches!(op.kind, Kind::Unpipelined(cycles) if u64::from(cycles) > k::PPU_MULLD_LATENCY_CYCLES);
    long && op.units & (unit::FXU | unit::VSU1 | unit::VSU2) != 0
}

/// Whether `op` issues alone for long without dividing: a microcoded
/// instruction, or a long unpipelined one on another unit (`mfcr` and
/// `mfocrf` on the branch unit). Its cycles count in full toward a
/// sibling's stall, and toward no repeated multiply hold.
///
/// [CBE-Handbook p:753 s:A.5] a nonpipelined or microcoded instruction
/// issues alone.
fn issues_alone_for_long(op: &Op) -> bool {
    let long = matches!(op.kind, Kind::Unpipelined(cycles) if u64::from(cycles) > k::PPU_MULLD_LATENCY_CYCLES);
    !divides(op) && (op.words != 0 || long)
}

/// The cycles a multiply holds the FXU, or `None` for any other
/// instruction: a multiply is an unpipelined FXU instruction that holds
/// it no longer than `mulld` does.
///
/// [CBE-Handbook p:718 s:A.1.2] the FXU does not pipeline `mulli`,
/// `mullw`, `mulhw`, `mulld` or their non-recording forms, and each
/// causes a stall after issue. The recording forms run as microcode.
///
/// Beside a sibling of divides or square roots a multiply waits
/// [`PPU_SMT_SHARED_STALL_CYCLES`] and then this hold again. In the
/// `ppu_issue_multiply` capture, beside dependent `divd` or `fdiv`, the
/// slowest run of each multiply chain waits 23 to 33 cycles an
/// instruction past the chain alone. Beside `fdiv` the means are 22 for
/// `mulli` and 25 to 27 for `mullw`, `mulhw` and `mulld`; beside `divd`,
/// 24 to 26 for those three, and 10 for `mulli`, whose runs spread from
/// 1 to 27. Beside a microcoded `lha` sibling a `mulli` waits 14.8 to
/// 16.2 cycles in the `ppu_issue_shared` capture, and its `add`, `addi`
/// and `ld` chains 12.5 to 14.4.
///
/// [`PPU_SMT_SHARED_STALL_CYCLES`]: k::PPU_SMT_SHARED_STALL_CYCLES
fn multiply_hold(op: &Op) -> Option<u64> {
    match op.kind {
        Kind::Unpipelined(cycles)
            if u64::from(cycles) <= k::PPU_MULLD_LATENCY_CYCLES && op.units == unit::FXU =>
        {
            Some(u64::from(cycles))
        }
        _ => None,
    }
}

/// The cycles the sibling's stalls in `s` add to `op`.
///
/// `own_vscr` marks this thread's own `mtvscr` or a wait at its own
/// `mtvscr` fence. `own_wait` is the cycles `op` already waits past its
/// in-order floor and its fetch.
pub(super) fn sibling_stall(op: &Op, s: &PpeIssuePressure, own_vscr: bool, own_wait: u64) -> u64 {
    // [CBE-Handbook p:301 s:10.2.3.1] the sibling's stalls at IS2
    // stall this thread too. In the `ppu_issue_units` capture the
    // younger of a pair waits the stall as well.
    let stalled = match multiply_hold(op) {
        Some(hold) => {
            u64::from(s.multiply_cycles) + (hold * u64::from(s.multiply_hold_share) + 128) / 256
        }
        None if issues_to_vsu(op) => u64::from(s.vsu_cycles),
        None => u64::from(s.cycles),
    };
    // A sibling's `mtvscr` stall overlaps this thread's own `mtvscr`
    // and its wait at its own fence. The `ppu_issue_vscr_smt` capture
    // times a loop of an `mtvscr` and four `vaddubm` beside a sibling of
    // `mtvscr`. On the mean of five runs, the model with the overlap lies
    // 22% over the console, and with no overlap 37% over.
    if own_vscr {
        return stalled;
    }
    // Each instruction waits the sibling's drain at IS2. A VSU instruction
    // waits it only past its own wait at issue: for its operands, its
    // unit, or an older `mfvscr` or drain. In the
    // `ppu_issue_vscr_smt_runs` capture the sibling runs an `mtvscr` and
    // seven `vaddubm`. Beside it, 8 independent `vaddubm` take 63.54 cycles
    // an iteration on the median shared run, and 8 `li` take 57.84. In the
    // `ppu_issue_vscr_smt` capture 8 dependent `fadd` beside that sibling
    // take 117.40. A store waits all of the drain and only a share of the
    // `mtvscr` stall (the `ppu_issue_vscr_smt_drain` capture).
    let drain = if issues_to_vsu(op) {
        u64::from(s.drain_cycles).saturating_sub(own_wait)
    } else {
        u64::from(s.drain_cycles)
    };
    let vscr = if matches!(op.kind, Kind::Store { .. }) {
        let (num, den) = k::PPU_SMT_MTVSCR_STORE_STALL_SHARE;
        u64::from(s.vscr_cycles) * num / den
    } else {
        u64::from(s.vscr_cycles)
    } + drain;
    stalled.max(vscr)
}

/// One instruction's `w` cycles in a share averaged over
/// [`PRESSURE_WINDOW_CYCLES`]: toward [`SHARE_ONE`] when `counts`, toward
/// zero otherwise.
fn weigh(share: u32, w: u64, counts: bool) -> u32 {
    weigh_over(share, w, counts, PRESSURE_WINDOW_CYCLES)
}

/// [`weigh`] over a window of `window` cycles.
pub(super) fn weigh_over(share: u32, w: u64, counts: bool, window: u64) -> u32 {
    let w = w.min(window);
    let kept = u64::from(share) * (window - w);
    let added = if counts { u64::from(SHARE_ONE) * w } else { 0 };
    ((kept + added) / window) as u32
}

/// Delays each pending time by the length of the sibling's stall.
///
/// A time is pending when it is later than the start of the stall:
/// - each result and each outstanding load miss;
/// - each unit's release;
/// - each queued store's write into the cache;
/// - each touched line's arrival;
/// - the cycle a touched line's L1 set let its request go;
/// - each L1 set's next free reload.
///
/// These times do not move:
/// - the cycle a slot of the store queue is free again;
/// - the cycle the next touch can send its request;
/// - the `mtvscr` fence;
/// - the cycle a flush or store-back finishes with its line. No capture
///   times one whose busy time outlasts the stalls after it.
///
/// The console captures show the delay:
/// - `ppu_issue_units`: a dependent `fadd` with an independent `addi`
///   after each waits its latency and both stalls.
/// - `ppu_issue_miss`: an L2 miss beside an `fdiv` sibling waits out
///   the stall of every instruction after it, about 40 cycles each
///   there, past the model's own stall.
/// - `ppu_issue_stall_release`, beside an `fdiv` sibling: a load of
///   bytes a store just queued costs more over a load of another line
///   than it does alone. This is also true with four `addi` between the
///   two, whose stalls outlast the store's write when that write does
///   not move.
/// - `ppu_issue_stall_release`, beside an `fdiv` sibling: a `divd` with
///   four independent loads after it costs most of what it costs alone
///   over an `addi` with the same loads. The loads' stalls outlast the
///   divide when its release does not move.
/// - `ppu_issue_stall_pending`, beside an `fdiv` sibling, in the fastest
///   runs: a touch, 8 `addi` and a load of the touched line take 172
///   cycles an iteration more than alone. An `mtvscr`, 8 `addi` and a
///   `vaddubm` take 88.69 cycles more than the same loop with no `addi`
///   there.
///
/// While a sibling's divides stall the shared issue point back to back,
/// the thread's dispatch groups wait the window apart
/// ([`PpeIssuePressure::window_cycles`]). The thread then waits no stall,
/// so no pending time moves.
#[inline]
pub(super) fn stretch_pending(c: &mut Core, tables: &mut Tables, began: u64, stalled: u64) {
    if stalled == 0 {
        return;
    }
    tables.caches.stretch_touches(began, stalled);
    let pending = c
        .ready
        .iter_mut()
        .chain(c.addr_ready.iter_mut())
        .chain(c.miss.iter_mut())
        .chain(c.unit_free.iter_mut())
        .chain(tables.stores.iter_mut().map(|s| &mut s.done));
    for r in pending {
        if *r > began {
            *r += stalled;
        }
    }
}

/// Add a divide that issued at cycle `t` to [`Core::divide_interval`].
///
/// A gap of more than [`STALL_WINDOW_CYCLES`] adds no sample. The other
/// work in such a gap takes [`Core::stall_share`] far under
/// [`WINDOW_STALL_SHARE`], so the two divides do not stall the shared
/// issue point back to back.
///
/// [`WINDOW_STALL_SHARE`]: super::WINDOW_STALL_SHARE
pub(super) fn note_divide(c: &mut Core, t: u64) {
    if let Some(last) = c.last_divide {
        let gap = t.saturating_sub(last);
        if gap <= STALL_WINDOW_CYCLES {
            let n = u64::from(PRESSURE_WINDOW_INSTRUCTIONS);
            c.divide_interval = ((u64::from(c.divide_interval) * (n - 1) + gap * 256) / n) as u32;
        }
    }
    c.last_divide = Some(t);
}

/// Add an instruction to the averages [`PpeTiming::pressure`] reads.
///
/// The instruction took `own` cycles of this thread's own, and waited
/// `held` more for the sibling's dispatch group. A drain spent `queued` of
/// the `own` cycles on the ops the VSU issue queue held. The instruction's
/// VSU work held the head of that queue `head.0` cycles, `head.1` of them
/// at the fence.
///
/// [`PpeTiming::pressure`]: super::PpeTiming::pressure
pub(super) fn note_pressure(
    c: &mut Core,
    op: &Op,
    own: u64,
    held: u64,
    queued: u64,
    head: (u64, u64),
) {
    let n = PRESSURE_WINDOW_INSTRUCTIONS;
    let mean = |mean: u32, w: u64| {
        let w = w.saturating_mul(256).min(u64::from(u32::MAX));
        ((u64::from(mean) * u64::from(n - 1) + w) / u64::from(n)) as u32
    };
    let interval = own.saturating_mul(256).min(u64::from(u32::MAX));
    c.interval = ((u64::from(c.interval) * u64::from(n - 1) + interval) / u64::from(n)) as u32;
    c.queue_wait = mean(c.queue_wait, head.0);
    c.fence_wait = mean(c.fence_wait, head.1);
    let divide = divides(op);
    let alone = issues_alone_for_long(op);
    c.divide_share = weigh(c.divide_share, own, divide);
    // The stalls stop the sibling for their whole length, so their share
    // is of the cycles this thread dispatches in, its holds for the
    // sibling's dispatch group included.
    c.stall_share = weigh_over(c.stall_share, own + held, divide, STALL_WINDOW_CYCLES);
    c.alone_share = weigh(c.alone_share, own, alone);
    c.kernel_share = weigh_over(c.kernel_share, own, false, KERNEL_WINDOW_CYCLES);
    // An `mtvscr` that drains its own thread's held vector work leaves the
    // sibling most of its issue slots, so the drain counts in a share of
    // its own (`PPU_SMT_MTVSCR_DRAIN_STALL_SHARE`). In the
    // `ppu_issue_vscr_smt` capture a sibling of an `mtvscr` and seven
    // `vaddubm` slows a `vaddubm` chain from 17.9 to 61.4 cycles, and one
    // of back-to-back `mtvscr` to 220.
    let queued = queued.min(own);
    let vscr = op.kind == Kind::VscrWrite;
    c.vscr_share = weigh(c.vscr_share, queued, false);
    c.vscr_share = weigh(c.vscr_share, own - queued, vscr);
    c.drain_share = weigh(c.drain_share, own - queued, false);
    c.drain_share = weigh(c.drain_share, queued, vscr);
    if divide || alone {
        return;
    }
    let mut wait = own.saturating_sub(1).saturating_mul(256);
    // [CBE-Handbook p:756 s:A.6.2] a VSU instruction waits for its
    // operands in the VSU issue queue, past IS2.
    // [CBE-Handbook p:301 s:10.2.3.1] it stalls the sibling only once the
    // queue fills.
    if issues_to_vsu(op) {
        let (num, den) = k::PPU_SMT_VSU_STALL_SHARE;
        wait = wait.saturating_mul(num) / den;
    }
    let wait = u32::try_from(wait).unwrap_or(u32::MAX);
    c.operand_wait =
        ((u64::from(c.operand_wait) * u64::from(n - 1) + u64::from(wait)) / u64::from(n)) as u32;
}
