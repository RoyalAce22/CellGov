//! Where a block goes once frees have left holes: past the cursor while
//! the user region has room there, then in the lowest hole; and what a
//! container block takes from the available user memory.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;

use super::*;
use crate::host::test_support::FakeRuntime;
use crate::request::Lv2Request;
use crate::Lv2Dispatch;

const MIB: u64 = 0x10_0000;

fn allocate(host: &mut Lv2Host, rt: &FakeRuntime) -> Result<u32, u64> {
    let request = Lv2Request::MemoryAllocate {
        size: MIB,
        flags: page_size::FLAG_64K,
        alloc_addr_ptr: 0x100,
    };
    match host.dispatch(request, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => match &effects[0] {
            cellgov_effects::Effect::SharedWriteIntent { bytes, .. } => Ok(u32::from_be_bytes(
                bytes.bytes()[..4].try_into().expect("a u32 write"),
            )),
            other => panic!("expected a write, got {other:?}"),
        },
        Lv2Dispatch::Immediate { code, .. } => Err(code),
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn free(host: &mut Lv2Host, rt: &FakeRuntime, addr: u32) -> Lv2Dispatch {
    host.dispatch(Lv2Request::MemoryFree { addr }, UnitId::new(0), rt)
}

#[test]
fn a_freed_base_is_reused_only_once_the_region_past_the_cursor_is_full() {
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    let first = allocate(&mut host, &rt).expect("first");
    let second = allocate(&mut host, &rt).expect("second");
    assert_eq!(free(&mut host, &rt, first), Lv2Dispatch::immediate(0));
    let third = allocate(&mut host, &rt).expect("third");
    assert_eq!(third, second + MIB as u32, "the cursor still has room");

    // The region holds a few hundred 1 MiB blocks; a bound turns an
    // allocator that never fills into a failure, not a hang.
    let mut bases = vec![second, third];
    let refusal = (0..1024)
        .find_map(|_| match allocate(&mut host, &rt) {
            Ok(base) => {
                bases.push(base);
                None
            }
            Err(code) => Some(code),
        })
        .expect("the user region fills within 1024 blocks");
    assert_eq!(refusal, u64::from(errno::CELL_ENOMEM));
    assert_eq!(
        bases.last(),
        Some(&first),
        "once the region is full past the cursor, the hole at the first base is used"
    );
    assert!(
        bases[..bases.len() - 1].iter().all(|&b| b > first),
        "no earlier block reused it"
    );

    let freed = bases[5];
    assert_eq!(free(&mut host, &rt, freed), Lv2Dispatch::immediate(0));
    assert_eq!(allocate(&mut host, &rt), Ok(freed));
}

fn available(host: &mut Lv2Host, rt: &FakeRuntime) -> u32 {
    let request = Lv2Request::MemoryGetUserMemorySize {
        mem_info_ptr: 0x200,
    };
    match host.dispatch(request, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => match &effects[0] {
            cellgov_effects::Effect::SharedWriteIntent { bytes, .. } => {
                u32::from_be_bytes(bytes.bytes()[4..8].try_into().expect("info"))
            }
            other => panic!("expected a write, got {other:?}"),
        },
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn a_block_from_a_container_takes_nothing_more_from_the_available_memory() {
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    let before = available(&mut host, &rt);
    let create = Lv2Request::MemoryContainerCreate {
        cid_ptr: 0x100,
        size: MIB,
    };
    let cid = match host.dispatch(create, UnitId::new(0), &rt) {
        Lv2Dispatch::Immediate { code: 0, effects } => {
            crate::host::test_support::extract_write_u32(&effects[0])
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let after_create = available(&mut host, &rt);
    assert_eq!(
        before - after_create,
        MIB as u32,
        "the container takes its size"
    );
    let from = Lv2Request::MemoryAllocateFromContainer {
        size: MIB,
        cid,
        flags: page_size::FLAG_1M,
        alloc_addr_ptr: 0x104,
    };
    assert!(matches!(
        host.dispatch(from, UnitId::new(0), &rt),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
    assert_eq!(
        available(&mut host, &rt),
        after_create,
        "the block comes out of the container's size"
    );
}
