use super::*;
use cellgov_exec::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

fn attrs(priority: i32) -> cellgov_lv2::PpuThreadAttrs {
    cellgov_lv2::PpuThreadAttrs {
        entry: 0,
        arg: 0,
        stack_base: 0,
        stack_size: 0,
        priority: priority as u32,
        tls_base: 0,
    }
}

/// A runtime with one PPE unit per entry of `threads`: `Some(priority)`
/// gives the unit an LV2 thread record at that priority, `None` none.
/// Each unit runs `ops` single-step opcodes, then ends.
fn runtime(threads: &[Option<i32>], ops: usize) -> (Runtime, Vec<UnitId>) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 10_000);
    let mut ids = Vec::new();
    let mut program = vec![FakeOp::LoadImm(1); ops];
    program.push(FakeOp::End);
    for _ in threads {
        let p = program.clone();
        ids.push(rt.register_unit_with(|id| FakeIsaUnit::new(id, p).as_ppe_thread()));
    }
    let mut primary = true;
    for (&id, prio) in ids.iter().zip(threads) {
        let Some(prio) = *prio else { continue };
        if primary {
            rt.lv2_host_mut().seed_primary_ppu_thread(id, attrs(prio));
            primary = false;
        } else {
            rt.lv2_host_mut()
                .ppu_threads_mut()
                .create(id, attrs(prio))
                .expect("a thread id");
        }
    }
    (rt, ids)
}

/// Run `steps` steps and return how many each of `ids` took.
fn steps_taken(rt: &mut Runtime, ids: &[UnitId], steps: usize) -> Vec<usize> {
    let mut taken = vec![0; ids.len()];
    for _ in 0..steps {
        let Ok(step) = rt.step() else { break };
        if let Some(k) = ids.iter().position(|&id| id == step.unit) {
            taken[k] += 1;
        }
        rt.commit_unchecked(&step.result, &step.effects)
            .expect("the step commits");
    }
    taken
}

#[test]
fn a_third_thread_of_equal_priority_waits_while_two_run() {
    let (mut rt, ids) = runtime(&[Some(1500), Some(1500), Some(1500)], 1000);
    let taken = steps_taken(&mut rt, &ids, 100);
    assert_eq!(taken[2], 0, "{taken:?}");
    assert!(taken[0] > 0 && taken[1] > 0, "{taken:?}");
    assert_eq!(
        rt.registry().effective_status(ids[2]),
        Some(UnitStatus::Suspended)
    );
}

#[test]
fn the_two_highest_priority_threads_take_the_hardware_threads() {
    let (mut rt, ids) = runtime(&[Some(1500), Some(1600), Some(1400)], 1000);
    let taken = steps_taken(&mut rt, &ids, 100);
    assert_eq!(taken[1], 0, "the 1600 thread waits: {taken:?}");
    assert!(taken[0] > 0 && taken[2] > 0, "{taken:?}");
}

#[test]
fn a_higher_priority_thread_that_wakes_preempts_the_lowest_running_one() {
    let (mut rt, ids) = runtime(&[Some(1500), Some(1600), Some(1400)], 1000);
    rt.set_unit_status_override(ids[2], UnitStatus::Blocked);
    let before = steps_taken(&mut rt, &ids, 20);
    assert!(
        before[0] > 0 && before[1] > 0 && before[2] == 0,
        "{before:?}"
    );
    rt.set_unit_status_override(ids[2], UnitStatus::Runnable);
    let after = steps_taken(&mut rt, &ids, 100);
    assert_eq!(
        after[1], 0,
        "the 1600 thread gave up its hardware thread: {after:?}"
    );
    assert!(after[0] > 0 && after[2] > 0, "{after:?}");
}

#[test]
fn a_thread_that_finishes_frees_its_hardware_thread_for_the_one_waiting() {
    let (mut rt, ids) = runtime(&[Some(1500), Some(1500), Some(1500)], 10);
    let taken = steps_taken(&mut rt, &ids, 200);
    assert_eq!(taken, vec![11, 11, 11]);
}

#[test]
fn a_unit_with_no_lv2_thread_is_outside_the_rule() {
    let (mut rt, ids) = runtime(&[None, None, None], 1000);
    let taken = steps_taken(&mut rt, &ids, 99);
    assert_eq!(taken, vec![33, 33, 33]);
}

/// The runnable set a caller reads between a commit and the next step,
/// as the explorer does to choose a branch, is the one that step
/// chooses from.
#[test]
fn every_commit_leaves_the_placement_current() {
    let (mut rt, ids) = runtime(&[Some(1500), Some(1500), Some(1500), Some(1500)], 6);
    let mut commits = 0;
    while let Ok(step) = rt.step() {
        rt.commit_unchecked(&step.result, &step.effects)
            .expect("the step commits");
        commits += 1;
        let runnable = ids
            .iter()
            .filter(|&&id| rt.registry().effective_status(id) == Some(UnitStatus::Runnable))
            .count();
        let waiting = ids
            .iter()
            .filter(|&&id| rt.registry().is_off_core(id))
            .count();
        assert!(runnable <= 2, "commit {commits}: {runnable} runnable");
        assert!(
            waiting == 0 || runnable == 2,
            "commit {commits}: {waiting} waiting beside {runnable} running"
        );
    }
    assert_eq!(commits, 4 * 7, "every thread ran to its end");
}

/// The placement's sync-state terms are a wire format: a change to
/// which lanes it writes moves every recorded checkpoint hash.
#[test]
fn ppe_placement_sync_term_wire_format_golden() {
    let mut p = PpePlacement::default();
    assert_eq!(p.sync_term(), 0, "an empty placement adds no term");
    p.on_core = vec![UnitId::new(3)];
    p.queued.insert(UnitId::new(3), 0);
    p.queued.insert(UnitId::new(5), 1);
    p.next_seq = 2;
    p.yielded = Some(UnitId::new(5));
    assert_eq!(
        p.sync_term(),
        48_436_272_174_534_908_728_985_807_271_667_032_129
    );
}

/// The `ppu_sched_third` capture's payload as big-endian u64 words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_sched_third/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[151], 0x5050_5354, "the probe wrote every word");
    words
}

/// Spinner `k` of phase `p`'s round count on the console.
fn console_rounds(words: &[u64], p: usize, k: usize) -> u64 {
    let at = 8 * (3 * p + k);
    assert_eq!(
        (words[at], words[at + 1]),
        (0, 0),
        "phase {p} spinner {k} created and joined"
    );
    words[at + 3]
}

#[test]
fn the_threads_that_wait_are_the_ones_that_wait_on_the_console() {
    let words = console_words();
    // Phases 0 and 1: three spinners while the main thread sleeps.
    // Phases 4 and 5: the busy main thread at 1001 beside two spinners.
    let phases: [(usize, &[Option<i32>], usize); 4] = [
        (0, &[Some(1500), Some(1500), Some(1500)], 0),
        (1, &[Some(1400), Some(1500), Some(1600)], 0),
        (4, &[Some(1001), Some(2000), Some(2000)], 1),
        (5, &[Some(1001), Some(1001), Some(1001)], 1),
    ];
    let mut waiting = 0;
    for (p, threads, first_spinner) in phases {
        let (mut rt, ids) = runtime(threads, 1000);
        let taken = steps_taken(&mut rt, &ids, 120);
        for k in 0..threads.len() - first_spinner {
            let console_waits = console_rounds(&words, p, k) == 0;
            waiting += usize::from(console_waits);
            assert_eq!(
                taken[first_spinner + k] == 0,
                console_waits,
                "phase {p} spinner {k}: CellGov {taken:?}"
            );
        }
    }
    assert_eq!(waiting, 4, "one spinner waits in each phase");
}

#[test]
fn a_thread_that_yields_waits_behind_the_one_that_was_waiting() {
    let words = console_words();
    assert!(
        console_rounds(&words, 2, 0) * 1000 < console_rounds(&words, 2, 1),
        "phase 2: the yielding spinner all but stops on the console"
    );
    let (mut rt, ids) = runtime(&[Some(1500), Some(1500), Some(1500)], 1000);
    steps_taken(&mut rt, &ids, 10);
    // Spinner 0's sys_ppu_thread_yield, through the syscall dispatch.
    let mut args = [0u64; 9];
    args[0] = cellgov_ps3_abi::lv2::syscall::PPU_THREAD_YIELD;
    let yield_call = cellgov_exec::ExecutionStepResult {
        yield_reason: cellgov_exec::YieldReason::Syscall,
        consumed_cost: cellgov_time::InstructionCost::new(1),
        local_diagnostics: cellgov_exec::LocalDiagnostics::with_pc(0x1000),
        fault: None,
        syscall_args: Some(args),
    };
    rt.dispatch_syscall(&yield_call, ids[0]);
    let taken = steps_taken(&mut rt, &ids, 100);
    assert_eq!(taken[0], 0, "{taken:?}");
    assert!(taken[1] > 0 && taken[2] > 0, "{taken:?}");
}
