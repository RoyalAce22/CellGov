//! A 1 MiB-page `sys_memory_allocate` installs its process's 1 MiB-page
//! area in the caller's space, so the block it hands out is backed.

use super::*;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ps3_abi::lv2::memory::{page_size, VM_AREA_GRANULE};
use cellgov_ps3_abi::lv2::syscall::MEMORY_ALLOCATE;

fn allocate_1m(rt: &mut Runtime) {
    let mut syscall_args = [0u64; 9];
    syscall_args[0] = MEMORY_ALLOCATE;
    syscall_args[1] = 0x10_0000;
    syscall_args[2] = page_size::FLAG_1M;
    syscall_args[3] = 0x100;
    rt.registry_mut().register_with(|id| Lv2SyscallEmitterUnit {
        id,
        steps: Cell::new(0),
        syscall_args,
    });
    let s = rt.step().unwrap();
    rt.commit_unchecked(&s.result, &s.effects).unwrap();
}

#[test]
fn a_1m_page_block_is_backed_where_the_kernel_put_it() {
    let mem = GuestMemory::from_regions(vec![Region::new(0, 0x10000, "flat", PageSize::Page4K)])
        .expect("one region");
    let mut rt = Runtime::new(mem, Budget::new(1), 100);
    allocate_1m(&mut rt);
    let base = u32::from_be_bytes(
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(0x100), 4).unwrap())
            .expect("the out word")
            .try_into()
            .expect("4 bytes"),
    );
    assert_eq!(base, 0x5000_0000);
    let last = u64::from(base) + VM_AREA_GRANULE - 4;
    assert!(
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(last), 4).unwrap())
            .is_some(),
        "the whole 256 MiB area is backed"
    );
}
