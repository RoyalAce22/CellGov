//! Worker-thread callback-dispatch end-to-end test: a real PPU
//! worker steps through a synthetic callback body, hits its
//! terminal `blr`, lands on the runtime-installed trampoline,
//! fires `sc 0x80000`, and the runtime classifies the syscall as
//! `CallbackDispatchReturn` and unparks the parent with the
//! worker's `r3` in the parent's `r3`.
//!
//! This covers the happy-path and r11-clobber tests. The
//! lower-level cellgov_lv2 / cellgov_core tests cover dispatch
//! shape, recursion cap, depth tracking, and args round-trip
//! without needing real instruction execution.

use std::collections::BTreeSet;

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::{
    CallbackReturnStage, Lv2Dispatch, PendingResponse, PpuThreadAttrs, PpuThreadInitState,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::callback_dispatch::{
    CALLBACK_RETURN_CODE_ADDR, CALLBACK_RETURN_OPD_ADDR, TRAMPOLINE_CODE_BYTES,
    TRAMPOLINE_OPD_BYTES,
};
use cellgov_ps3_abi::cell_errors::CELL_EFAULT;
use cellgov_ps3_abi::process_address_space::{
    PS3_CHILD_STACKS_BASE, PS3_CHILD_STACKS_SIZE, PS3_PRIMARY_STACK_BASE, PS3_PRIMARY_STACK_SIZE,
};
use cellgov_time::Budget;

/// Sentinel value for the parent's `PendingResponse::CallbackReturn`
/// `args` slot. Distinct from any worker-init register value used in
/// these tests so an arg-source confusion (the runtime returning the
/// pending-response args instead of the worker's captured r3..=r10)
/// surfaces as a wrong r3 rather than an accidentally-matching zero.
const PENDING_SENTINEL: u64 = 0xDEAD_DEAD_DEAD_DEADu64;

/// Tight ceiling for `step_until_parent_unparks`: enough margin for
/// the longest synthetic callback in this file (3 body insns + 3
/// trampoline insns = 6 worker steps; the loop returns after the
/// step that flips parent status, so 6 is the expected exact count).
/// A regression that causes the worker to spin or take a different
/// path through the trampoline trips this bound rather than
/// silently passing under a generous slack.
const UNPARK_STEP_CEILING: usize = 10;

const PARENT_PC: u64 = 0x10_8000;
const CALLBACK_BODY_PC: u64 = 0x10_4000;

/// Build a Runtime with a multi-region memory layout that includes
/// the callback-dispatch trampoline region. Mirrors the production
/// setup in `apps/cellgov_cli/src/game/boot.rs` minus the ELF load.
fn build_runtime_with_trampoline() -> Runtime {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x40_0000, "main", PageSize::Page64K),
        Region::new(
            PS3_PRIMARY_STACK_BASE,
            PS3_PRIMARY_STACK_SIZE,
            "stack",
            PageSize::Page4K,
        ),
        Region::new(
            PS3_CHILD_STACKS_BASE,
            PS3_CHILD_STACKS_SIZE,
            "child_stacks",
            PageSize::Page4K,
        ),
    ])
    .expect("region layout");

    // Install the trampoline body and OPD slot inside the main
    // region (pre-user-heap scratch zone).
    mem.apply_commit(
        ByteRange::new(
            GuestAddr::new(CALLBACK_RETURN_CODE_ADDR as u64),
            TRAMPOLINE_CODE_BYTES.len() as u64,
        )
        .unwrap(),
        &TRAMPOLINE_CODE_BYTES,
    )
    .unwrap();
    mem.apply_commit(
        ByteRange::new(
            GuestAddr::new(CALLBACK_RETURN_OPD_ADDR as u64),
            TRAMPOLINE_OPD_BYTES.len() as u64,
        )
        .unwrap(),
        &TRAMPOLINE_OPD_BYTES,
    )
    .unwrap();

    let mut rt = Runtime::new(mem, Budget::new(1), 10_000);
    rt.set_ppu_factory(|id, init| {
        let mut unit = PpuExecutionUnit::new(id);
        {
            let state = unit.state_mut();
            state.pc = init.entry_code;
            state.gpr[1] = init.stack_top;
            state.gpr[2] = init.entry_toc;
            state.gpr[3] = init.arg;
            for (i, value) in init.extra_args.iter().enumerate() {
                state.gpr[4 + i] = *value;
            }
            state.gpr[13] = init.tls_base;
            state.lr = init.lr_sentinel;
        }
        Box::new(unit)
    });
    rt
}

/// Encode an `addi rT, rA, SIMM` instruction (PPC opcode 14).
fn enc_addi(rt: u32, ra: u32, simm: i16) -> u32 {
    (14 << 26) | (rt << 21) | (ra << 16) | (simm as u16 as u32)
}

/// Encode `li rT, SIMM` (= `addi rT, 0, SIMM`).
fn enc_li(rt: u32, simm: i16) -> u32 {
    enc_addi(rt, 0, simm)
}

/// Encode `blr`: branch to LR. Form: `bclr 20, 0, 0` -- branch
/// always, no link.
fn enc_blr() -> u32 {
    // PPC: opcode 19, BO=20 (always), BI=0, LK=0; XO=16 (BCLR).
    (19 << 26) | (20 << 21) | (16 << 1)
}

fn write_u32_be(mem: &mut GuestMemory, addr: u64, raw: u32) {
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(addr), 4).unwrap(),
        &raw.to_be_bytes(),
    )
    .unwrap();
}

/// Drive the runtime until `parent` transitions out of `Blocked`,
/// up to `max_steps`. Returns the number of steps consumed. On
/// timeout, dumps parent and worker status + the parent's pending
/// syscall return so a CI failure is immediately diagnosable
/// (otherwise "did not unpark in N steps" leaves the operator with
/// no clue whether the worker hung, the trampoline failed, or the
/// dispatch wake path silently dropped the wake). PPU register
/// state isn't reachable from the registry's dyn trait without an
/// `as_any` widening; status + step count are enough to localize.
fn step_until_parent_unparks(
    rt: &mut Runtime,
    parent: UnitId,
    worker: UnitId,
    max_steps: usize,
) -> usize {
    for steps in 0..max_steps {
        let status = rt.registry().effective_status(parent);
        if status != Some(UnitStatus::Blocked) {
            return steps;
        }
        let s = rt.step().expect("step");
        rt.commit_step(&s.result, &s.effects).expect("commit");
    }
    let parent_status = rt.registry().effective_status(parent);
    let worker_status = rt.registry().effective_status(worker);
    panic!(
        "parent did not unpark within {max_steps} steps; \
         parent status = {parent_status:?}, worker status = {worker_status:?}",
    );
}

/// Park `parent` on a synthetic callback by constructing a
/// CallbackSpawn dispatch and routing it through
/// `apply_callback_spawn`. Returns the worker's `UnitId`, captured
/// by diffing the registry before/after the spawn (the runtime's
/// public spawn entry does not expose the new id directly).
///
/// The parent's `PendingResponse::CallbackReturn` payload is
/// pre-filled with [`PENDING_SENTINEL`] in every slot so an
/// arg-source confusion (the runtime returning the pending-response
/// args instead of the worker's captured r3..=r10) trips a
/// detectable wrong-r3 check rather than passing on accidentally
/// matching zeros.
fn park_parent_on_callback(
    rt: &mut Runtime,
    parent: UnitId,
    entry_code: u64,
    args: [u64; 8],
) -> UnitId {
    let mut extra_args = [0u64; 7];
    extra_args.copy_from_slice(&args[1..]);
    let stack = rt
        .lv2_host_mut()
        .allocate_child_stack(0x4000, 0x10)
        .expect("worker stack");
    let init = PpuThreadInitState {
        entry_code,
        entry_toc: 0,
        arg: args[0],
        extra_args,
        stack_top: stack.initial_sp(),
        tls_base: 0,
        lr_sentinel: CALLBACK_RETURN_CODE_ADDR as u64,
    };
    let dispatch = Lv2Dispatch::CallbackSpawn {
        worker_init: init,
        worker_stack_base: stack.base(),
        worker_stack_size: stack.size(),
        worker_priority: 0,
        parent,
        parent_pending: PendingResponse::CallbackReturn {
            stage: CallbackReturnStage::Synthetic,
            args: [PENDING_SENTINEL; 8],
        },
        effects: vec![],
    };
    let pre: BTreeSet<UnitId> = rt.registry().ids().collect();
    rt.apply_callback_spawn(dispatch);
    let post: BTreeSet<UnitId> = rt.registry().ids().collect();
    *post
        .difference(&pre)
        .next()
        .expect("apply_callback_spawn registers a worker unit")
}

/// Happy path: callback body `addi r3, r3, 1; blr` increments the
/// r3 input by one, then the worker's blr lands on the trampoline,
/// the trampoline issues `sc 0x80000`, the runtime classifies and
/// dispatches `CallbackDispatchReturn`, and the parent unparks
/// with `args[0] = (input + 1)` in its r3.
#[test]
fn callback_dispatch_e2e_happy_path_addi_blr() {
    let mut rt = build_runtime_with_trampoline();
    // Parent: a real PpuExecutionUnit that will only run after
    // unpark; PC at PARENT_PC where we have a single nop-like
    // instruction. We won't actually step the parent past unpark
    // -- the test asserts on its drained syscall return.
    let parent = rt.registry_mut().register_with(PpuExecutionUnit::new);
    rt.lv2_host_mut().seed_primary_ppu_thread(
        parent,
        PpuThreadAttrs {
            entry: PARENT_PC,
            arg: 0,
            stack_base: PS3_PRIMARY_STACK_BASE as u32,
            stack_size: PS3_PRIMARY_STACK_SIZE as u32,
            priority: 1000,
            tls_base: 0,
        },
    );

    // Write the callback body: `addi r3, r3, 1; blr` at CALLBACK_BODY_PC.
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC, enc_addi(3, 3, 1));
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC + 4, enc_blr());

    // Park the parent; spawn the worker with r3 = 0x42.
    let input = 0x42u64;
    let worker = park_parent_on_callback(
        &mut rt,
        parent,
        CALLBACK_BODY_PC,
        [input, 0, 0, 0, 0, 0, 0, 0],
    );

    // Step until the parent unparks. Worker insns: addi (1) + blr
    // (1) + lis (1) + ori (1) + sc (1) = 5; the loop returns on
    // the iteration AFTER the wake, so consumed == 5 in steady
    // state. UNPARK_STEP_CEILING gives a small safety margin and
    // catches scheduling regressions that would push the count up.
    let consumed = step_until_parent_unparks(&mut rt, parent, worker, UNPARK_STEP_CEILING);
    assert!(
        consumed > 0 && consumed <= UNPARK_STEP_CEILING,
        "expected unpark within {UNPARK_STEP_CEILING} steps, took {consumed}",
    );

    // Parent's r3 must be input + 1 (the worker's r3 after addi).
    // If the runtime accidentally returned the parent's pending-
    // response args instead of the worker's captured r3, this
    // would be PENDING_SENTINEL rather than input + 1.
    let r3 = rt
        .registry_mut()
        .drain_syscall_return(parent)
        .expect("parent has a pending syscall return");
    assert_eq!(r3, input + 1, "parent r3 must be worker r3 = input + 1");

    // Worker lifecycle: dispatch_callback_return marks the worker's
    // PpuThread Finished, and the runtime's wake-side fix mirrors
    // that into the unit's status so the PPU execution loop stops
    // fetching past the trampoline `sc 0`. Without this, the
    // worker would stay Runnable, fetch from the OPD slot at
    // CALLBACK_RETURN_OPD_ADDR, and decode-fault on the OPD bytes.
    assert_eq!(
        rt.registry().effective_status(worker),
        Some(UnitStatus::Finished),
        "worker unit must transition to Finished after trampoline-return wake",
    );
}

/// Decision 1 proof: callback clobbers r11 before blr. The
/// trampoline's `lis r11, 8; ori r11, r11, 0` reloads r11 to
/// `CB_RETURN_SYSCALL = 0x80000` before `sc 0`, so the dispatch
/// classifies correctly even when the title's callback codegen
/// trashes r11 (which PPC64 ELFv1 marks volatile across `blr`).
#[test]
fn callback_dispatch_e2e_r11_clobber_proof() {
    let mut rt = build_runtime_with_trampoline();
    let parent = rt.registry_mut().register_with(PpuExecutionUnit::new);
    rt.lv2_host_mut().seed_primary_ppu_thread(
        parent,
        PpuThreadAttrs {
            entry: PARENT_PC,
            arg: 0,
            stack_base: PS3_PRIMARY_STACK_BASE as u32,
            stack_size: PS3_PRIMARY_STACK_SIZE as u32,
            priority: 1000,
            tls_base: 0,
        },
    );

    // Callback body that clobbers r11 with garbage before blr:
    //   li r11, -1   ; r11 = 0xFFFF_FFFF_FFFF_FFFF (sign-extended)
    //   addi r3, r3, 7
    //   blr
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC, enc_li(11, -1));
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC + 4, enc_addi(3, 3, 7));
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC + 8, enc_blr());

    let worker =
        park_parent_on_callback(&mut rt, parent, CALLBACK_BODY_PC, [10, 0, 0, 0, 0, 0, 0, 0]);

    // If r11 weren't reloaded, the trampoline's `sc 0` would issue
    // syscall 0xFFFF_FFFF_FFFF_FFFF, which the classifier routes
    // to Unsupported, the host stub returns CELL_OK, and the
    // parent never unparks (no callback_parents lookup). The
    // unpark itself is the proof the trampoline's reload was
    // structurally needed.
    //
    // Worker insns: li r11 (1) + addi (1) + blr (1) + lis (1) +
    // ori (1) + sc (1) = 6.
    let consumed = step_until_parent_unparks(&mut rt, parent, worker, UNPARK_STEP_CEILING);
    assert!(
        consumed > 0 && consumed <= UNPARK_STEP_CEILING,
        "r11-clobber path failed to unpark within {UNPARK_STEP_CEILING} steps \
         (took {consumed}); trampoline did not reload r11",
    );

    let r3 = rt
        .registry_mut()
        .drain_syscall_return(parent)
        .expect("parent has a pending syscall return");
    assert_eq!(
        r3,
        10 + 7,
        "parent r3 must be worker r3 even with r11 clobbered before blr"
    );
    assert_eq!(
        rt.registry().effective_status(worker),
        Some(UnitStatus::Finished),
    );
}

/// r11-clobber proof variant: the callback writes r11 = 1, a value
/// that DOES route to a real LV2 syscall (`sys_process_exit` lives
/// at syscall 22 but small numbers like 1 still route into the LV2
/// namespace via `SyscallNamespace::Lv2`). Without the trampoline's
/// reload, the worker's `sc 0` would issue syscall 1 -- a
/// real-syscall dispatch path that does NOT route through
/// `dispatch_callback_return`, so the parent would never unpark.
/// This catches the off-by-one class of classifier bug that the
/// `0xFFFF_FFFF_FFFF_FFFF` variant misses (any catch-all
/// Unsupported-router would reject either, but a classifier that
/// confused `Lv2`'s 0..0x10000 range with the `CellGovPrivate`
/// 0x80000+ range would only fail this variant).
#[test]
fn callback_dispatch_e2e_r11_clobber_with_real_syscall_value() {
    let mut rt = build_runtime_with_trampoline();
    let parent = rt.registry_mut().register_with(PpuExecutionUnit::new);
    rt.lv2_host_mut().seed_primary_ppu_thread(
        parent,
        PpuThreadAttrs {
            entry: PARENT_PC,
            arg: 0,
            stack_base: PS3_PRIMARY_STACK_BASE as u32,
            stack_size: PS3_PRIMARY_STACK_SIZE as u32,
            priority: 1000,
            tls_base: 0,
        },
    );

    // li r11, 1; addi r3, r3, 3; blr.
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC, enc_li(11, 1));
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC + 4, enc_addi(3, 3, 3));
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC + 8, enc_blr());

    let worker = park_parent_on_callback(
        &mut rt,
        parent,
        CALLBACK_BODY_PC,
        [100, 0, 0, 0, 0, 0, 0, 0],
    );

    let consumed = step_until_parent_unparks(&mut rt, parent, worker, UNPARK_STEP_CEILING);
    assert!(
        consumed > 0 && consumed <= UNPARK_STEP_CEILING,
        "r11=1 clobber path failed to unpark; trampoline reload may be off-by-one in classifier",
    );

    assert_eq!(
        rt.registry_mut().drain_syscall_return(parent),
        Some(100 + 3),
    );
    assert_eq!(
        rt.registry().effective_status(worker),
        Some(UnitStatus::Finished),
    );
}

/// Drive the runtime until either the parent unparks or `max_steps`
/// is hit, recording whether any step raised `YieldReason::Fault`
/// along the way. Returns `(fault_seen, steps_consumed)`.
fn drive_until_unpark_or_fault(
    rt: &mut Runtime,
    parent: UnitId,
    max_steps: usize,
) -> (bool, usize) {
    use cellgov_exec::YieldReason;
    let mut fault_seen = false;
    for steps in 0..max_steps {
        if rt.registry().effective_status(parent) != Some(UnitStatus::Blocked) {
            return (fault_seen, steps);
        }
        let s = rt.step().expect("step");
        if s.result.yield_reason == YieldReason::Fault {
            fault_seen = true;
        }
        rt.commit_step(&s.result, &s.effects).expect("commit");
    }
    (fault_seen, max_steps)
}

/// Worker callback body decode-faults before reaching `blr`: opcode
/// 0 (`raw=0x00000000`) is a reserved PPC encoding that surfaces as
/// a `YieldReason::Fault`. The runtime absorbs the fault, marks the
/// worker `Finished`, and unparks the parent with `CELL_EFAULT` in
/// r3 (sign-extended via the standard PPC64 ELFv1 i32 -> i64 -> u64
/// path: `0xFFFFFFFF8001000D`). The fault is recorded in the step
/// result so trace and diagnostics see it; the step-loop classifier
/// treats steps with `commit_outcome.callback_worker_fault_absorbed`
/// as `Continue` so the parent runs through its error-path code.
#[test]
fn callback_dispatch_e2e_decode_fault_in_body_propagates_cell_efault() {
    let mut rt = build_runtime_with_trampoline();
    let parent = rt.registry_mut().register_with(PpuExecutionUnit::new);
    rt.lv2_host_mut().seed_primary_ppu_thread(
        parent,
        PpuThreadAttrs {
            entry: PARENT_PC,
            arg: 0,
            stack_base: PS3_PRIMARY_STACK_BASE as u32,
            stack_size: PS3_PRIMARY_STACK_SIZE as u32,
            priority: 1000,
            tls_base: 0,
        },
    );
    // Callback body: a single illegal instruction (opcode 0).
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC, 0x0000_0000);

    let worker = park_parent_on_callback(&mut rt, parent, CALLBACK_BODY_PC, [0; 8]);

    let (fault_seen, _steps) = drive_until_unpark_or_fault(&mut rt, parent, UNPARK_STEP_CEILING);
    assert!(
        fault_seen,
        "callback body must surface a YieldReason::Fault"
    );
    assert_ne!(
        rt.registry().effective_status(parent),
        Some(UnitStatus::Blocked),
        "callback worker fault must unpark parent (propagation absorbed)",
    );
    let r3 = rt
        .registry_mut()
        .drain_syscall_return(parent)
        .expect("parent r3 set after absorbed fault");
    assert_eq!(
        r3,
        u64::from(CELL_EFAULT),
        "parent r3 must carry CELL_EFAULT after absorbed worker fault",
    );
    assert_eq!(
        rt.registry().effective_status(worker),
        Some(UnitStatus::Finished),
        "worker unit must transition to Finished after absorbed fault",
    );
}

/// Worker callback body hits an unmapped load: `lis r5, 0xFFFE;
/// lwz r4, 0(r5)` reads from `0xFFFE_0000`, between this fixture's
/// `main` region (`0..0x40_0000`) and the stack regions. The
/// runtime absorbs the fault the same way as the decode-fault case
/// and propagates `CELL_EFAULT` to the parent.
#[test]
fn callback_dispatch_e2e_unmapped_load_in_body_propagates_cell_efault() {
    let mut rt = build_runtime_with_trampoline();
    let parent = rt.registry_mut().register_with(PpuExecutionUnit::new);
    rt.lv2_host_mut().seed_primary_ppu_thread(
        parent,
        PpuThreadAttrs {
            entry: PARENT_PC,
            arg: 0,
            stack_base: PS3_PRIMARY_STACK_BASE as u32,
            stack_size: PS3_PRIMARY_STACK_SIZE as u32,
            priority: 1000,
            tls_base: 0,
        },
    );

    // lis r5, 0xFFFE     ; r5 = 0xFFFE_0000 (unmapped, sign-ext to u64)
    // lwz r4, 0(r5)      ; FAULT: load from 0xFFFE_0000
    let lis_r5_fffe: u32 = (15 << 26) | (5 << 21) | 0xFFFE;
    let lwz_r4_0_r5: u32 = (32 << 26) | (4 << 21) | (5 << 16);
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC, lis_r5_fffe);
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC + 4, lwz_r4_0_r5);

    let worker = park_parent_on_callback(&mut rt, parent, CALLBACK_BODY_PC, [0; 8]);

    let (fault_seen, _steps) = drive_until_unpark_or_fault(&mut rt, parent, UNPARK_STEP_CEILING);
    assert!(
        fault_seen,
        "unmapped load must surface a YieldReason::Fault"
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(parent),
        Some(u64::from(CELL_EFAULT)),
    );
    assert_eq!(
        rt.registry().effective_status(worker),
        Some(UnitStatus::Finished),
    );
}
