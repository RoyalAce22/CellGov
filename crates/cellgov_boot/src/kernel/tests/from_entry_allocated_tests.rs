use cellgov_ps3_abi::hw::ppu_mmu::{PTE_C, PTE_R};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, SLB_VALID};
use cellgov_ps3_abi::lv1::call::{
    ALLOCATE_MEMORY, CONSTRUCT_VIRTUAL_ADDRESS_SPACE, SELECT_VIRTUAL_ADDRESS_SPACE,
    WRITE_HTAB_ENTRY,
};
use cellgov_ps3_abi::lv1::page_table::PTE_M;

use super::page_table::{
    hypercall, li, lis, mr, ori, oris, read_and_send, send_r8, vas_id, DATA_PAGE, ESID_EA,
    PP_READ_WRITE, SLOT, VSID,
};
use super::*;

const MFMSR_R8: u32 = 0x7C00_00A6 | (8 << 21);
const MTMSRD_R8: u32 = 0x7C00_0164 | (8 << 21);
const MTMSRD_R9: u32 = 0x7C00_0164 | (9 << 21);
const ISYNC: u32 = 0x4C00_012C;

/// The doubleword that the program stores and loads back.
const VALUE: u32 = 0x1234_5678;

/// Runs `access`, an access through r20, with data translation on.
fn translated(access: u32) -> [u32; 6] {
    [
        MFMSR_R8,
        ori(9, 8, MSR_DR as u32),
        MTMSRD_R9,
        ISYNC,
        access,
        MTMSRD_R8,
    ]
}

/// A supervisor program in real mode that:
/// 1. constructs and selects the 4.93 kernel's address space;
/// 2. allocates a 64 KB region in 64 KB pages;
/// 3. maps the segment, and writes an entry that maps the data page to
///    the region's first page;
/// 4. stores a doubleword in the data page with data translation on;
/// 5. reads the entry and sends its low halfword;
/// 6. loads the doubleword with data translation on and sends it;
/// 7. stops at hypercall 3, which CellGov does not answer.
fn program() -> Vec<u32> {
    let mut code = vec![li(3, 0), li(4, 2), lis(5, 0x1014), SLDI_R5_32];
    code.extend(hypercall(CONSTRUCT_VIRTUAL_ADDRESS_SPACE));
    code.push(li(3, vas_id()));
    code.extend(hypercall(SELECT_VIRTUAL_ADDRESS_SPACE));
    code.extend([lis(3, 1), li(4, 16), li(5, 0), li(6, 0)]);
    code.extend(hypercall(ALLOCATE_MEMORY));
    code.push(mr(21, 4));
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
        lis(10, VALUE >> 16),
        ori(10, 10, VALUE & 0xFFFF),
    ]);
    // std r10, 8(r20)
    code.extend(translated(0xF800_0000 | (10 << 21) | (20 << 16) | 8));
    code.push(ISYNC);
    code.extend(read_and_send());
    // ld r11, 8(r20)
    code.extend(translated(0xE800_0000 | (11 << 21) | (20 << 16) | 8));
    code.push(ISYNC);
    code.push(mr(8, 11));
    code.extend(send_r8());
    code.extend(hypercall(3));
    code
}

#[test]
fn a_page_of_an_allocated_region_holds_a_translated_store() {
    let run = run_kernel_from_entry(&kernel_elf(&program()), "4.93", 10_000).unwrap();
    assert!(
        matches!(
            run.stop,
            KernelRunStop::Refused(HypercallRefusal::Unmodelled { number: 3, .. })
        ),
        "the run stopped at {:?}",
        run.stop
    );
    let referenced = PTE_M | u64::from(PP_READ_WRITE) | PTE_R | PTE_C;
    let mut expected = (referenced << 48).to_be_bytes().to_vec();
    expected.extend_from_slice(&u64::from(VALUE).to_be_bytes());
    assert_eq!(run.console, expected);
}
