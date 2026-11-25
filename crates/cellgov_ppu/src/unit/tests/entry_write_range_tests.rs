//! A unit keeps an R or C write only with the one-byte range it
//! publishes, and refuses an entry byte that has no such range.

use crate::exec::PpuFault;
use crate::mmu::EntryHit;
use crate::unit::translation::RefillStop;
use crate::PpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_R_BYTE, PTE_R_BYTE_MASK};

#[test]
fn an_r_write_to_the_last_byte_of_the_address_space_is_an_internal_fault() {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let hit = EntryHit {
        addr: u64::MAX - PTE_R_BYTE as u64,
        d1: 0,
    };
    let refused = unit.note_entry_writes(hit, false);
    assert!(matches!(
        refused,
        Err(RefillStop::Fault(PpuFault::Internal(
            "a page table entry byte at the end of the address space"
        )))
    ));
    assert!(unit.table_writes.is_empty());
}

#[test]
fn an_r_write_to_the_second_last_byte_keeps_its_one_byte_range() {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let hit = EntryHit {
        addr: u64::MAX - 1 - PTE_R_BYTE as u64,
        d1: 0,
    };
    assert!(matches!(unit.note_entry_writes(hit, false), Ok(true)));
    let range = ByteRange::new(GuestAddr::new(u64::MAX - 1), 1).unwrap();
    assert_eq!(unit.table_writes, [(range, PTE_R_BYTE_MASK)]);
}
