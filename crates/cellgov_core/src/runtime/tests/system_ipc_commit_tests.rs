//! A committed batch reaches the system-IPC write witness under the
//! process of the unit that committed it.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_lv2::request::classify;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv2::ipc::SYSTEM_IPC_KEY_NAMESPACE;
use cellgov_ps3_abi::lv2::memory::page_size::FLAG_64K;
use cellgov_ps3_abi::lv2::syscall;
use cellgov_time::{GuestTicks, InstructionCost};

use super::super::spaces::AddressSpaceId;
use super::super::Runtime;
use super::tests::{build_spawn_ready, spawn_request};

const AREA: u64 = 0x1000_0000;
const OUT: u64 = 0x400;
const NS_KEY: u64 = SYSTEM_IPC_KEY_NAMESPACE | 0x10;

fn call(rt: &mut Runtime, unit: UnitId, number: u64, args: [u64; 4]) -> u32 {
    let [a, b, c, d] = args;
    rt.dispatch_lv2_request(classify(number, &[a, b, c, d, 0, 0, 0, 0]), unit);
    assert_eq!(
        rt.registry_mut().drain_syscall_return(unit),
        Some(0),
        "{number}"
    );
    let space = rt.spaces.space_of(unit);
    let bytes = rt
        .space_memory(space)
        .unwrap()
        .read(ByteRange::new(GuestAddr::new(OUT), 4).unwrap())
        .expect("the call wrote its word");
    u32::from_be_bytes(bytes.try_into().unwrap())
}

/// Commit one 4-byte store at `addr` as `unit`'s batch.
fn commit_store(rt: &mut Runtime, unit: UnitId, addr: u64) {
    rt.last_scheduled_unit = Some(unit);
    let result = ExecutionStepResult {
        yield_reason: YieldReason::BudgetExhausted,
        consumed_cost: InstructionCost::new(1),
        local_diagnostics: LocalDiagnostics::empty(),
        fault: None,
        syscall_args: None,
    };
    let store = Effect::shared_write(
        ByteRange::new(GuestAddr::new(addr), 4).unwrap(),
        WritePayload::from_slice(&[1, 2, 3, 4]),
        unit,
        GuestTicks::ZERO,
    );
    rt.commit_step(&result, &[store])
        .expect("the store commits");
}

#[test]
fn a_childs_store_into_its_namespace_mapping_counts_under_the_child() {
    let mut rt = build_spawn_ready();
    let parent = UnitId::new(0);
    rt.dispatch_lv2_request(spawn_request(), parent);
    let child = UnitId::new(1);
    assert_eq!(rt.spaces.space_of(child), AddressSpaceId::new(1));

    let base = call(
        &mut rt,
        child,
        syscall::MMAPPER_ALLOCATE_ADDRESS,
        [AREA, FLAG_64K, AREA, OUT],
    );
    let mem_id = call(
        &mut rt,
        child,
        syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
        [NS_KEY, 0x1_0000, FLAG_64K, OUT],
    );
    rt.dispatch_lv2_request(
        classify(
            syscall::MMAPPER_MAP_SHARED_MEMORY,
            &[u64::from(base), u64::from(mem_id), 0, 0, 0, 0, 0, 0],
        ),
        child,
    );
    assert_eq!(rt.registry_mut().drain_syscall_return(child), Some(0));

    commit_store(&mut rt, child, u64::from(base) + 0x10);

    let witness = &rt.lv2_host().observability().system_ipc_witness;
    assert_eq!(witness.shm_writes, 1);
}
