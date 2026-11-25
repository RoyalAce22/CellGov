//! The page table entry a search matched, and the R and C writes an
//! access makes to it.

use cellgov_ps3_abi::hw::ppu_mmu::{PTEG_BYTES, PTE_C, PTE_G, PTE_H, PTE_R, PTE_V};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_SF, SLB_VALID};

use super::*;
use crate::exec::{PpuFault, PpuInterrupt};
use crate::supervisor::{SlbEntry, SupervisorState};

/// The page table's origin: 2^11 groups of 128 bytes at 16 MB.
const HTAB: u64 = 0x0100_0000;
/// The EA every test translates: VSID 0x123, page index 0x45.
const EA: u64 = 0x1004_5678;
/// The primary group of [`EA`]: hash 0x123 XOR 0x45 = 0x166.
const PRIMARY: u64 = HTAB + 0x166 * 128;
/// The secondary group of [`EA`]: the complement of the primary hash,
/// !0x166 & 0x7FF = 0x699.
const SECONDARY: u64 = HTAB + 0x699 * 128;

/// A thread with MSR `msr` whose SLB maps [`EA`]'s segment to VSID 0x123
/// with key 0.
fn thread(msr: u64) -> SupervisorState {
    let mut s = SupervisorState::new(msr);
    s.slb[0] = SlbEntry {
        esid_v: (EA & !0x0FFF_FFFF) | SLB_VALID,
        vsid_flags: 0x123 << 12,
    };
    s.partition = Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).expect("partition");
    s
}

/// The primary group of [`EA`] with an entry for it in `slot`.
fn group(slot: usize, d1: u64) -> [u8; PTEG_BYTES] {
    let mut g = [0; PTEG_BYTES];
    let d0 = (0x123u64 << 12) | PTE_V;
    g[slot * 16..slot * 16 + 8].copy_from_slice(&d0.to_be_bytes());
    g[slot * 16 + 8..slot * 16 + 16].copy_from_slice(&d1.to_be_bytes());
    g
}

fn search(access: Access, msr: u64, d1: u64) -> (Result<Translation, PpuFault>, Option<EntryHit>) {
    let g = group(3, d1);
    translate_entry(EA, access, &thread(msr), |addr| {
        (addr == PRIMARY).then_some(g)
    })
}

const RELOCATE: u64 = MSR_SF | MSR_IR | MSR_DR;

/// [PPC-Book3 p:36 s:4.5.1] R is doubleword 1 bit 55 and C bit 56: bytes 14 and 15 of the entry.
#[test]
fn the_reference_write_sets_bit_0x01_of_byte_14_and_keeps_its_other_bits() {
    let hit = EntryHit {
        addr: PRIMARY + 48,
        d1: 0x0234_5EFE | 0b10,
    };
    assert_eq!(hit.reference_write(), Some((PRIMARY + 48 + 14, 0x5F)));
    let set = EntryHit {
        d1: hit.d1 | PTE_R,
        ..hit
    };
    assert_eq!(set.reference_write(), None);
}

#[test]
fn the_change_write_sets_bit_0x80_of_byte_15_and_keeps_its_other_bits() {
    let hit = EntryHit {
        addr: PRIMARY + 48,
        d1: 0x0234_5000 | PTE_G | 0b11,
    };
    assert_eq!(hit.change_write(), Some((PRIMARY + 48 + 15, 0x80 | 0x0B)));
    let set = EntryHit {
        d1: hit.d1 | PTE_C,
        ..hit
    };
    assert_eq!(set.change_write(), None);
}

#[test]
fn a_search_gives_the_address_and_doubleword_1_of_the_entry_it_matched() {
    let d1 = 0x0234_5000 | 0b10;
    let (t, hit) = search(Access::Load, RELOCATE, d1);
    assert!(t.is_ok(), "{t:?}");
    assert_eq!(
        hit,
        Some(EntryHit {
            addr: PRIMARY + 3 * 16,
            d1
        })
    );
}

/// [PPC-Book3 p:38 s:4.5.3] The secondary hash is the complement of the primary, and its PTEG address is formed the same way.
#[test]
fn a_match_in_the_secondary_group_gives_an_address_in_that_group() {
    let d1 = 0x0234_5000 | 0b10;
    let mut g = group(6, d1);
    let d0 = (0x123u64 << 12) | PTE_H | PTE_V;
    g[6 * 16..6 * 16 + 8].copy_from_slice(&d0.to_be_bytes());
    let (t, hit) = translate_entry(EA, Access::Load, &thread(RELOCATE), |addr| {
        (addr == SECONDARY).then_some(g)
    });
    assert!(t.is_ok(), "{t:?}");
    assert_eq!(
        hit,
        Some(EntryHit {
            addr: SECONDARY + 6 * 16,
            d1
        })
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] The PPE sets R for any page with a load or store request, regardless of any storage interrupt.
#[test]
fn a_refused_access_still_gives_the_entry_it_matched() {
    // PP 11 with key 0 is read only.
    let d1 = 0x0234_5000 | 0b11;
    let (t, hit) = search(Access::Store, RELOCATE, d1);
    assert_eq!(
        t,
        Err(PpuInterrupt::DataStorage {
            ea: EA,
            store: true,
            protection: true
        }
        .into())
    );
    assert_eq!(hit.map(|h| h.addr), Some(PRIMARY + 3 * 16));
    // A fetch from a guarded page is not permitted.
    // [PPC-Book3 p:68 s:5.5] SRR1 bit 35 is set for a fetch with MSR[IR] = 1 from no-execute or guarded storage.
    let (t, hit) = search(Access::Fetch, RELOCATE, 0x0234_5000 | PTE_G | 0b10);
    assert_eq!(
        t,
        Err(PpuInterrupt::InstructionStorage {
            no_execute: true,
            protection: false
        }
        .into())
    );
    assert_eq!(hit.map(|h| h.addr), Some(PRIMARY + 3 * 16));
}

#[test]
fn real_mode_and_a_search_with_no_match_give_no_entry() {
    let (t, hit) = search(Access::Load, MSR_SF, 0x0234_5000);
    assert!(t.is_ok(), "{t:?}");
    assert_eq!(hit, None);
    let (t, hit) = translate_entry(EA, Access::Load, &thread(RELOCATE), |_| None);
    assert_eq!(
        t,
        Err(PpuInterrupt::DataStorage {
            ea: EA,
            store: false,
            protection: false
        }
        .into())
    );
    assert_eq!(hit, None);
}
