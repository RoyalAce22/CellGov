//! The 32-byte `sys_spu_thread_argument` block: every slot takes the
//! eight bytes at its own offset, and an unreadable block is EFAULT.

use super::*;
use crate::host::test_support::FakeRuntime;
use cellgov_mem::{GuestAddr, GuestMemory};

/// Four values no two of which share an eight-byte window, so a slot
/// decoded at the wrong stride cannot match.
const ARG_BLOCK: [u64; 4] = [
    0x1122_3344_5566_7788,
    0x99AA_BBCC_DDEE_FF00,
    0x0102_0304_0506_0708,
    0xF0E0_D0C0_B0A0_9080,
];

const ARG_PTR: u32 = 0x200;
const IMG_PTR: u32 = 0x300;
const MEM_SIZE: usize = 0x4000;

/// A host holding image 1 and an empty one-slot group, plus a runtime
/// whose memory carries the image record and [`ARG_BLOCK`].
fn host_with_group() -> (Lv2Host, FakeRuntime) {
    let mut host = Lv2Host::new();
    host.content_store_mut()
        .register(b"/spu.elf", vec![0xAA, 0xBB]);

    let mut mem = GuestMemory::new(MEM_SIZE);
    let path = b"/spu.elf\0";
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0x100), path.len() as u64).unwrap(),
        path,
    )
    .unwrap();
    // Kernel-shaped record: type KERNEL, image id 1 in entry_point.
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(IMG_PTR)), 8).unwrap(),
        &[0, 0, 0, 1, 0, 0, 0, 1],
    )
    .unwrap();
    let mut arg_bytes = [0u8; 32];
    for (slot, value) in ARG_BLOCK.iter().enumerate() {
        arg_bytes[slot * 8..slot * 8 + 8].copy_from_slice(&value.to_be_bytes());
    }
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(ARG_PTR)), 32).unwrap(),
        &arg_bytes,
    )
    .unwrap();

    let rt = FakeRuntime::with_memory(mem);
    host.dispatch(
        Lv2Request::SpuImageOpen {
            img_ptr: IMG_PTR,
            path_ptr: 0x100,
        },
        UnitId::new(0),
        &rt,
    );
    host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x400,
            num_threads: 1,
            priority: 0,
            attr_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    (host, rt)
}

fn initialize_slot_zero(arg_ptr: u32) -> Lv2Request {
    Lv2Request::SpuThreadInitialize {
        thread_ptr: 0x500,
        group_id: 1,
        thread_num: 0,
        img_ptr: IMG_PTR,
        attr_ptr: 0,
        arg_ptr,
    }
}

#[test]
fn every_argument_slot_takes_the_eight_bytes_at_its_own_offset() {
    let (mut host, rt) = host_with_group();
    host.dispatch(initialize_slot_zero(ARG_PTR), UnitId::new(0), &rt);
    let result = host.dispatch(
        Lv2Request::SpuThreadGroupStart { group_id: 1 },
        UnitId::new(0),
        &rt,
    );
    let inits = match result {
        Lv2Dispatch::RegisterSpu { inits, .. } => inits,
        other => panic!("expected RegisterSpu, got {other:?}"),
    };
    assert_eq!(inits[&0].args, ARG_BLOCK);
}

#[test]
fn a_zero_argument_pointer_leaves_every_slot_zero() {
    let (mut host, rt) = host_with_group();
    host.dispatch(initialize_slot_zero(0), UnitId::new(0), &rt);
    let result = host.dispatch(
        Lv2Request::SpuThreadGroupStart { group_id: 1 },
        UnitId::new(0),
        &rt,
    );
    let inits = match result {
        Lv2Dispatch::RegisterSpu { inits, .. } => inits,
        other => panic!("expected RegisterSpu, got {other:?}"),
    };
    assert_eq!(inits[&0].args, [0u64; 4]);
}

#[test]
fn an_argument_block_running_past_the_mapped_end_is_efault() {
    let (mut host, rt) = host_with_group();
    // 32 bytes from here run four bytes past the mapped end, so the
    // whole block is unreadable.
    let arg_ptr = (MEM_SIZE - 28) as u32;
    let result = host.dispatch(initialize_slot_zero(arg_ptr), UnitId::new(0), &rt);
    assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_EFAULT.into()));
}
