use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, SLB_VALID};
use cellgov_ps3_abi::lv1::call::{
    CONSTRUCT_VIRTUAL_ADDRESS_SPACE, MAP_DEVICE_MMIO_REGION, OPEN_DEVICE,
    SELECT_VIRTUAL_ADDRESS_SPACE, WRITE_HTAB_ENTRY,
};
use cellgov_ps3_abi::lv1::device_bus::{mmio_region, MMIO_REAL_BASE};

use super::page_table::{
    hypercall, li, lis, mr, ori, oris, vas_id, DATA_PAGE, ESID_EA, PP_READ_WRITE, SLOT, VSID,
};
use super::*;
use crate::kernel::DeviceMmioAccess;

/// The bus address of the region in slot 2 of device 1 of bus 1.
const REGION: u32 = 0x381_0000;
/// The offset of the access in the region.
const OFFSET: u32 = 0x10;

/// A supervisor program in real mode that:
/// 1. opens device 1 of bus 1 and maps its 64 KB region at bus address
///    [`REGION`] with 4 KB pages;
/// 2. constructs and selects the 4.93 kernel's address space, and maps
///    the segment;
/// 3. writes the data page's entry with the mapping's logical partition
///    address;
/// 4. runs `access`, a load or a store of r10 at [`OFFSET`] in the data
///    page, with data translation on;
/// 5. stops at hypercall 3, which CellGov does not answer.
///
/// Returns the program and the index of `access` in it.
fn program(access: u32) -> (Vec<u32>, usize) {
    let mut code = vec![li(3, 1), li(4, 1), li(5, 0)];
    code.extend(hypercall(OPEN_DEVICE));
    code.extend([
        li(3, 1),
        li(4, 1),
        lis(5, REGION >> 16),
        lis(6, 1),
        li(7, 12),
    ]);
    code.extend(hypercall(MAP_DEVICE_MMIO_REGION));
    code.push(mr(21, 4));
    code.extend([li(3, 0), li(4, 2), lis(5, 0x1014), SLDI_R5_32]);
    code.extend(hypercall(CONSTRUCT_VIRTUAL_ADDRESS_SPACE));
    code.push(li(3, vas_id()));
    code.extend(hypercall(SELECT_VIRTUAL_ADDRESS_SPACE));
    // slbmte r13, r12: ESID 0xC, valid, entry 0; VSID 0x123.
    let slb_valid_high = (SLB_VALID >> 16) as u32;
    code.extend([
        li(12, 0),
        oris(12, 12, ESID_EA | slb_valid_high),
        lis(13, VSID >> 4),
        ori(13, 13, (VSID & 0xF) << 12),
        0x7C00_0324 | (13 << 21) | (12 << 11),
    ]);
    code.extend([
        li(3, 0),
        li(4, SLOT),
        lis(5, VSID >> 4),
        ori(5, 5, ((VSID & 0xF) << 12) | 1),
        ori(6, 21, PP_READ_WRITE),
    ]);
    code.extend(hypercall(WRITE_HTAB_ENTRY));
    code.extend([
        li(20, DATA_PAGE),
        oris(20, 20, ESID_EA),
        0x7C00_00A6 | (8 << 21),
        ori(9, 8, MSR_DR as u32),
        0x7C00_0164 | (9 << 21),
        0x4C00_012C,
    ]);
    let at = code.len();
    code.push(access | (10 << 21) | (20 << 16) | OFFSET);
    code.extend(hypercall(3));
    (code, at)
}

/// The stop of the run of [`program`] with `access`, and the access the
/// run should stop at.
fn run_and_expect(access: u32) -> (KernelRunStop, KernelRunStop) {
    let (code, at) = program(access);
    let run = run_kernel_from_entry(&kernel_elf(&code), "4.93", 10_000).unwrap();
    let region = *mmio_region("4.93", 1, 1, u64::from(REGION)).expect("the 4.93 region");
    let bus_address = region.address + u64::from(OFFSET);
    let expected = KernelRunStop::DeviceMmio(DeviceMmioAccess {
        thread: 0,
        pc: Some(ENTRY + 4 * at as u64),
        region,
        bus_address,
        real: MMIO_REAL_BASE + bus_address,
        ea: u64::from((ESID_EA << 16) | DATA_PAGE | OFFSET),
    });
    (run.stop, expected)
}

#[test]
fn a_load_through_a_page_that_write_htab_entry_mapped_to_device_mmio_stops_at_the_device() {
    // lwz
    let (stop, expected) = run_and_expect(32 << 26);
    assert_eq!(stop, expected);
}

#[test]
fn a_store_through_a_page_that_write_htab_entry_mapped_to_device_mmio_stops_at_the_device() {
    // stw
    let (stop, expected) = run_and_expect(36 << 26);
    assert_eq!(stop, expected);
}
