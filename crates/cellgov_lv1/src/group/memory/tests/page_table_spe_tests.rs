//! `write_htab_entry` for a page in a region of a logical SPE.

use super::*;
use crate::{shadow_storage, AddressSpaceChange, LogicalSpe, SHADOW_STORAGE_BASE};
use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_L, PTE_N, PTE_PP};
use cellgov_ps3_abi::lv1::call::WRITE_HTAB_ENTRY;
use cellgov_ps3_abi::lv1::page_table::PTE_M;
use cellgov_ps3_abi::lv1::spe::{WRITE_HTAB_SHADOW_PAGE_SIZE_R3, WRITE_HTAB_SHADOW_WRITABLE_R3};
use cellgov_time::GuestTicks;

/// The vas id that the console capture `tests/micro/lv2_vas_boot_record`
/// reads from the 4.93 kernel's record.
const VAS_ID: u64 = 2;

/// The shadow page of the 4.93 kernel's boot SPE, from the capture
/// `lv2_logical_spe_object`.
const SHADOW: u64 = 0x0000_3000_0000_6000;

/// The area of the 4.93 kernel's boot SPE: the 512 KB region of counter
/// value 5.
const AREA: u64 = 0x0000_4c00_0028_0000;

/// A shadow page whose address is a multiple of 64 KB: the 4 KB region
/// of counter value 16.
const ALIGNED_SHADOW: u64 = 0x0000_3000_0001_0000;

/// The entry index, doubleword 0 and doubleword 1 that CellGov's boot of
/// the 4.93 kernel passes in its call
/// [OFW-4.93 lv2_kernel a:0x800000000005e4f0]. Doubleword 1 holds the
/// shadow page, PP = 0b11, N and M
/// [OFW-4.93 lv2_kernel a:0x800000000005e4d0-0x800000000005e4e8].
const KERNEL_INDEX: u64 = 0x790;
const KERNEL_D0: u64 = 0xD001;
const KERNEL_D1: u64 = SHADOW | PTE_M | PTE_N | PTE_PP;

fn request(index: u64, d0: u64, d1: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: WRITE_HTAB_ENTRY,
        inputs: [0, index, d0, d1, 0, 0, 0, 0],
        pc: 0x8000_0000_0005_e4f0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(63_164),
    }
}

/// The 4.93 kernel's address space, selected: a 256 KB page table with
/// page shifts 16 and 20.
fn spaces() -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
        vas_id: VAS_ID,
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
        mapping: None,
    }));
    spaces.apply(AddressSpaceChange::Selected(VAS_ID));
    spaces
}

/// The boot SPE, and an SPE whose shadow page is a multiple of 64 KB.
fn spes() -> LogicalSpeState {
    let mut spes = LogicalSpeState::default();
    let boot = LogicalSpe {
        id: 11,
        spe_type: 1,
        vas_id: VAS_ID,
        area: AREA,
        shadow: SHADOW,
        shadow_real: SHADOW_STORAGE_BASE,
    };
    spes.construct(boot);
    spes.construct(LogicalSpe {
        id: 12,
        area: 0x0000_4c00_0078_0000,
        shadow: ALIGNED_SHADOW,
        shadow_real: SHADOW_STORAGE_BASE + 0x1000,
        ..boot
    });
    spes
}

/// The page table storage and the shadow page storage.
fn storage() -> GuestMemory {
    GuestMemory::from_regions(vec![page_table_storage(), shadow_storage()]).expect("storage")
}

fn write(index: u64, d0: u64, d1: u64, storage: &GuestMemory) -> HypercallDisposition {
    write_entry(
        &request(index, d0, d1),
        Some("4.93"),
        &spaces(),
        &spes(),
        &MemoryState::default(),
        &DeviceBusState::default(),
        storage,
    )
}

fn refusal_what(d: &HypercallDisposition) -> Option<&'static str> {
    match d {
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput { what, .. }) => Some(what),
        _ => None,
    }
}

/// The answer that writes doubleword 0 `KERNEL_D0` and doubleword 1
/// `stored` at `KERNEL_INDEX`.
fn written(stored: u64) -> HypercallDisposition {
    let mut bytes = KERNEL_D0.to_be_bytes().to_vec();
    bytes.extend_from_slice(&stored.to_be_bytes());
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(entry_address(KERNEL_INDEX)), 16).expect("range"),
        bytes: WritePayload::new(bytes),
        source: UnitId::new(1),
        source_time: GuestTicks::new(63_164),
    });
    HypercallDisposition::Answered(answer)
}

#[test]
fn the_kernels_boot_entry_maps_the_shadow_page_at_its_storage_page() {
    assert_eq!(
        write(KERNEL_INDEX, KERNEL_D0, KERNEL_D1, &storage()),
        written(SHADOW_STORAGE_BASE | PTE_M | PTE_N | PTE_PP)
    );
}

/// lv1's translation checks the page's size before the read only flag
/// [OFW-4.93 lv1 a:0x2FF7C0-0x2FF7E0], and lv1's writer sets that flag
/// for PP = 0b11 alone [OFW-4.93 lv1 a:0x2D308C-0x2D30B0].
#[test]
fn shadow_page_entries_that_lv1s_translation_refuses_get_its_r3() {
    let cases = [
        (
            KERNEL_D0,
            KERNEL_D1 & !PTE_PP | 0b10,
            WRITE_HTAB_SHADOW_WRITABLE_R3,
        ),
        (
            KERNEL_D0,
            KERNEL_D1 & !PTE_PP | 0b01,
            WRITE_HTAB_SHADOW_WRITABLE_R3,
        ),
        (
            KERNEL_D0 | PTE_L,
            ALIGNED_SHADOW | PTE_M | PTE_PP,
            WRITE_HTAB_SHADOW_PAGE_SIZE_R3,
        ),
        (
            KERNEL_D0 | PTE_L,
            ALIGNED_SHADOW | PTE_M | 0b10,
            WRITE_HTAB_SHADOW_PAGE_SIZE_R3,
        ),
    ];
    assert_eq!(cases.len(), 4);
    for (d0, d1, r3) in cases {
        assert_eq!(
            write(KERNEL_INDEX, d0, d1, &storage()),
            lv1_answer(r3),
            "d0 {d0:#x} d1 {d1:#x}"
        );
    }
}

/// lv1's writer searches for a duplicate before it translates the page
/// [OFW-4.93 lv1 a:0x2D2F20-0x2D3038].
#[test]
fn a_writable_shadow_page_entry_that_duplicates_another_answers_duplicate_entry() {
    let mut storage = storage();
    let other = (KERNEL_INDEX & !7) + 2;
    let range = ByteRange::new(GuestAddr::new(entry_address(other)), 8).expect("range");
    storage
        .apply_commit(range, &KERNEL_D0.to_be_bytes())
        .expect("commit");
    assert_eq!(
        write(KERNEL_INDEX, KERNEL_D0, KERNEL_D1 & !PTE_PP, &storage),
        lv1_answer(WRITE_HTAB_DUPLICATE_R3)
    );
}

/// lv1 finds the region by the page's address alone
/// [OFW-4.93 lv1 a:0x2D2510] [OFW-4.93 lv1 a:0x2D2614-0x2D262C]. So a
/// 64 KB page that starts below the boot SPE's shadow page and covers it
/// is not a shadow page. CellGov holds no region for it.
#[test]
fn a_large_page_that_covers_a_shadow_page_from_below_is_refused() {
    let below = SHADOW & !0xFFFF;
    let d1 = below | PTE_M | PTE_PP;
    let d = write(KERNEL_INDEX, KERNEL_D0 | PTE_L, d1, &storage());
    assert_eq!(
        refusal_what(&d),
        Some("a valid entry whose page lies outside the partition's real mode region")
    );
}

#[test]
fn an_area_page_or_a_shadow_page_with_no_storage_is_refused() {
    assert_eq!(
        refusal_what(&write(KERNEL_INDEX, KERNEL_D0, AREA | PTE_PP, &storage())),
        Some("a valid entry whose page lies in a logical SPE's area")
    );
    let table_only = GuestMemory::from_regions(vec![page_table_storage()]).expect("storage");
    assert_eq!(
        refusal_what(&write(KERNEL_INDEX, KERNEL_D0, KERNEL_D1, &table_only)),
        Some("a valid entry whose page CellGov holds no storage for")
    );
}

/// A page outside every region of an SPE lies in the real mode region
/// when it ends at or below the region's size, `bi.pu.rm_size`.
#[test]
fn a_page_that_ends_at_the_real_mode_size_is_written_and_the_next_is_refused() {
    let rm_size = memory::real_mode_size("4.93").expect("4.93 real mode size");
    assert!(rm_size + 0x1000 <= PAGE_TABLE_STORAGE_BASE);
    let last = rm_size - 0x1000;
    let storage = GuestMemory::from_regions(vec![
        Region::new(last, 0x2000, "real mode end", PageSize::Page4K),
        page_table_storage(),
        shadow_storage(),
    ])
    .expect("storage");
    assert_eq!(
        write(KERNEL_INDEX, KERNEL_D0, last | 0b10, &storage),
        written(last | PTE_M | 0b10)
    );
    assert_eq!(
        refusal_what(&write(KERNEL_INDEX, KERNEL_D0, rm_size | 0b10, &storage)),
        Some("a valid entry whose page lies outside the partition's real mode region")
    );
}
