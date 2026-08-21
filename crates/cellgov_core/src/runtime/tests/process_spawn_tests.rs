use std::cell::Cell;

use cellgov_effects::Effect;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_lv2::request::classify;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize};
use cellgov_ps3_abi::cell_errors;
use cellgov_time::{Budget, InstructionCost};

use super::super::spaces::AddressSpaceId;
use super::super::Runtime;
use cellgov_event::UnitId;

/// Stays runnable, emits nothing; a stand-in for a live guest thread.
#[derive(Clone)]
struct Idle {
    id: UnitId,
    finished: Cell<bool>,
}

impl Idle {
    fn new(id: UnitId) -> Self {
        Self {
            id,
            finished: Cell::new(false),
        }
    }
}

impl ExecutionUnit for Idle {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.finished.get() {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

const CHILD_PATH: &[u8] = b"/test/child.self";
const PID_OUT: u64 = 0x20;
const BLOCK: u64 = 0x40;
const PATH_STR: u64 = 0x80;
const EXPECTED_PID: u32 = cellgov_ps3_abi::sys_process::BOOT_PROCESS_PID + 0x100;

/// Write the decoded marshal layout: `{u64 table_off=16, u64 0,
/// ptr table [path, 0], path string}`.
fn write_marshal_block(rt: &mut Runtime) {
    let write = |rt: &mut Runtime, addr: u64, bytes: &[u8]| {
        let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
        rt.memory_mut().apply_commit(range, bytes).unwrap();
    };
    write(rt, BLOCK, &16u64.to_be_bytes());
    write(rt, BLOCK + 16, &PATH_STR.to_be_bytes());
    write(rt, BLOCK + 24, &0u64.to_be_bytes());
    let mut path = CHILD_PATH.to_vec();
    path.push(0);
    write(rt, PATH_STR, &path);
}

fn build_spawn_ready() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(4), 100);
    rt.registry_mut().register_with(Idle::new);
    write_marshal_block(&mut rt);
    rt.lv2_host_mut()
        .content_store_mut()
        .register(CHILD_PATH, vec![0xEE; 16]);
    rt.set_ppu_factory(|id, _init| Box::new(Idle::new(id)));
    rt.set_process_spawn_loader(|elf_bytes, mem| {
        assert_eq!(
            elf_bytes,
            vec![0xEE; 16],
            "loader must see the stored bytes"
        );
        mem.install_region(0, 0x1000, "child", PageSize::Page64K)
            .map_err(
                |source| crate::runtime::types::ProcessSpawnLoadError::RegionInstall { source },
            )?;
        Ok(crate::runtime::types::SpawnedProcessImage {
            entry_code: 0x100,
            entry_toc: 0x200,
            stack_top: 0xF00,
            lr_sentinel: 0,
        })
    });
    rt
}

fn spawn_request() -> cellgov_lv2::Lv2Request {
    classify(
        cellgov_ps3_abi::syscall::PROCESS_SPAWN,
        &[PID_OUT, 1000, 0, BLOCK, 0x60, 0, 0, 0],
    )
}

#[test]
fn spawn_creates_child_space_unit_and_pid() {
    let mut rt = build_spawn_ready();
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));

    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
    let pid_bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new(PID_OUT), 4).unwrap())
        .unwrap()
        .to_vec();
    assert_eq!(
        u32::from_be_bytes(pid_bytes.try_into().unwrap()),
        EXPECTED_PID
    );

    let child = UnitId::new(1);
    assert_eq!(rt.unit_space(child), AddressSpaceId::new(1));
    assert_eq!(rt.lv2_host().process_of_unit(child), EXPECTED_PID);
    assert_eq!(
        rt.registry().effective_status(child),
        Some(UnitStatus::Runnable)
    );
    assert!(rt.space_memory(AddressSpaceId::new(1)).is_ok());
    assert_eq!(rt.lv2_host().process_exit_status(EXPECTED_PID), None);
}

#[test]
fn child_exit_finishes_only_the_child_process() {
    let mut rt = build_spawn_ready();
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    let child = UnitId::new(1);

    let exit = classify(
        cellgov_ps3_abi::syscall::PROCESS_EXIT,
        &[42, 0, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(exit, child);

    assert_eq!(
        rt.registry().effective_status(child),
        Some(UnitStatus::Finished)
    );
    assert_eq!(
        rt.registry().effective_status(UnitId::new(0)),
        Some(UnitStatus::Runnable),
        "parent must survive the child's exit",
    );
    assert_eq!(rt.lv2_host().process_exit_status(EXPECTED_PID), Some(42));

    // Liveness poll flips CELL_OK -> CELL_ESRCH across the exit.
    let status = classify(
        cellgov_ps3_abi::syscall::PROCESS_GET_STATUS,
        &[u64::from(EXPECTED_PID), 0, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(status, UnitId::new(0));
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(cell_errors::CELL_ESRCH.into())
    );
}

#[test]
fn get_status_reports_live_child_as_ok() {
    let mut rt = build_spawn_ready();
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    let status = classify(
        cellgov_ps3_abi::syscall::PROCESS_GET_STATUS,
        &[u64::from(EXPECTED_PID), 0, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(status, UnitId::new(0));
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
}

#[test]
fn spawn_unknown_path_returns_enoent() {
    let mut rt = build_spawn_ready();
    let mut bad = b"/test/missing.self".to_vec();
    bad.push(0);
    let range = ByteRange::new(GuestAddr::new(PATH_STR), bad.len() as u64).unwrap();
    rt.memory_mut().apply_commit(range, &bad).unwrap();

    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(cell_errors::CELL_ENOENT.into())
    );
    assert!(rt.space_memory(AddressSpaceId::new(1)).is_err());
}

#[test]
fn spawn_loader_failure_rolls_back_space_and_pid() {
    let mut rt = build_spawn_ready();
    rt.set_process_spawn_loader(|_bytes, _mem| {
        Err(crate::runtime::types::ProcessSpawnLoadError::ImageParse {
            detail: "refused".into(),
        })
    });

    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(cell_errors::CELL_EFAULT.into())
    );
    assert!(rt.space_memory(AddressSpaceId::new(1)).is_err());
    // The minted pid was rolled back: a status poll sees ESRCH.
    let status = classify(
        cellgov_ps3_abi::syscall::PROCESS_GET_STATUS,
        &[u64::from(EXPECTED_PID), 0, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(status, UnitId::new(0));
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(cell_errors::CELL_ESRCH.into())
    );
}

#[test]
fn spawn_with_max_space_id_in_use_fails_enomem_and_rolls_back() {
    let mut rt = build_spawn_ready();
    rt.create_address_space(AddressSpaceId::new(u32::MAX))
        .unwrap();

    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(cell_errors::CELL_ENOMEM.into())
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.process_spawn_space_ids_exhausted"),
        1
    );
    // No child unit was registered and the minted pid was rolled back.
    assert!(rt.registry().effective_status(UnitId::new(1)).is_none());
    let status = classify(
        cellgov_ps3_abi::syscall::PROCESS_GET_STATUS,
        &[u64::from(EXPECTED_PID), 0, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(status, UnitId::new(0));
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(cell_errors::CELL_ESRCH.into())
    );
}

#[test]
fn nested_spawn_writes_pid_into_the_childs_space() {
    let mut rt = build_spawn_ready();
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    let child = UnitId::new(1);
    let child_space = AddressSpaceId::new(1);

    // Stage the same marshal layout inside the child's space; the
    // spawn parameters must resolve through the caller's space.
    {
        let mem = rt.space_memory_mut(child_space).unwrap();
        let mut write = |addr: u64, bytes: &[u8]| {
            let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
            mem.apply_commit(range, bytes).unwrap();
        };
        write(BLOCK, &16u64.to_be_bytes());
        write(BLOCK + 16, &PATH_STR.to_be_bytes());
        write(BLOCK + 24, &0u64.to_be_bytes());
        let mut path = CHILD_PATH.to_vec();
        path.push(0);
        write(PATH_STR, &path);
    }

    rt.dispatch_lv2_request(spawn_request(), child);
    assert_eq!(rt.registry_mut().drain_syscall_return(child), Some(0));

    let grandchild_pid = EXPECTED_PID + 0x100;
    let grandchild = UnitId::new(2);
    assert_eq!(rt.unit_space(grandchild), AddressSpaceId::new(2));
    assert_eq!(rt.lv2_host().process_of_unit(grandchild), grandchild_pid);

    // The grandchild pid landed in the CHILD's space...
    let read_pid = |mem: &GuestMemory| {
        let bytes = mem
            .read(ByteRange::new(GuestAddr::new(PID_OUT), 4).unwrap())
            .unwrap()
            .to_vec();
        u32::from_be_bytes(bytes.try_into().unwrap())
    };
    assert_eq!(
        read_pid(rt.space_memory(child_space).unwrap()),
        grandchild_pid
    );
    // ...while the boot space still holds the first spawn's pid.
    assert_eq!(read_pid(rt.memory()), EXPECTED_PID);
}

#[test]
fn a_thread_created_by_a_child_process_inherits_its_space_and_pid() {
    let mut rt = build_spawn_ready();
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    let child = UnitId::new(1);
    let child_space = AddressSpaceId::new(1);

    // Stage ppu_thread_param_t {entry_opd_ptr, tls} and its 8-byte
    // OPD inside the CHILD's space; the dispatch reads both through
    // the caller's memory view.
    {
        let mem = rt.space_memory_mut(child_space).unwrap();
        let mut write = |addr: u64, bytes: &[u8]| {
            let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
            mem.apply_commit(range, bytes).unwrap();
        };
        write(0x200, &0x210u32.to_be_bytes()); // param.entry_opd_ptr
        write(0x204, &0u32.to_be_bytes()); // param.tls
        write(0x210, &0x300u32.to_be_bytes()); // OPD code
        write(0x214, &0x400u32.to_be_bytes()); // OPD toc
    }

    // id_ptr is clear of PID_OUT (0x20), which the earlier spawn
    // already wrote in the boot space.
    const TID_OUT: u64 = 0x30;
    let create = classify(
        cellgov_ps3_abi::syscall::PPU_THREAD_CREATE,
        &[TID_OUT, 0x200, 0, 0, 1000, 0x4000, 0, 0],
    );
    rt.dispatch_lv2_request(create, child);
    assert_eq!(rt.registry_mut().drain_syscall_return(child), Some(0));

    // The new thread runs in the creator's space and pid...
    let thread = UnitId::new(2);
    assert_eq!(rt.unit_space(thread), child_space);
    assert_eq!(rt.lv2_host().process_of_unit(thread), EXPECTED_PID);

    // ...and the tid writeback landed in the creator's space, not
    // the boot space.
    let read_u64 = |mem: &GuestMemory| {
        let bytes = mem
            .read(ByteRange::new(GuestAddr::new(TID_OUT), 8).unwrap())
            .unwrap()
            .to_vec();
        u64::from_be_bytes(bytes.try_into().unwrap())
    };
    assert_ne!(read_u64(rt.space_memory(child_space).unwrap()), 0);
    assert_eq!(read_u64(rt.memory()), 0);

    // The child's exit sweep now covers the thread it created.
    let exit = classify(
        cellgov_ps3_abi::syscall::PROCESS_EXIT,
        &[0, 0, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(exit, child);
    assert_eq!(
        rt.registry().effective_status(thread),
        Some(UnitStatus::Finished),
        "the child's thread must finish with the child's process",
    );
}

#[test]
fn a_child_process_out_pointer_effect_lands_in_its_own_space() {
    // sys_process_get_sdk_version answers through a SharedWriteIntent
    // on the dispatch's effects -- the apply_lv2_effects direct-commit
    // path. Called by a child-process unit, the write must land in
    // the CHILD's space, not corrupt the boot space.
    let mut rt = build_spawn_ready();
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    let child = UnitId::new(1);
    let child_space = AddressSpaceId::new(1);

    const VERSION_OUT: u64 = 0x500;
    let req = classify(
        cellgov_ps3_abi::syscall::PROCESS_GET_SDK_VERSION,
        &[0, VERSION_OUT, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(req, child);
    assert_eq!(rt.registry_mut().drain_syscall_return(child), Some(0));

    let read_u32 = |mem: &GuestMemory| {
        let bytes = mem
            .read(ByteRange::new(GuestAddr::new(VERSION_OUT), 4).unwrap())
            .unwrap()
            .to_vec();
        u32::from_be_bytes(bytes.try_into().unwrap())
    };
    let expected = rt.lv2_host().sdk_version();
    assert_ne!(
        expected, 0,
        "vacuity guard: the sentinel version is nonzero"
    );
    assert_eq!(
        read_u32(rt.space_memory(child_space).unwrap()),
        expected,
        "the version write must land in the caller's space",
    );
    assert_eq!(
        read_u32(rt.memory()),
        0,
        "the boot space must not receive a child process's out-pointer write",
    );
}

#[test]
fn a_join_inside_a_child_process_writes_status_into_its_own_space() {
    // The PpuThreadJoin status writeback goes through commit_bytes_at;
    // with the joiner in a child process it must resolve the joiner's
    // space, not the boot space.
    let mut rt = build_spawn_ready();
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    let child = UnitId::new(1);
    let child_space = AddressSpaceId::new(1);

    // Stage ppu_thread_param_t {entry_opd_ptr, tls} + OPD in the
    // child's space, as in the inheritance test above.
    {
        let mem = rt.space_memory_mut(child_space).unwrap();
        let mut write = |addr: u64, bytes: &[u8]| {
            let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
            mem.apply_commit(range, bytes).unwrap();
        };
        write(0x200, &0x210u32.to_be_bytes()); // param.entry_opd_ptr
        write(0x204, &0u32.to_be_bytes()); // param.tls
        write(0x210, &0x300u32.to_be_bytes()); // OPD code
        write(0x214, &0x400u32.to_be_bytes()); // OPD toc
    }
    const TID_OUT: u64 = 0x30;
    // Clear of everything staged at fixed addresses: PID_OUT (0x20),
    // TID_OUT (0x30), the boot-space marshal block (0x40..0xA0), and
    // the child-space param/OPD staging (0x200..0x218) -- so the
    // boot-space zero check below cannot pass or fail off pre-staged
    // bytes.
    const STATUS_OUT: u64 = 0x600;
    let create = classify(
        cellgov_ps3_abi::syscall::PPU_THREAD_CREATE,
        &[TID_OUT, 0x200, 0, 0, 1000, 0x4000, 0, 0],
    );
    rt.dispatch_lv2_request(create, child);
    assert_eq!(rt.registry_mut().drain_syscall_return(child), Some(0));
    let thread = UnitId::new(2);
    let tid = {
        let bytes = rt
            .space_memory(child_space)
            .unwrap()
            .read(ByteRange::new(GuestAddr::new(TID_OUT), 8).unwrap())
            .unwrap()
            .to_vec();
        u64::from_be_bytes(bytes.try_into().unwrap())
    };
    assert_ne!(tid, 0);

    // Child parks on the join, then the thread exits with 0x77.
    let join = classify(
        cellgov_ps3_abi::syscall::PPU_THREAD_JOIN,
        &[tid, STATUS_OUT, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(join, child);
    assert_eq!(
        rt.registry().effective_status(child),
        Some(UnitStatus::Blocked)
    );
    let exit = classify(
        cellgov_ps3_abi::syscall::PPU_THREAD_EXIT,
        &[0x77, 0, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(exit, thread);
    assert_eq!(rt.registry_mut().drain_syscall_return(child), Some(0));

    let read_u64 = |mem: &GuestMemory| {
        let bytes = mem
            .read(ByteRange::new(GuestAddr::new(STATUS_OUT), 8).unwrap())
            .unwrap()
            .to_vec();
        u64::from_be_bytes(bytes.try_into().unwrap())
    };
    assert_eq!(
        read_u64(rt.space_memory(child_space).unwrap()),
        0x77,
        "the join status must land in the joiner's space",
    );
    assert_eq!(
        read_u64(rt.memory()),
        0,
        "the boot space must not receive a child joiner's status writeback",
    );
}

#[test]
fn a_thread_create_without_a_ppu_factory_is_loud_and_creates_nothing() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(4), 100);
    rt.registry_mut().register_with(Idle::new);
    // Stage ppu_thread_param_t {entry_opd_ptr, tls} + OPD so the
    // dispatch layer's reads succeed and the runtime handler is the
    // arm that refuses.
    {
        let mem = rt.memory_mut();
        let mut write = |addr: u64, bytes: &[u8]| {
            let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
            mem.apply_commit(range, bytes).unwrap();
        };
        write(0x200, &0x210u32.to_be_bytes()); // param.entry_opd_ptr
        write(0x204, &0u32.to_be_bytes()); // param.tls
        write(0x210, &0x300u32.to_be_bytes()); // OPD code
        write(0x214, &0x400u32.to_be_bytes()); // OPD toc
    }
    let create = classify(
        cellgov_ps3_abi::syscall::PPU_THREAD_CREATE,
        &[0x30, 0x200, 0, 0, 1000, 0x4000, 0, 0],
    );
    rt.dispatch_lv2_request(create, UnitId::new(0));
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(cell_errors::CELL_E2BIG.into())
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.ppu_thread_create_factory_missing"),
        1,
        "the missing-factory refusal must be named, not a bare errno",
    );
    assert!(rt.registry().effective_status(UnitId::new(1)).is_none());
}

#[test]
fn boot_process_exit2_with_empty_argv_finishes_the_run() {
    let mut rt = build_spawn_ready();
    // sys_exit2_param with a NULL argv[0]: header 0x28 bytes then the
    // args-array pointer; point it at a zeroed slot.
    {
        let mem = rt.memory_mut();
        let mut write = |addr: u64, bytes: &[u8]| {
            let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
            mem.apply_commit(range, bytes).unwrap();
        };
        write(0x200 + 0x28, &0x300u64.to_be_bytes()); // param.args
        write(0x300, &0u64.to_be_bytes()); // argv terminator
    }
    let exit2 = classify(
        cellgov_ps3_abi::syscall::PROCESS_EXIT2,
        &[0, 0x200, 0x30, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(exit2, UnitId::new(0));
    assert_eq!(
        rt.registry().effective_status(UnitId::new(0)),
        Some(UnitStatus::Finished),
        "a boot-process _sys_process_exit2 with empty argv is a plain \
         process exit; the caller must not resume past it",
    );
}

#[test]
fn snapshot_after_spawn_restores_child_space_and_process_state() {
    let mut rt = build_spawn_ready();
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    let child = UnitId::new(1);
    let snap = rt.snapshot();
    let hashes_at_snap = (rt.sync_state_hash(), rt.committed_memory_hash());

    // Mutate past the snapshot: the child exits.
    let exit = classify(
        cellgov_ps3_abi::syscall::PROCESS_EXIT,
        &[9, 0, 0, 0, 0, 0, 0, 0],
    );
    rt.dispatch_lv2_request(exit, child);
    assert_eq!(rt.lv2_host().process_exit_status(EXPECTED_PID), Some(9));

    rt.restore_into(&snap);
    assert_eq!(
        (rt.sync_state_hash(), rt.committed_memory_hash()),
        hashes_at_snap
    );
    assert_eq!(rt.lv2_host().process_exit_status(EXPECTED_PID), None);
    assert_eq!(rt.unit_space(child), AddressSpaceId::new(1));
    assert_eq!(rt.lv2_host().process_of_unit(child), EXPECTED_PID);
    assert_eq!(
        rt.registry().effective_status(child),
        Some(UnitStatus::Runnable)
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "does not contain the caller")]
fn exit_child_for_an_unbound_pid_debug_asserts() {
    let mut rt = build_spawn_ready();
    rt.handle_process_exit_child(UnitId::new(0), 0xDEAD_BEEF, 1, vec![]);
}

#[cfg(not(debug_assertions))]
#[test]
fn exit_child_for_an_unbound_pid_is_loud_and_finishes_the_caller() {
    let mut rt = build_spawn_ready();
    rt.handle_process_exit_child(UnitId::new(0), 0xDEAD_BEEF, 1, vec![]);
    assert_eq!(
        rt.registry().effective_status(UnitId::new(0)),
        Some(UnitStatus::Finished)
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.process_exit_child_unbound_caller"),
        1
    );
}

#[test]
fn an_out_of_range_spawn_prio_is_witnessed_not_silently_clamped() {
    let mut rt = build_spawn_ready();
    let req = classify(
        cellgov_ps3_abi::syscall::PROCESS_SPAWN,
        &[PID_OUT, (-1i64) as u64, 0, BLOCK, 0x60, 0, 0, 0],
    );
    rt.dispatch_lv2_request(req, UnitId::new(0));
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0),
        "the spawn itself proceeds; only the clamp is witnessed",
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.process_spawn_prio_out_of_range"),
        1
    );
}

#[test]
fn spawn_then_exit_hash_stream_is_deterministic() {
    let run = || {
        let mut rt = build_spawn_ready();
        let mut hashes = Vec::new();
        rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
        hashes.push((rt.sync_state_hash(), rt.committed_memory_hash()));
        let exit = classify(
            cellgov_ps3_abi::syscall::PROCESS_EXIT,
            &[7, 0, 0, 0, 0, 0, 0, 0],
        );
        rt.dispatch_lv2_request(exit, UnitId::new(1));
        hashes.push((rt.sync_state_hash(), rt.committed_memory_hash()));
        hashes
    };
    assert_eq!(run(), run());
}

/// The exit purge unqueues the waiter host-side before any later
/// send can reach it.
#[test]
fn a_send_after_child_exit_buffers_instead_of_resurrecting_the_dead_waiter() {
    let mut rt = build_spawn_ready();
    rt.lv2_host_mut().seed_primary_ppu_thread(
        UnitId::new(0),
        cellgov_lv2::ppu_thread::PpuThreadAttrs {
            entry: 0x100,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x1000,
            priority: 1000,
            tls_base: 0,
        },
    );
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    let child = UnitId::new(1);

    // A second thread of the child process, parked on a queue that
    // the boot process can also reach (the LV2 object namespace is
    // host-global).
    let waiter = rt.registry_mut().register_with(Idle::new);
    rt.lv2_host_mut()
        .ppu_threads_mut()
        .create(
            waiter,
            cellgov_lv2::ppu_thread::PpuThreadAttrs {
                entry: 0x100,
                arg: 0,
                stack_base: 0xE00,
                stack_size: 0x100,
                priority: 1000,
                tls_base: 0,
            },
        )
        .unwrap();
    rt.lv2_host_mut().bind_unit_process(waiter, EXPECTED_PID);
    rt.assign_unit_space(waiter, AddressSpaceId::new(1))
        .unwrap();

    let read_u32 = |rt: &Runtime, addr: u64| {
        u32::from_be_bytes(
            rt.memory()
                .read(ByteRange::new(GuestAddr::new(addr), 4).unwrap())
                .unwrap()
                .to_vec()
                .try_into()
                .unwrap(),
        )
    };

    let q_id_ptr: u64 = 0x30;
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::EVENT_QUEUE_CREATE,
            &[q_id_ptr, 0, 0, 4, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
    let queue = u64::from(read_u32(&rt, q_id_ptr));

    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::EVENT_QUEUE_RECEIVE,
            &[queue, 0x50, 0, 0, 0, 0, 0, 0],
        ),
        waiter,
    );
    assert_eq!(
        rt.registry().effective_status(waiter),
        Some(UnitStatus::Blocked),
        "the child-process waiter must park on the empty queue",
    );

    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::PROCESS_EXIT,
            &[7, 0, 0, 0, 0, 0, 0, 0],
        ),
        child,
    );
    assert_eq!(
        rt.registry().effective_status(waiter),
        Some(UnitStatus::Finished)
    );
    assert_eq!(
        rt.lv2_host()
            .observability()
            .process_exit_waiter_purges
            .get("equeue"),
        Some(&1),
        "the exit must purge the parked waiter from the queue",
    );

    // Boot side: port -> connect -> send into the purged queue.
    let port_id_ptr: u64 = 0x34;
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::EVENT_PORT_CREATE,
            &[port_id_ptr, 1, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
    let port = u64::from(read_u32(&rt, port_id_ptr));
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::EVENT_PORT_CONNECT_LOCAL,
            &[port, queue, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::EVENT_PORT_SEND,
            &[port, 1, 2, 3, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );

    // The dead waiter stayed dead, silently: no wake reached it, so
    // the Finished guard in resolve_sync_wakes never had to fire.
    assert_eq!(
        rt.registry().effective_status(waiter),
        Some(UnitStatus::Finished)
    );
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.resolve_sync_wakes_waiter_finished"),
        0,
        "the purge must remove the waiter before any wake can target it",
    );

    // The event went to the buffer: the boot process receives it.
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::EVENT_QUEUE_RECEIVE,
            &[queue, 0x60, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0),
        "the sent event must be buffered for survivors, not lost",
    );
}

/// The canceller's own effects (the num_ptr count write) stay in the
/// canceller's space.
#[test]
fn event_flag_cancel_stores_the_pattern_in_the_waiters_space() {
    let mut rt = build_spawn_ready();
    rt.lv2_host_mut().seed_primary_ppu_thread(
        UnitId::new(0),
        cellgov_lv2::ppu_thread::PpuThreadAttrs {
            entry: 0x100,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x1000,
            priority: 1000,
            tls_base: 0,
        },
    );
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));

    let waiter = rt.registry_mut().register_with(Idle::new);
    rt.lv2_host_mut()
        .ppu_threads_mut()
        .create(
            waiter,
            cellgov_lv2::ppu_thread::PpuThreadAttrs {
                entry: 0x100,
                arg: 0,
                stack_base: 0xE00,
                stack_size: 0x100,
                priority: 1000,
                tls_base: 0,
            },
        )
        .unwrap();
    rt.lv2_host_mut().bind_unit_process(waiter, EXPECTED_PID);
    rt.assign_unit_space(waiter, AddressSpaceId::new(1))
        .unwrap();

    // sys_event_flag_attribute_t in boot memory: protocol@0,
    // type@20; init pattern 0xABCD.
    let attr_ptr: u64 = 0x300;
    let write_boot = |rt: &mut Runtime, addr: u64, bytes: &[u8]| {
        let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
        rt.memory_mut().apply_commit(range, bytes).unwrap();
    };
    write_boot(
        &mut rt,
        attr_ptr,
        &cellgov_ps3_abi::sys_sync::SYS_SYNC_FIFO.to_be_bytes(),
    );
    write_boot(
        &mut rt,
        attr_ptr + 20,
        &cellgov_ps3_abi::sys_sync::SYS_SYNC_WAITER_SINGLE.to_be_bytes(),
    );
    let flag_id_ptr: u64 = 0x38;
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::EVENT_FLAG_CREATE,
            &[flag_id_ptr, attr_ptr, 0xABCD, 0, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );
    let flag = u64::from(u32::from_be_bytes(
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(flag_id_ptr), 4).unwrap())
            .unwrap()
            .to_vec()
            .try_into()
            .unwrap(),
    ));

    // The child-space waiter parks on a pattern the flag never
    // matches (AND on a clear bit). The result pointer targets ITS
    // space, at an address no boot-side fixture data occupies (the
    // spawn marshal block spans 0x40..0x60).
    let result_ptr: u64 = 0x200;
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::EVENT_FLAG_WAIT,
            &[flag, 0x10000, 0x01, result_ptr, 0, 0, 0, 0],
        ),
        waiter,
    );
    assert_eq!(
        rt.registry().effective_status(waiter),
        Some(UnitStatus::Blocked)
    );

    // Boot cancels; count lands at num_ptr in BOOT's memory.
    let num_ptr: u64 = 0x58;
    rt.dispatch_lv2_request(
        classify(
            cellgov_ps3_abi::syscall::EVENT_FLAG_CANCEL,
            &[flag, num_ptr, 0, 0, 0, 0, 0, 0],
        ),
        UnitId::new(0),
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(0)
    );

    // The waiter woke with ECANCELED and the pattern in ITS space.
    assert_eq!(
        rt.registry().effective_status(waiter),
        Some(UnitStatus::Runnable)
    );
    assert_eq!(
        rt.registry_mut().drain_syscall_return(waiter),
        Some(cell_errors::CELL_ECANCELED.into())
    );
    let child_bytes = rt
        .space_memory(AddressSpaceId::new(1))
        .unwrap()
        .read(ByteRange::new(GuestAddr::new(result_ptr), 8).unwrap())
        .unwrap()
        .to_vec();
    assert_eq!(
        u64::from_be_bytes(child_bytes.try_into().unwrap()),
        0xABCD,
        "the captured pattern must land through the waiter's space",
    );
    let boot_bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new(result_ptr), 8).unwrap())
        .unwrap()
        .to_vec();
    assert_eq!(
        u64::from_be_bytes(boot_bytes.try_into().unwrap()),
        0,
        "boot memory at the same address must stay untouched",
    );
    let count = u32::from_be_bytes(
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(num_ptr), 4).unwrap())
            .unwrap()
            .to_vec()
            .try_into()
            .unwrap(),
    );
    assert_eq!(
        count, 1,
        "the canceller's count write stays in its own space"
    );
}

#[test]
fn a_child_process_thread_gets_a_stack_region_in_its_own_space() {
    let mut rt = build_spawn_ready();
    rt.dispatch_lv2_request(spawn_request(), UnitId::new(0));
    let child = UnitId::new(1);
    let child_space = AddressSpaceId::new(1);

    {
        let mem = rt.space_memory_mut(child_space).unwrap();
        let mut write = |addr: u64, bytes: &[u8]| {
            let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
            mem.apply_commit(range, bytes).unwrap();
        };
        write(0x200, &0x210u32.to_be_bytes()); // param.entry_opd_ptr
        write(0x204, &0u32.to_be_bytes()); // param.tls
        write(0x210, &0x300u32.to_be_bytes()); // OPD code
        write(0x214, &0x400u32.to_be_bytes()); // OPD toc
    }
    let create = classify(
        cellgov_ps3_abi::syscall::PPU_THREAD_CREATE,
        &[0x30, 0x200, 0, 0, 1000, 0x4000, 0, 0],
    );
    rt.dispatch_lv2_request(create, child);
    assert_eq!(rt.registry_mut().drain_syscall_return(child), Some(0));

    let thread = UnitId::new(2);
    let stack_base = rt
        .lv2_host()
        .ppu_thread_for_unit(thread)
        .expect("created thread has a record")
        .attrs
        .stack_base as u64;
    // The whole block is readable through the child's space...
    assert!(
        rt.space_memory(child_space)
            .unwrap()
            .read(ByteRange::new(GuestAddr::new(stack_base), 0x4000).unwrap())
            .is_some(),
        "the child's space must map the new thread's stack block",
    );
    // ...and the boot space never grew a region for it (its layout
    // is the 0x1000-byte test arena).
    assert!(
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(stack_base), 16).unwrap())
            .is_none(),
        "a child thread's stack must not be installed into the boot space",
    );
}

#[test]
fn a_refused_thread_create_returns_its_stack_block_to_the_arena() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(4), 100);
    rt.registry_mut().register_with(Idle::new);
    {
        let mem = rt.memory_mut();
        let mut write = |addr: u64, bytes: &[u8]| {
            let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
            mem.apply_commit(range, bytes).unwrap();
        };
        write(0x200, &0x210u32.to_be_bytes());
        write(0x204, &0u32.to_be_bytes());
        write(0x210, &0x300u32.to_be_bytes());
        write(0x214, &0x400u32.to_be_bytes());
    }
    let create = classify(
        cellgov_ps3_abi::syscall::PPU_THREAD_CREATE,
        &[0x30, 0x200, 0, 0, 1000, 0x4000, 0, 0],
    );
    // No factory installed: the runtime refuses with E2BIG.
    rt.dispatch_lv2_request(create, UnitId::new(0));
    assert_eq!(
        rt.registry_mut().drain_syscall_return(UnitId::new(0)),
        Some(cell_errors::CELL_E2BIG.into())
    );
    // The arena rewound: the next block starts at the arena base.
    let next = rt
        .lv2_host_mut()
        .allocate_child_stack(0x4000, 0x10)
        .unwrap();
    assert_eq!(
        next.base(),
        0xD010_0000,
        "the refused create must return its block to the arena",
    );
}
