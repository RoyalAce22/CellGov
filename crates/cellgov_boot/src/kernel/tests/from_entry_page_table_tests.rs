use cellgov_ps3_abi::hw::ppu_mmu::{PTE_C, PTE_R};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, SLB_VALID};
use cellgov_ps3_abi::lv1::address_space::game_os_boot_vas_id;
use cellgov_ps3_abi::lv1::call::{
    CONSTRUCT_VIRTUAL_ADDRESS_SPACE, LOGICAL_CONSOLE_SEND, READ_HTAB_ENTRIES,
    SELECT_VIRTUAL_ADDRESS_SPACE, WRITE_HTAB_ENTRY,
};
use cellgov_ps3_abi::lv1::page_table::PTE_M;

use super::*;

/// The segment that the program's SLB entry maps: EA 0xC000_0000.
const ESID_EA: u32 = 0xC000;
/// The segment's VSID.
const VSID: u32 = 0x123;
/// The data page: page 1 of the segment, at real address 0x2_0000.
const DATA_PAGE: u32 = 0x1000;
const DATA_RA_HIGH: u32 = 0x2;
/// PP = 2: loads and stores for both keys.
const PP_READ_WRITE: u32 = 0x2;
/// The data page's entry: slot 0 of its primary group, whose hash is the
/// VSID XOR the page index, in a 256 KB table of 2^11 groups.
const SLOT: u32 = ((VSID ^ 1) & 0x7FF) * 8;
/// The vas id that the 4.93 hypervisor gives the kernel's address space.
fn vas_id() -> u32 {
    let id = game_os_boot_vas_id("4.93").expect("the 4.93 boot vas id");
    u32::try_from(id).expect("the vas id fits a 32-bit word")
}

fn li(rd: u32, value: u32) -> u32 {
    0x3800_0000 | (rd << 21) | value
}

fn lis(rd: u32, value: u32) -> u32 {
    0x3C00_0000 | (rd << 21) | value
}

fn ori(ra: u32, rs: u32, value: u32) -> u32 {
    0x6000_0000 | (rs << 21) | (ra << 16) | value
}

fn oris(ra: u32, rs: u32, value: u32) -> u32 {
    0x6400_0000 | (rs << 21) | (ra << 16) | value
}

fn mr(ra: u32, rs: u32) -> u32 {
    0x7C00_0378 | (rs << 21) | (ra << 16) | (rs << 11)
}

fn hypercall(number: u64) -> [u32; 2] {
    [li(11, number as u32), SC_1]
}

/// Sends r8, all 8 bytes, to the logical console.
fn send_r8() -> Vec<u32> {
    let mut code = vec![mr(5, 8), li(3, 0), li(4, 8)];
    code.extend(hypercall(LOGICAL_CONSOLE_SEND));
    code
}

/// Reads the four entries from the data page's slot and sends r8, their
/// low halfwords, to the console.
fn read_and_send() -> Vec<u32> {
    let mut code = vec![li(3, 0), li(4, SLOT)];
    code.extend(hypercall(READ_HTAB_ENTRIES));
    code.extend(send_r8());
    code
}

/// A supervisor program in real mode that:
/// 1. constructs and selects the 4.93 kernel's address space;
/// 2. maps the segment and writes the data page's entry;
/// 3. reads the entry and sends its low halfword;
/// 4. loads and stores the data page with data translation on;
/// 5. reads the entry and sends its low halfword again;
/// 6. stops at hypercall 3, which CellGov does not answer.
fn program() -> Vec<u32> {
    let mut code = vec![li(3, 0), li(4, 2), lis(5, 0x1014), SLDI_R5_32];
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
        lis(6, DATA_RA_HIGH),
        ori(6, 6, PP_READ_WRITE),
    ]);
    code.extend(hypercall(WRITE_HTAB_ENTRY));
    code.extend(read_and_send());
    code.extend([
        li(20, DATA_PAGE),
        oris(20, 20, ESID_EA),
        0x7C00_00A6 | (8 << 21),
        ori(9, 8, MSR_DR as u32),
        0x7C00_0164 | (9 << 21),
        0x4C00_012C,
        0xE800_0000 | (10 << 21) | (20 << 16),
        0xF800_0000 | (10 << 21) | (20 << 16) | 8,
        0x7C00_0164 | (8 << 21),
        0x4C00_012C,
    ]);
    code.extend(read_and_send());
    code.extend(hypercall(3));
    code
}

#[test]
fn read_htab_entries_returns_the_r_and_c_bits_that_a_translated_load_and_store_set() {
    let run = run_kernel_from_entry(&kernel_elf(&program()), "4.93", 10_000).unwrap();
    assert!(
        matches!(
            run.stop,
            KernelRunStop::Refused(HypercallRefusal::Unmodelled { number: 3, .. })
        ),
        "the run stopped at {:?}",
        run.stop
    );
    let written = PTE_M | u64::from(PP_READ_WRITE);
    let referenced = written | PTE_R | PTE_C;
    let mut expected = (written << 48).to_be_bytes().to_vec();
    expected.extend_from_slice(&(referenced << 48).to_be_bytes());
    assert_eq!(run.console, expected);
}
