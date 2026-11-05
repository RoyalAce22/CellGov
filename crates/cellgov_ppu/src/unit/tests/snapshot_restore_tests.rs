//! `PpuExecutionUnit::restore` is the exact inverse of the snapshot, and
//! the differential snapshot, the observation and the fingerprint are
//! projections of it.

use crate::differential::PpuStateSnapshot;
use crate::instruction::encode::encode;
use crate::instruction::PpuInstruction as I;
use crate::observation::PpuArchitecturalState;
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, UnitStatus};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_sync::ReservedLine;
use cellgov_time::Budget;

/// The word the loop reserves and loads.
const DATA: u64 = 0x80;

/// `loop: addi r3, r3, 1; lwarx r4, 0, r5; add r4, r4, r3; eieio; b loop`,
/// with `0x1234_5678` at [`DATA`].
fn program() -> GuestMemory {
    let words = [
        I::Addi {
            rt: 3,
            ra: 3,
            imm: 1,
        },
        I::Lwarx {
            rt: 4,
            ra: 0,
            rb: 5,
        },
        I::Add {
            rt: 4,
            ra: 4,
            rb: 3,
            oe: false,
            rc: false,
        },
        I::Eieio,
        I::B {
            offset: -16,
            aa: false,
            link: false,
        },
    ];
    let code: Vec<u8> = words
        .iter()
        .flat_map(|insn| encode(insn).expect("encodable").to_be_bytes())
        .collect();
    let mut mem = GuestMemory::new(0x100);
    let range = ByteRange::new(GuestAddr::new(0), code.len() as u64).expect("range");
    mem.apply_commit(range, &code).expect("mapped");
    let data = ByteRange::new(GuestAddr::new(DATA), 4).expect("range");
    mem.apply_commit(data, &0x1234_5678u32.to_be_bytes())
        .expect("mapped");
    mem
}

/// A unit at the loop with `r5 = DATA`, after one batch of `warm`
/// instructions, so the reservation is held and the issue model has
/// moved.
fn warmed_unit(mem: &GuestMemory, warm: u64) -> PpuExecutionUnit {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(5, DATA);
    let ctx = ExecutionContext::new(mem);
    unit.run_until_yield(Budget::new(warm), &ctx, &mut Vec::new());
    unit
}

/// What one traced batch leaves behind. The full states drop their
/// retirement count, which a restore keeps.
struct Batch {
    hashes: Vec<(u64, u64)>,
    full_states: Vec<(u64, cellgov_exec::PpuFingerprint)>,
    barriers: Vec<cellgov_exec::RetiredBarrier>,
    effects: Vec<Effect>,
    snapshot: PpuSnapshot,
}

/// One batch of 12 instructions with per-step hashes and full states.
fn traced_batch(unit: &mut PpuExecutionUnit, mem: &GuestMemory, effects: &mut Vec<Effect>) {
    unit.set_full_state_window(Some((0, u64::MAX)));
    let ctx = ExecutionContext::new(mem).with_trace_per_step(true);
    unit.run_until_yield(Budget::new(12), &ctx, effects);
}

/// One traced batch, drained.
fn run(unit: &mut PpuExecutionUnit, mem: &GuestMemory) -> Batch {
    let mut effects = Vec::new();
    traced_batch(unit, mem, &mut effects);
    Batch {
        hashes: unit.drain_retired_state_hashes(),
        full_states: unit
            .drain_retired_state_full()
            .into_iter()
            .map(|(_, pc, fingerprint)| (pc, fingerprint))
            .collect(),
        barriers: unit.drain_barriers(),
        effects,
        snapshot: unit.snapshot(),
    }
}

#[test]
fn a_run_from_a_restored_snapshot_repeats_the_first_run() {
    let mem = program();
    let mut unit = warmed_unit(&mem, 7);
    let snapshot = unit.snapshot();
    assert!(
        snapshot.state.reservation.is_some(),
        "precondition: the warm-up holds a reservation"
    );
    let first = run(&mut unit, &mem);
    assert_ne!(
        first.snapshot, snapshot,
        "precondition: the run moves the unit"
    );
    assert!(!first.hashes.is_empty(), "precondition: per-step hashes");
    assert!(!first.full_states.is_empty(), "precondition: full states");
    assert!(
        !first.barriers.is_empty(),
        "precondition: a barrier retires"
    );

    // Trace output left undrained at the restore must not reach the
    // second run.
    traced_batch(&mut unit, &mem, &mut Vec::new());
    unit.restore(snapshot.clone());
    assert_eq!(unit.snapshot(), snapshot);
    let second = run(&mut unit, &mem);

    assert_eq!(second.hashes, first.hashes, "per-step hashes");
    assert_eq!(second.full_states, first.full_states, "full states");
    assert_eq!(second.barriers, first.barriers, "barriers");
    assert_eq!(second.effects, first.effects, "effects");
    assert_eq!(second.snapshot, first.snapshot, "snapshot after the run");
}

/// splitmix64: a fixed stream of well-mixed nonzero words.
struct Words(u64);

impl Words {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) | 1
    }

    fn word(&mut self) -> u32 {
        (self.next() >> 32) as u32 | 1
    }
}

/// Architected state with every field nonzero and the reservation held.
fn randomized_state(seed: u64) -> PpuArchitecturalState {
    let mut w = Words(seed);
    PpuArchitecturalState {
        gpr: std::array::from_fn(|_| w.next()),
        fpr: std::array::from_fn(|_| w.next()),
        vr: std::array::from_fn(|_| u128::from(w.next()) << 64 | u128::from(w.next())),
        pc: w.next() & !3,
        cr: w.word(),
        lr: w.next(),
        ctr: w.next(),
        xer: w.next(),
        fpscr: w.word(),
        vrsave: w.word(),
        // The state stores only NJ and SAT.
        vscr: cellgov_ps3_abi::hw::ppu::VSCR_NJ | cellgov_ps3_abi::hw::ppu::VSCR_SAT,
        tb: w.next(),
        reservation: Some(ReservedLine::containing(
            w.next() & cellgov_ps3_abi::hw::ppu::CELL_EA_LIMIT,
        )),
    }
}

/// A unit in a randomized context: the warmed issue model, a randomized
/// architected state, a debug break and a blocked status.
fn randomized_snapshot(seed: u64) -> PpuSnapshot {
    let mem = program();
    let mut unit = warmed_unit(&mem, 9);
    randomized_state(seed).apply(unit.state_mut());
    unit.set_break_pc(0x4000, 3);
    unit.status = UnitStatus::Blocked;
    unit.snapshot()
}

#[test]
fn a_randomized_snapshot_restores_into_a_fresh_unit_unchanged() {
    for seed in [1, 0xC0FFEE, u64::MAX] {
        let snapshot = randomized_snapshot(seed);
        let fresh = PpuExecutionUnit::new(UnitId::new(1)).snapshot();
        assert_differs_in_every_field(&snapshot, &fresh);

        let mut unit = PpuExecutionUnit::new(UnitId::new(1));
        unit.restore(snapshot.clone());

        assert_eq!(unit.snapshot(), snapshot, "seed {seed:#x}");
        assert!(unit.state().hash_is_current(), "seed {seed:#x}");
    }
}

/// Every field of `a` differs from `b`, so a restore that drops any one
/// field leaves the restored unit unequal to `a`.
fn assert_differs_in_every_field(a: &PpuSnapshot, b: &PpuSnapshot) {
    let (s, t) = (&a.state, &b.state);
    for k in 0..32 {
        assert_ne!(s.gpr[k], t.gpr[k], "gpr {k}");
        assert_ne!(s.fpr[k], t.fpr[k], "fpr {k}");
        assert_ne!(s.vr[k], t.vr[k], "vr {k}");
    }
    assert_ne!(s.pc, t.pc, "pc");
    assert_ne!(s.cr, t.cr, "cr");
    assert_ne!(s.lr, t.lr, "lr");
    assert_ne!(s.ctr, t.ctr, "ctr");
    assert_ne!(s.xer, t.xer, "xer");
    assert_ne!(s.fpscr, t.fpscr, "fpscr");
    assert_ne!(s.vrsave, t.vrsave, "vrsave");
    assert_ne!(s.vscr, t.vscr, "vscr");
    assert_ne!(s.tb, t.tb, "tb");
    assert_ne!(s.reservation, t.reservation, "reservation");
    assert_ne!(a.status, b.status, "status");
    assert_ne!(a.break_pc, b.break_pc, "break_pc");
    assert_ne!(a.break_skip, b.break_skip, "break_skip");
    assert_ne!(a.timing, b.timing, "timing");
}

#[test]
fn every_projection_agrees_with_the_snapshot_on_the_fields_it_carries() {
    let snapshot = randomized_snapshot(7);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.restore(snapshot.clone());
    let s = &snapshot.state;

    assert_eq!(&PpuArchitecturalState::capture(unit.state()), s);

    let differential = PpuStateSnapshot::capture(unit.state());
    assert_eq!(differential.gpr, s.gpr);
    assert_eq!(differential.fpr, s.fpr);
    assert_eq!(differential.vr, s.vr);
    assert_eq!(differential.cr, s.cr);
    assert_eq!(differential.lr, s.lr);
    assert_eq!(differential.ctr, s.ctr);
    assert_eq!(differential.xer, s.xer);
    assert_eq!(differential.fpscr, s.fpscr);
    assert_eq!(differential.vrsave, s.vrsave);
    assert_eq!(differential.vscr, s.vscr);
    assert_eq!(differential.reservation, s.reservation);

    let fingerprint = unit.state().fingerprint();
    assert_eq!(fingerprint.gpr, s.gpr);
    assert_eq!(fingerprint.fpr, s.fpr);
    assert_eq!(fingerprint.vr, s.vr);
    assert_eq!(fingerprint.cr, s.cr);
    assert_eq!(fingerprint.lr, s.lr);
    assert_eq!(fingerprint.ctr, s.ctr);
    assert_eq!(fingerprint.xer, s.xer);
    assert_eq!(fingerprint.fpscr, s.fpscr);
    assert_eq!(fingerprint.vrsave, s.vrsave);
    assert_eq!(fingerprint.vscr, s.vscr);
    assert_eq!(
        fingerprint.reservation_line,
        s.reservation.map(|line| line.addr())
    );
}
