//! Cursor-get catch-up from the MMIO GET register and ref-address mirroring at commit time.

use cellgov_mem::{GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

use crate::Runtime;

fn make_rt_with_rsx_region() -> Runtime {
    let regions = vec![
        Region::new(0, 0x10000, "flat", PageSize::Page4K),
        Region::new(0xC000_0000, 0x1000, "rsx", PageSize::Page64K),
    ];
    let mem = GuestMemory::from_regions(regions).expect("non-overlapping");
    Runtime::new(mem, Budget::new(1), 100)
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "after mirror_rsx_cursor_to_mmio")]
fn assert_ref_addr_mirrors_cursor_panics_when_writeback_was_dropped() {
    let mut rt = make_rt_with_rsx_region();
    rt.rsx_cursor_mut().set_reference(0xFFFF_FFFF);
    rt.test_only_assert_ref_addr_mirrors_cursor();
}

#[test]
fn catch_up_advances_cursor_get_when_mmio_get_is_ahead() {
    use crate::rsx::control_register::GET_ADDR;
    let mut rt = make_rt_with_rsx_region();
    let range = cellgov_mem::ByteRange::new(GuestAddr::new(GET_ADDR as u64), 4).unwrap();
    rt.memory_mut()
        .apply_commit(range, &0x0000_1000u32.to_be_bytes())
        .unwrap();
    rt.rsx_cursor_mut().set_get(0x0000_098c);
    rt.test_only_catch_up_cursor_get_from_mmio();
    assert_eq!(rt.rsx_cursor().get(), 0x0000_1000);
}

#[test]
fn catch_up_leaves_cursor_alone_when_mmio_get_is_behind() {
    use crate::rsx::control_register::GET_ADDR;
    let mut rt = make_rt_with_rsx_region();
    let range = cellgov_mem::ByteRange::new(GuestAddr::new(GET_ADDR as u64), 4).unwrap();
    rt.memory_mut()
        .apply_commit(range, &0x0000_0500u32.to_be_bytes())
        .unwrap();
    rt.rsx_cursor_mut().set_get(0x0000_1000);
    rt.test_only_catch_up_cursor_get_from_mmio();
    assert_eq!(rt.rsx_cursor().get(), 0x0000_1000);
}

#[test]
fn catch_up_leaves_cursor_alone_when_mmio_get_equals_cursor() {
    use crate::rsx::control_register::GET_ADDR;
    let mut rt = make_rt_with_rsx_region();
    let range = cellgov_mem::ByteRange::new(GuestAddr::new(GET_ADDR as u64), 4).unwrap();
    rt.memory_mut()
        .apply_commit(range, &0x0000_1000u32.to_be_bytes())
        .unwrap();
    rt.rsx_cursor_mut().set_get(0x0000_1000);
    rt.test_only_catch_up_cursor_get_from_mmio();
    assert_eq!(rt.rsx_cursor().get(), 0x0000_1000);
}
