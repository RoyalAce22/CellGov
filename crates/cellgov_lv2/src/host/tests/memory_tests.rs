//! `sys_memory` dispatch tests: aligned sequential allocation, alloc-base override, user-memory-size reporting, and container id minting.

use super::*;
use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{extract_write_u32, FakeRuntime};
use crate::request::Lv2Request;

#[test]
fn memory_allocate_returns_aligned_sequential_addresses() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let source = UnitId::new(0);

    let addr1 = match host.dispatch(
        Lv2Request::MemoryAllocate {
            size: 0x10000,
            flags: 0x200,
            alloc_addr_ptr: 0x100,
        },
        source,
        &rt,
    ) {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let addr2 = match host.dispatch(
        Lv2Request::MemoryAllocate {
            size: 0x10000,
            flags: 0x200,
            alloc_addr_ptr: 0x104,
        },
        source,
        &rt,
    ) {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };

    assert_eq!(addr1 & 0xFFFF, 0, "addr1 not 64KB-aligned");
    assert_eq!(addr2 & 0xFFFF, 0, "addr2 not 64KB-aligned");
    assert!(
        addr2 >= addr1 + 0x10000,
        "allocations overlap: 0x{addr1:x} and 0x{addr2:x}"
    );
}

#[test]
fn set_mem_alloc_base_overrides_first_allocation_address() {
    let mut host = Lv2Host::new();
    host.set_mem_alloc_base(0x008A_0000);
    let rt = FakeRuntime::new(0x10000);
    let addr = match host.dispatch(
        Lv2Request::MemoryAllocate {
            size: 0x10000,
            flags: 0x200,
            alloc_addr_ptr: 0x100,
        },
        UnitId::new(0),
        &rt,
    ) {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    assert_eq!(
        addr, 0x008A_0000,
        "first allocation must use configured base"
    );
    assert_eq!(addr & 0xFFFF, 0, "alignment must be preserved");
}

#[test]
fn user_memory_size_available_falls_with_allocation() {
    let mut host = Lv2Host::new();
    host.set_mem_alloc_base(0x008A_0000);
    let rt = FakeRuntime::new(0x10000);
    let source = UnitId::new(0);
    let total = cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL;

    let query = |host: &mut Lv2Host| -> u32 {
        match host.dispatch(
            Lv2Request::MemoryGetUserMemorySize {
                mem_info_ptr: 0x200,
            },
            source,
            &rt,
        ) {
            Lv2Dispatch::Immediate { code: 0, effects } => match &effects[0] {
                cellgov_effects::Effect::SharedWriteIntent { bytes, .. } => {
                    let b = bytes.bytes();
                    u32::from_be_bytes([b[4], b[5], b[6], b[7]])
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            },
            other => panic!("expected Immediate(0), got {other:?}"),
        }
    };
    let alloc = |host: &mut Lv2Host, size: u64| {
        let d = host.dispatch(
            Lv2Request::MemoryAllocate {
                size,
                flags: 0x200,
                alloc_addr_ptr: 0x100,
            },
            source,
            &rt,
        );
        assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
    };

    assert_eq!(
        query(&mut host),
        total,
        "nothing consumed before first alloc"
    );

    alloc(&mut host, 0x100);
    assert_eq!(
        query(&mut host),
        total - 0x100,
        "available falls by the allocated size"
    );

    // The next allocation first re-aligns the cursor to the 64 KiB
    // granule, so the fall includes the alignment padding.
    alloc(&mut host, 0x100);
    assert_eq!(
        query(&mut host),
        total - 0x1_0000 - 0x100,
        "available falls by padding-to-granule plus size"
    );
}

#[test]
fn memory_free_is_noop_stub() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let result = host.dispatch(
        Lv2Request::MemoryFree { addr: 0x0001_0000 },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn memory_get_user_memory_size_writes_info_struct() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let source = UnitId::new(0);

    let result = host.dispatch(
        Lv2Request::MemoryGetUserMemorySize {
            mem_info_ptr: 0x200,
        },
        source,
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code: 0, effects } => {
            assert_eq!(effects.len(), 1, "expect one 8-byte write");
            match &effects[0] {
                cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } => {
                    assert_eq!(range.start().raw(), 0x200);
                    assert_eq!(range.length(), 8);
                    let b = bytes.bytes();
                    let total = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
                    let avail = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
                    assert_eq!(total, cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL);
                    assert_eq!(avail, cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL);
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            }
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn memory_container_create_writes_monotonic_id() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let source = UnitId::new(0);

    let id1 = match host.dispatch(
        Lv2Request::MemoryContainerCreate {
            cid_ptr: 0x100,
            size: 0x10_0000,
        },
        source,
        &rt,
    ) {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let id2 = match host.dispatch(
        Lv2Request::MemoryContainerCreate {
            cid_ptr: 0x104,
            size: 0x10_0000,
        },
        source,
        &rt,
    ) {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    assert_ne!(id1, 0);
    assert_ne!(id1, id2, "IDs must be monotonic across create calls");
}

/// Syscalls 324 and 341 are one kernel entry point, so the sub-granule
/// refusal cannot depend on which number the guest used.
#[test]
fn memory_container_create_sub_granule_size_is_enomem_on_both_syscall_numbers() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let source = UnitId::new(0);
    let enomem = Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());

    assert_eq!(
        host.dispatch(
            Lv2Request::MemoryContainerCreate {
                cid_ptr: 0x100,
                size: 0xF_FFFF,
            },
            source,
            &rt,
        ),
        enomem,
    );
    assert_eq!(
        host.dispatch(
            Lv2Request::Unsupported {
                number: 324,
                args: [0x100, 0xF_FFFF, 0, 0, 0, 0, 0, 0],
            },
            source,
            &rt,
        ),
        enomem,
    );
}

#[test]
fn memory_container_create_mints_no_id_for_a_refused_size() {
    let rt = FakeRuntime::new(0x10000);
    let source = UnitId::new(0);
    let accepted = |host: &mut Lv2Host| match host.dispatch(
        Lv2Request::MemoryContainerCreate {
            cid_ptr: 0x100,
            size: 0x10_0000,
        },
        source,
        &rt,
    ) {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };

    let mut after_refusal = Lv2Host::new();
    after_refusal.dispatch(
        Lv2Request::MemoryContainerCreate {
            cid_ptr: 0x100,
            size: 0,
        },
        source,
        &rt,
    );
    let mut untouched = Lv2Host::new();
    assert_eq!(
        accepted(&mut after_refusal),
        accepted(&mut untouched),
        "the refused create must not consume an id",
    );
}
