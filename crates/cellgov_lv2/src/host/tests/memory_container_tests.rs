//! `sys_memory_allocate_from_container` against the container ids
//! `sys_memory_container_create` minted.

use super::*;
use crate::host::test_support::FakeRuntime;
use crate::request::Lv2Request;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv2::memory::page_size;

const OUT: u32 = 0x2000;

fn src() -> UnitId {
    UnitId::new(0)
}

fn code_of(d: &Lv2Dispatch) -> u64 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn written_u32(d: &Lv2Dispatch) -> u32 {
    match d {
        Lv2Dispatch::Immediate { effects, .. } => {
            crate::host::test_support::extract_write_u32(&effects[0])
        }
        other => panic!("expected Immediate with a write, got {other:?}"),
    }
}

fn create_container(host: &mut Lv2Host, rt: &FakeRuntime) -> u32 {
    let d = host.dispatch(
        Lv2Request::MemoryContainerCreate {
            cid_ptr: OUT,
            size: 0x10_0000,
        },
        src(),
        rt,
    );
    assert_eq!(code_of(&d), 0);
    written_u32(&d)
}

fn allocate(host: &mut Lv2Host, rt: &FakeRuntime, size: u64, cid: u32, flags: u64) -> Lv2Dispatch {
    allocate_to(host, rt, size, cid, flags, OUT)
}

fn allocate_to(
    host: &mut Lv2Host,
    rt: &FakeRuntime,
    size: u64,
    cid: u32,
    flags: u64,
    alloc_addr_ptr: u32,
) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::MemoryAllocateFromContainer {
            size,
            cid,
            flags,
            alloc_addr_ptr,
        },
        src(),
        rt,
    )
}

#[test]
fn a_container_id_the_process_minted_allocates_at_the_page_size_named() {
    let rt = FakeRuntime::with_memory(GuestMemory::new(0x10000));
    let mut host = Lv2Host::new();
    let cid = create_container(&mut host, &rt);
    assert!(host.state.memory_containers.contains_by(&cid));

    let d = allocate(&mut host, &rt, 0x1_0000, cid, page_size::FLAG_64K);
    assert_eq!(code_of(&d), 0);
    let first = written_u32(&d);
    assert_eq!(first % page_size::GRANULE_64K, 0);

    let d = allocate(&mut host, &rt, 0x10_0000, cid, page_size::FLAG_1M);
    assert_eq!(code_of(&d), 0);
    let second = written_u32(&d);
    assert_eq!(second % page_size::GRANULE_1M, 0);
    assert!(
        second >= first + 0x1_0000,
        "the cursor moved past the first block"
    );

    let d = allocate(&mut host, &rt, 0x10_0000, cid, 0);
    assert_eq!(code_of(&d), 0, "flags 0 is the 1 MiB default");
}

#[test]
fn an_unknown_container_id_is_esrch_after_the_argument_gates() {
    let rt = FakeRuntime::with_memory(GuestMemory::new(0x10000));
    let mut host = Lv2Host::new();
    let bogus = 0x7fff_ffff;
    assert_eq!(
        code_of(&allocate(
            &mut host,
            &rt,
            0x1_0000,
            bogus,
            page_size::FLAG_64K
        )),
        u64::from(errno::CELL_ESRCH)
    );
    assert_eq!(
        code_of(&allocate(&mut host, &rt, 0, bogus, page_size::FLAG_64K)),
        u64::from(errno::CELL_EALIGN),
        "a zero size is refused before the id is looked up"
    );
    assert_eq!(
        code_of(&allocate(&mut host, &rt, 0x1_0000, bogus, 0x100)),
        u64::from(errno::CELL_EINVAL),
        "an unknown page-size flag is refused before the id is looked up"
    );
    assert_eq!(
        code_of(&allocate(
            &mut host,
            &rt,
            0x1_0000,
            bogus,
            page_size::FLAG_1M
        )),
        u64::from(errno::CELL_EALIGN),
        "64 KiB is not a multiple of the 1 MiB page"
    );
}

#[test]
fn a_null_out_pointer_is_efault_after_the_budget_gates_and_moves_no_cursor() {
    let rt = FakeRuntime::with_memory(GuestMemory::new(0x10000));
    let mut host = Lv2Host::new();
    let cid = create_container(&mut host, &rt);
    let cursor = host.state.mem_alloc_ptr;
    assert_eq!(
        code_of(&allocate_to(
            &mut host,
            &rt,
            0x1_0000,
            cid,
            page_size::FLAG_64K,
            0
        )),
        u64::from(errno::CELL_EFAULT)
    );
    assert_eq!(
        host.state.mem_alloc_ptr, cursor,
        "no address was handed out"
    );
    assert_eq!(
        code_of(&allocate_to(
            &mut host,
            &rt,
            0x1_0000,
            0x7fff_ffff,
            page_size::FLAG_64K,
            0
        )),
        u64::from(errno::CELL_ESRCH),
        "the id gate fires before the pointer gate"
    );
}

#[test]
fn an_exhausted_budget_is_enomem_and_leaves_the_cursor() {
    let rt = FakeRuntime::with_memory(GuestMemory::new(0x10000));
    let mut host = Lv2Host::new();
    let cid = create_container(&mut host, &rt);
    let cursor = host.state.mem_alloc_ptr;
    let total = u64::from(cellgov_ps3_abi::lv2::memory::USER_MEMORY_TOTAL);
    for size in [
        total + u64::from(page_size::GRANULE_1M),
        0x1_0000_0000,
        0xffff_ffff_fff0_0000,
    ] {
        assert_eq!(
            code_of(&allocate(&mut host, &rt, size, cid, 0)),
            u64::from(errno::CELL_ENOMEM),
            "size {size:#x}"
        );
        assert_eq!(host.state.mem_alloc_ptr, cursor, "size {size:#x}");
    }
    let d = allocate(&mut host, &rt, 0x10_0000, cid, 0);
    assert_eq!(code_of(&d), 0, "the refusals consumed nothing");
}

#[test]
fn the_container_set_moves_the_host_partial() {
    let rt = FakeRuntime::with_memory(GuestMemory::new(0x10000));
    let mut host = Lv2Host::new();
    let before = host.sync_partial();
    let cid = create_container(&mut host, &rt);
    assert_ne!(host.sync_partial(), before);
    assert_eq!(host.sync_partial(), host.sync_partial_from_scratch());
    let with = host.sync_partial();
    host.state.memory_containers.remove(cid);
    assert_ne!(
        host.sync_partial(),
        with,
        "removing the container moves it back"
    );
    host.state.memory_containers.insert(cid, ());
    assert_eq!(host.sync_partial(), with);
}
