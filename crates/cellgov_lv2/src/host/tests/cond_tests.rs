//! LV2 condition-variable dispatch tests, split by lifecycle and wake operation.

use super::*;
use crate::host::test_support::{
    create_mutex_host, extract_write_u32, primary_attrs, seed_primary_ppu, FakeRuntime,
};
use crate::ppu_thread::PpuThreadId;
use crate::request::Lv2Request;

fn cond_fixture() -> (Lv2Host, FakeRuntime, UnitId) {
    cond_fixture_with(FakeRuntime::new(0x10000), UnitId::new(0))
}

fn cond_fixture_with(runtime: FakeRuntime, source: UnitId) -> (Lv2Host, FakeRuntime, UnitId) {
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, source);
    (host, runtime, source)
}

fn runtime_with_cond_attr(pshared: u32, ipc_key: u64, stamps: &[(u64, u32)]) -> FakeRuntime {
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let commit = |mem: &mut cellgov_mem::GuestMemory, addr: u64, bytes: &[u8]| {
        let range =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), bytes.len() as u64)
                .unwrap();
        mem.apply_commit(range, bytes).unwrap();
    };
    commit(&mut mem, 0x800, &pshared.to_be_bytes());
    commit(&mut mem, 0x808, &ipc_key.to_be_bytes());
    for &(addr, value) in stamps {
        commit(&mut mem, addr, &value.to_be_bytes());
    }
    FakeRuntime::with_memory(mem)
}

fn create_cond_with_attr(host: &mut Lv2Host, src: UnitId, rt: &FakeRuntime, mutex_id: u32) -> u32 {
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0x800,
        },
        src,
        rt,
    );
    match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

fn mark_seed_applied_at(host: &mut Lv2Host, base: u32) {
    let key = cellgov_ps3_abi::lv2::ipc::CELLSYSUTIL_SHM_IPC_KEY;
    host.derived.system_seeds_applied.insert(key);
    host.derived.system_seed_bases.insert(key, base);
}

#[path = "cond_lifecycle_tests.rs"]
mod lifecycle;

#[path = "cond_wait_tests.rs"]
mod wait;

#[path = "cond_signal_tests.rs"]
mod signal;

#[path = "cond_broadcast_tests.rs"]
mod broadcast;
