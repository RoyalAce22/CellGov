//! End-to-end PPU worker callback dispatch through the runtime trampoline.

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

/// Distinct from any worker-init register value in this file so an
/// arg-source confusion surfaces as a wrong r3 rather than a zero match.
const PENDING_SENTINEL: u64 = 0xDEAD_DEAD_DEAD_DEADu64;

/// Longest synthetic callback here is 6 worker steps; ceiling catches
/// scheduler regressions instead of hiding them under generous slack.
const UNPARK_STEP_CEILING: usize = 10;

const PARENT_PC: u64 = 0x10_8000;
const CALLBACK_BODY_PC: u64 = 0x10_4000;

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

fn enc_addi(rt: u32, ra: u32, simm: i16) -> u32 {
    (14 << 26) | (rt << 21) | (ra << 16) | (simm as u16 as u32)
}

fn enc_li(rt: u32, simm: i16) -> u32 {
    enc_addi(rt, 0, simm)
}

fn enc_blr() -> u32 {
    (19 << 26) | (20 << 21) | (16 << 1)
}

fn write_u32_be(mem: &mut GuestMemory, addr: u64, raw: u32) {
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(addr), 4).unwrap(),
        &raw.to_be_bytes(),
    )
    .unwrap();
}

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

#[test]
fn callback_dispatch_e2e_happy_path_addi_blr() {
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

    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC, enc_addi(3, 3, 1));
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC + 4, enc_blr());

    let input = 0x42u64;
    let worker = park_parent_on_callback(
        &mut rt,
        parent,
        CALLBACK_BODY_PC,
        [input, 0, 0, 0, 0, 0, 0, 0],
    );

    let consumed = step_until_parent_unparks(&mut rt, parent, worker, UNPARK_STEP_CEILING);
    assert!(
        consumed > 0 && consumed <= UNPARK_STEP_CEILING,
        "expected unpark within {UNPARK_STEP_CEILING} steps, took {consumed}",
    );

    let r3 = rt
        .registry_mut()
        .drain_syscall_return(parent)
        .expect("parent has a pending syscall return");
    assert_eq!(r3, input + 1, "parent r3 must be worker r3 = input + 1");

    assert_eq!(
        rt.registry().effective_status(worker),
        Some(UnitStatus::Finished),
        "worker unit must transition to Finished after trampoline-return wake",
    );
}

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

    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC, enc_li(11, -1));
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC + 4, enc_addi(3, 3, 7));
    write_u32_be(rt.memory_mut(), CALLBACK_BODY_PC + 8, enc_blr());

    let worker =
        park_parent_on_callback(&mut rt, parent, CALLBACK_BODY_PC, [10, 0, 0, 0, 0, 0, 0, 0]);

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
