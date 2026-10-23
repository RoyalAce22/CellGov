//! `sys_rsx_context_allocate` maps its context window in the caller's
//! space when it commits: the control block with the host's window
//! access, the driver-info and reports blocks read-write.

use super::*;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_ps3_abi::lv2::memory::page_size;
use cellgov_ps3_abi::lv2::rsx::region;
use cellgov_ps3_abi::lv2::syscall::{MEMORY_ALLOCATE, SYS_RSX_CONTEXT_ALLOCATE};

fn syscall(rt: &mut Runtime, args: &[u64]) {
    let mut syscall_args = [0u64; 9];
    syscall_args[..args.len()].copy_from_slice(args);
    rt.registry_mut().register_with(|id| Lv2SyscallEmitterUnit {
        id,
        steps: Cell::new(0),
        syscall_args,
    });
    let mut s = rt.step().unwrap();
    rt.commit_step(&mut s).unwrap();
}

fn region_at(rt: &Runtime, addr: u64) -> (&'static str, RegionAccess) {
    let r = rt
        .memory()
        .containing_region(addr, 4)
        .expect("the address is mapped");
    (r.label(), r.access())
}

#[test]
fn a_context_allocate_maps_its_window_past_the_held_granule() {
    let mem = GuestMemory::from_regions(vec![Region::new(0, 0x10000, "flat", PageSize::Page4K)])
        .expect("one region");
    let mut rt = Runtime::new(mem, Budget::new(1), 100);
    rt.set_rsx_window_access(RegionAccess::ReservedZeroReadable);
    // The host area takes the lowest granule, as libgcm's does.
    syscall(
        &mut rt,
        &[MEMORY_ALLOCATE, 0x10_0000, page_size::FLAG_1M, 0x100],
    );
    assert!(
        rt.memory().containing_region(0x6010_0000, 4).is_none(),
        "nothing is mapped where the window opens before 670"
    );
    syscall(
        &mut rt,
        &[SYS_RSX_CONTEXT_ALLOCATE, 0x200, 0x208, 0x210, 0x218],
    );

    let control = u64::from_be_bytes(
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(0x208), 8).unwrap())
            .expect("the dma control out word")
            .try_into()
            .expect("8 bytes"),
    );
    assert_eq!(control, 0x6010_0000);
    assert_eq!(
        region_at(&rt, control + 0x40),
        (
            crate::rsx::CONTROL_REGION,
            RegionAccess::ReservedZeroReadable
        ),
        "the control block takes the host's window access"
    );
    let reports_end = control + u64::from(region::CONTEXT_RESERVATION) - 4;
    for addr in [control + u64::from(region::DRIVER_INFO_OFFSET), reports_end] {
        assert_eq!(
            region_at(&rt, addr),
            (crate::rsx::CONTEXT_REGION, RegionAccess::ReadWrite),
            "driver info and reports at 0x{addr:x}"
        );
    }
}
