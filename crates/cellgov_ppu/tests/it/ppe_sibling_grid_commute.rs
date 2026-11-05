//! Two PPE threads' steps commute when the dependency relation calls them
//! independent: no memory shared, no status change and steady issue
//! pressure. A batch ends at a grid line of the thread's own clock while a
//! sibling is runnable, so the sibling's clock does not move that line.

use cellgov_core::{Runtime, Scheduler, UnitRegistry};
use cellgov_event::UnitId;
use cellgov_exec::{PpeIssuePressure, UnitStatus};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::{Budget, GuestTicks};
use std::collections::VecDeque;

/// `addi r3,r3,1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;

/// The instructions a batch may retire.
const BUDGET: u64 = 256;

/// The `addi` in each thread's run of code. No step reaches its end, so
/// every instruction a step retires is one `addi`, and the issue pressure
/// settles.
const RUN: u64 = 0x2000;

/// Where each thread's code starts.
const CODE: [u64; 2] = [0x1000, 0x1000 + 4 * RUN];

/// The steps the warm-up may take before the threads' pressure holds.
const WARM_LIMIT: usize = 64;

/// Runs the units it holds in order.
struct Script(VecDeque<UnitId>);

impl Scheduler for Script {
    fn select_next(&mut self, _registry: &UnitRegistry) -> Option<UnitId> {
        self.0.pop_front()
    }
}

/// `RUN` dependent `addi` from `at`.
fn load_run(mem: &mut GuestMemory, at: u64) {
    for i in 0..RUN {
        let range = ByteRange::new(GuestAddr::new(at + 4 * i), 4).expect("range");
        mem.apply_commit(range, &ADDI_R3.to_be_bytes())
            .expect("mapped");
    }
}

/// Two PPE threads, each in its own run of code, stepped at the runtime's
/// own scheduling until a step changes neither thread's issue pressure.
fn warmed() -> (Runtime, [UnitId; 2]) {
    let mut mem = GuestMemory::new(0x1000 + 8 * RUN as usize);
    for at in CODE {
        load_run(&mut mem, at);
    }
    let mut rt = Runtime::new(mem, Budget::new(BUDGET), 10_000);
    let ids = CODE.map(|at| {
        rt.register_unit_with(|id| {
            let mut ppu = PpuExecutionUnit::new(id);
            ppu.state_mut().pc = at;
            ppu
        })
    });
    // Two steps in a row that hold both pressures: one of each thread
    // under the lowest-clock order.
    let mut held_steps = 0;
    for _ in 0..WARM_LIMIT {
        let before = ids.map(|id| pressure(&rt, id));
        let mut step = rt.step().expect("a step");
        rt.commit_step(&mut step).expect("the step commits");
        held_steps = if ids.map(|id| pressure(&rt, id)) == before {
            held_steps + 1
        } else {
            0
        };
        if held_steps == 2 {
            return (rt, ids);
        }
    }
    panic!("the threads' pressure did not settle in {WARM_LIMIT} steps");
}

/// What a step decides of one thread: its pc, its count and its clock.
type Outcome = (u64, u64, GuestTicks);

fn outcome(rt: &Runtime, id: UnitId) -> Outcome {
    let ppu = rt
        .registry()
        .get(id)
        .and_then(|u| u.as_any().downcast_ref::<PpuExecutionUnit>())
        .expect("the PPU unit");
    (ppu.state().pc, ppu.state().gpr[3], rt.registry().clock(id))
}

fn pressure(rt: &Runtime, id: UnitId) -> PpeIssuePressure {
    rt.registry()
        .get(id)
        .expect("the unit")
        .ppe_issue_pressure()
}

/// What [`run_pair`] reports: each thread's outcome after both steps, the
/// `addi` the first step retired, and thread 1's clock as thread 0's step
/// began.
struct Pair {
    outcomes: [Outcome; 2],
    first_retired: u64,
    sibling_clock: GuestTicks,
}

/// One step of each unit in `order` from the warmed state.
fn run_pair(order: [usize; 2]) -> Pair {
    let (mut rt, ids) = warmed();
    let held = ids.map(|id| pressure(&rt, id));
    let count_before = outcome(&rt, ids[order[0]]).1;
    rt.set_scheduler(Script(order.iter().map(|&i| ids[i]).collect()));
    let mut first = None;
    let mut sibling_clock = None;
    for &i in &order {
        if i == 0 {
            sibling_clock = Some(rt.registry().clock(ids[1]));
        }
        let mut step = rt.step().expect("a step");
        assert_eq!(step.unit, ids[i], "the script picks the unit");
        rt.commit_step(&mut step).expect("the step commits");
        // The relation's clauses for two PPE steps that share no memory:
        // neither step moves a thread's status or its pressure.
        for (k, &id) in ids.iter().enumerate() {
            assert_eq!(
                rt.registry().effective_status(id),
                Some(UnitStatus::Runnable),
                "order {order:?}: thread {k} stays runnable"
            );
            assert_eq!(
                pressure(&rt, id),
                held[k],
                "order {order:?}: thread {k}'s pressure holds"
            );
        }
        first.get_or_insert_with(|| outcome(&rt, ids[order[0]]).1 - count_before);
    }
    Pair {
        outcomes: ids.map(|id| outcome(&rt, id)),
        first_retired: first.expect("two steps ran"),
        sibling_clock: sibling_clock.expect("thread 0 stepped"),
    }
}

#[test]
fn two_ppe_steps_the_relation_calls_independent_end_alike_in_either_order() {
    let ab = run_pair([0, 1]);
    let ba = run_pair([1, 0]);
    // Each first step ends at a grid line before it spends its budget, so
    // the rule that ends it is the one under test.
    assert!(
        ab.first_retired < BUDGET,
        "thread 0 retired {}",
        ab.first_retired
    );
    assert!(
        ba.first_retired < BUDGET,
        "thread 1 retired {}",
        ba.first_retired
    );
    // Thread 0's step sees its sibling at a later clock when the sibling
    // steps first.
    assert!(ab.sibling_clock < ba.sibling_clock);
    assert_eq!(
        ab.outcomes, ba.outcomes,
        "the order of the two steps changed an outcome"
    );
}
