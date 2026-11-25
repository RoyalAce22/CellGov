use super::*;
use crate::group::spe::LogicalSpe;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_N, PTE_PP};
use cellgov_ps3_abi::lv1::page_table::PTE_M;

/// The area of the 4.93 kernel's boot SPE: the 512 KB region of counter
/// value 5, as the capture `lv2_logical_spe_object` gives it.
const AREA: u64 = 0x0000_4c00_0028_0000;

/// The shadow page of the 4.93 kernel's boot SPE, from the capture
/// `lv2_logical_spe_object`.
const SHADOW: u64 = 0x0000_3000_0000_6000;

/// Doubleword 1 of the 4.93 kernel's entry for the shadow page: PP =
/// 0b11, N and M. The kernel keeps these bits of its flags and adds the
/// page [OFW-4.93 lv2_kernel a:0x800000000005e4d0-0x800000000005e4e8].
const KERNEL_D1: u64 = SHADOW | PTE_M | PTE_N | PTE_PP;

fn boot_spes() -> LogicalSpeState {
    let mut spes = LogicalSpeState::default();
    spes.construct(LogicalSpe {
        id: 11,
        spe_type: 1,
        vas_id: 2,
        area: AREA,
        shadow: SHADOW,
        shadow_real: SHADOW_STORAGE_BASE,
    });
    spes
}

#[test]
fn the_kernels_read_only_entry_maps_the_shadow_page_at_its_real_page() {
    assert_eq!(
        region_page(&boot_spes(), SHADOW, 12, KERNEL_D1),
        Some(SpeRegionPage::Shadow(SHADOW_STORAGE_BASE))
    );
}

#[test]
fn a_shadow_page_entry_that_is_not_read_only_gets_lv1s_access_violation() {
    let cases = [0b00, 0b01, 0b10];
    assert_eq!(cases.len(), 3);
    for pp in cases {
        let d1 = (KERNEL_D1 & !PTE_PP) | pp;
        assert_eq!(
            region_page(&boot_spes(), SHADOW, 12, d1),
            Some(SpeRegionPage::Lv1(WRITE_HTAB_SHADOW_WRITABLE_R3)),
            "pp {pp:#b}"
        );
    }
    assert_eq!(WRITE_HTAB_SHADOW_WRITABLE_R3, 0x0000_0000_FFFF_FFFB);
}

/// lv1 checks the page's size before the read only flag.
#[test]
fn a_page_larger_than_the_shadow_page_gets_lv1s_marked_access_violation() {
    let cases = [(16, KERNEL_D1), (16, KERNEL_D1 & !PTE_PP), (20, KERNEL_D1)];
    assert_eq!(cases.len(), 3);
    for (shift, d1) in cases {
        assert_eq!(
            region_page(&boot_spes(), SHADOW, shift, d1),
            Some(SpeRegionPage::Lv1(WRITE_HTAB_SHADOW_PAGE_SIZE_R3)),
            "shift {shift} d1 {d1:#x}"
        );
    }
    assert_eq!(WRITE_HTAB_SHADOW_PAGE_SIZE_R3, 0x0000_0008_FFFF_FFFB);
}

#[test]
fn a_page_in_an_spes_area_is_refused() {
    let what = "a valid entry whose page lies in a logical SPE's area";
    let cases = [AREA, AREA + 0x4_0000, AREA + 0x6_0000, AREA + 0x7_F000];
    assert_eq!(cases.len(), 4);
    for address in cases {
        assert_eq!(
            region_page(&boot_spes(), address, 12, KERNEL_D1),
            Some(SpeRegionPage::Refused(what)),
            "{address:#x}"
        );
    }
}

#[test]
fn a_page_outside_every_spe_region_is_no_spe_page() {
    let cases = [
        SHADOW - 0x1000,
        SHADOW + 0x1000,
        AREA - 0x1000,
        AREA + 0x8_0000,
    ];
    assert_eq!(cases.len(), 4);
    for address in cases {
        assert_eq!(
            region_page(&boot_spes(), address, 12, KERNEL_D1),
            None,
            "{address:#x}"
        );
    }
    assert_eq!(
        region_page(&LogicalSpeState::default(), SHADOW, 12, KERNEL_D1),
        None
    );
}

#[test]
fn each_storage_page_has_its_own_real_address_and_the_storage_ends_after_the_last() {
    assert_eq!(shadow_page_real(0), Some(SHADOW_STORAGE_BASE));
    assert_eq!(
        shadow_page_real(SHADOW_STORAGE_PAGES - 1),
        Some(SHADOW_STORAGE_BASE + ((SHADOW_STORAGE_PAGES - 1) << 12))
    );
    assert_eq!(shadow_page_real(SHADOW_STORAGE_PAGES), None);
}

#[test]
fn each_page_of_the_shadow_storage_reads_zero_beside_the_page_table_storage() {
    let storage = GuestMemory::from_regions(vec![crate::page_table_storage(), shadow_storage()])
        .expect("disjoint regions");
    for index in 0..SHADOW_STORAGE_PAGES {
        let real = shadow_page_real(index).expect("a page");
        let range =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(real), 0x1000).expect("range");
        let bytes = storage.read(range).expect("held");
        assert!(bytes.iter().all(|&b| b == 0), "page {index}");
    }
}
