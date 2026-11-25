//! Translation against Book III's real addressing mode, SLB search and
//! page table search, with the PPE's rules on top.

use std::collections::BTreeMap;

use cellgov_ps3_abi::hw::ppu_mmu::{
    PPE_PTE_LP, PTEG_BYTES, PTE_C, PTE_G, PTE_H, PTE_I, PTE_L, PTE_N, PTE_V, SLB_KP, SLB_KS, SLB_L,
    SLB_N,
};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_PR, MSR_SF, PPE_SLB_LP, SLB_VALID};

use super::*;
use crate::exec::{PpuFault, PpuInterrupt};
use crate::supervisor::{SlbEntry, SupervisorState};

/// The page table's origin: 2^11 groups of 128 bytes (256 KB) at 16 MB.
const HTAB: u64 = 0x0100_0000;

/// The translation or the interrupt that [`translate_entry`] gives,
/// without the entry. A CellGov stop fails the test.
fn translate(
    ea: u64,
    access: Access,
    sup: &SupervisorState,
    pteg: impl Fn(u64) -> Option<[u8; PTEG_BYTES]>,
) -> Result<Translation, PpuInterrupt> {
    translate_entry(ea, access, sup, pteg)
        .0
        .map_err(|fault| match fault {
            PpuFault::Interrupt(interrupt) => interrupt,
            stop => panic!("translation stopped where an interrupt was due: {stop}"),
        })
}

/// A hashed page table: PTEG real address to its 128 bytes.
#[derive(Default)]
struct Table(BTreeMap<u64, [u8; PTEG_BYTES]>);

impl Table {
    fn put(&mut self, pteg: u64, slot: usize, d0: u64, d1: u64) {
        let group = self.0.entry(pteg).or_insert([0; PTEG_BYTES]);
        group[slot * 16..slot * 16 + 8].copy_from_slice(&d0.to_be_bytes());
        group[slot * 16 + 8..slot * 16 + 16].copy_from_slice(&d1.to_be_bytes());
    }

    fn read(&self) -> impl Fn(u64) -> Option<[u8; PTEG_BYTES]> + '_ {
        |addr| self.0.get(&addr).copied()
    }
}

/// Doubleword 0 for `ea` in segment `vsid`: VA bits 0:56 in bits 0:56,
/// and V.
fn d0(vsid: u64, ea: u64) -> u64 {
    let va = (u128::from(vsid) << 28) | u128::from(ea & 0x0FFF_FFFF);
    (((va >> 23) as u64) << 7) | PTE_V
}

/// A thread with relocation on for both sides, MSR\[PR\] = `pr`, and SLB
/// entry 0 mapping `ea`'s segment to `vsid` with `flags`.
fn thread(ea: u64, vsid: u64, flags: u64, pr: bool) -> SupervisorState {
    let msr = MSR_SF | MSR_IR | MSR_DR | if pr { MSR_PR } else { 0 };
    let mut s = SupervisorState::new(msr);
    s.slb[0] = SlbEntry {
        esid_v: (ea & !0x0FFF_FFFF) | SLB_VALID,
        vsid_flags: (vsid << 12) | flags,
    };
    s.partition = Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).expect("partition");
    s
}

#[test]
fn doubleword_0_for_a_small_page_is_the_vsid_above_the_offsets_five_high_bits() {
    // VA 0x123_0004_5678: AVPN is VA bits 0:56, so VSID 0x123 and the
    // offset's bits 0:4 (0) land in bits 0:56 of doubleword 0.
    assert_eq!(d0(0x123, 0x1004_5678), 0x0012_3001);
}

#[test]
fn real_mode_reaches_ea_bits_2_to_63_plus_the_offset_for_either_high_bit_pattern() {
    let mut s = SupervisorState::new(MSR_SF);
    s.partition = Partition::new(0, 1 << 28, 0x1000_0000, Partition::INITIAL.lb()).expect("p");
    let read = Table::default();
    let t = translate(0x8000_0000_0000_1234, Access::Load, &s, read.read()).expect("in limit");
    assert_eq!(
        t,
        Translation {
            ea: 0x8000_0000_0000_0000,
            ra: 0x1000_0000,
            len: 1 << 28,
            writable: true,
            inhibited: false,
            guarded: false,
            changed: true,
        }
    );
    assert_eq!(t.real(0x8000_0000_0000_1234), Some(0x1000_1234));
    let alias = translate(0x4000_0000_0000_1234, Access::Store, &s, read.read()).expect("alias");
    assert_eq!((alias.ea, alias.ra), (0x4000_0000_0000_0000, 0x1000_0000));
    assert_eq!(alias.real(0x4000_0000_0000_1234), Some(0x1000_1234));
    let fetch = translate(0x0FFF_FFFC, Access::Fetch, &s, read.read()).expect("fetch");
    assert_eq!(fetch.real(0x0FFF_FFFC), Some(0x1FFF_FFFC));
}

#[test]
fn real_mode_refuses_the_limit_and_above_with_the_protection_cause() {
    let mut s = SupervisorState::new(MSR_SF);
    s.partition = Partition::new(0, 1 << 28, 0, Partition::INITIAL.lb()).expect("p");
    let read = Table::default();
    let ea = 0x8000_0000_1000_0000;
    assert_eq!(
        translate(ea, Access::Store, &s, read.read()),
        Err(PpuInterrupt::DataStorage {
            ea,
            store: true,
            protection: true,
        })
    );
    // Bits 2:63 count: an EA with bit 2 set is past any limit below 2^61.
    assert_eq!(
        translate(0x2000_0000_0000_0000, Access::Load, &s, read.read()),
        Err(PpuInterrupt::DataStorage {
            ea: 0x2000_0000_0000_0000,
            store: false,
            protection: true,
        })
    );
    assert_eq!(
        translate(ea, Access::Fetch, &s, read.read()),
        Err(PpuInterrupt::InstructionStorage {
            no_execute: false,
            protection: true,
        })
    );
}

#[test]
fn each_side_follows_its_own_relocation_bit() {
    let ea = 0x1004_5678;
    let mut s = thread(ea, 0x123, 0, false);
    let read = Table::default();
    s.msr &= !MSR_IR;
    assert_eq!(
        translate(ea, Access::Fetch, &s, read.read()).map(|t| t.real(ea)),
        Ok(Some(ea)),
        "IR = 0 is real mode"
    );
    assert_eq!(
        translate(ea, Access::Load, &s, read.read()),
        Err(PpuInterrupt::DataStorage {
            ea,
            store: false,
            protection: false,
        }),
        "DR = 1 searches the empty page table"
    );
    s.msr = (s.msr | MSR_IR) & !MSR_DR;
    assert_eq!(
        translate(ea, Access::Load, &s, read.read()).map(|t| t.real(ea)),
        Ok(Some(ea)),
        "DR = 0 is real mode"
    );
    assert_eq!(
        translate(ea, Access::Fetch, &s, read.read()),
        Err(PpuInterrupt::InstructionStorage {
            no_execute: false,
            protection: false,
        })
    );
}

#[test]
fn an_ea_whose_segment_no_valid_entry_maps_takes_the_segment_interrupt() {
    let ea = 0x2000_0000;
    let mut s = thread(0x1000_0000, 0x123, 0, false);
    // The same ESID in an invalid entry does not match.
    s.slb[1] = SlbEntry {
        esid_v: ea,
        vsid_flags: 0x456 << 12,
    };
    let read = Table::default();
    assert_eq!(
        translate(ea, Access::Load, &s, read.read()),
        Err(PpuInterrupt::DataSegment { ea, store: false })
    );
    assert_eq!(
        translate(ea, Access::Store, &s, read.read()),
        Err(PpuInterrupt::DataSegment { ea, store: true })
    );
    assert_eq!(
        translate(ea, Access::Fetch, &s, read.read()),
        Err(PpuInterrupt::InstructionSegment)
    );
}

#[test]
fn a_small_page_in_its_primary_group_translates_to_its_rpn_and_offset() {
    let ea = 0x1004_5678;
    let s = thread(ea, 0x123, 0, false);
    let mut table = Table::default();
    // Hash = VSID 0x123 XOR page index 0x45 = 0x166; HTABSIZE 0 indexes
    // with its low 11 bits.
    table.put(HTAB + 0x166 * 128, 5, d0(0x123, ea), 0x0234_5000 | 0b10);
    let t = translate(ea, Access::Store, &s, table.read()).expect("primary hit");
    assert_eq!(
        t,
        Translation {
            ea: 0x1004_5000,
            ra: 0x0234_5000,
            len: 4096,
            writable: true,
            inhibited: false,
            guarded: false,
            changed: false,
        }
    );
    assert_eq!(t.real(ea), Some(0x0234_5678));
}

#[test]
fn an_entry_in_the_secondary_group_matches_only_with_h_set() {
    let ea = 0x1004_5678;
    let s = thread(ea, 0x123, 0, false);
    // The secondary hash is the primary's complement: !0x166 & 0x7FF.
    let secondary = HTAB + 0x699 * 128;
    let not_found = Err(PpuInterrupt::DataStorage {
        ea,
        store: false,
        protection: false,
    });
    let mut table = Table::default();
    table.put(secondary, 0, d0(0x123, ea), 0x0700_0000);
    assert_eq!(
        translate(ea, Access::Load, &s, table.read()),
        not_found,
        "H = 0 names the primary group"
    );
    table.put(secondary, 0, d0(0x123, ea) | PTE_H, 0x0700_0000);
    let t = translate(ea, Access::Load, &s, table.read()).expect("secondary hit");
    assert_eq!(t.real(ea), Some(0x0700_0678));
    // An H = 1 entry in the primary group does not match.
    let mut wrong = Table::default();
    wrong.put(HTAB + 0x166 * 128, 0, d0(0x123, ea) | PTE_H, 0x0700_0000);
    assert_eq!(translate(ea, Access::Load, &s, wrong.read()), not_found);
}

/// [CBE-Handbook p:87 s:4.2.6.4] The PPE searches PTEs 0, 2, 4 and 6 of a group before PTEs 1, 3, 5 and 7.
#[test]
fn of_two_matching_entries_in_a_group_the_even_slot_is_searched_before_the_odd() {
    let ea = 0x1004_5678;
    let s = thread(ea, 0x123, 0, false);
    let pteg = HTAB + 0x166 * 128;
    // The two entries differ only in C, a bit that matching entries may
    // differ in.
    for (c1, c2) in [(PTE_C, 0), (0, PTE_C)] {
        let mut table = Table::default();
        table.put(pteg, 1, d0(0x123, ea), 0x0234_5000 | c1);
        table.put(pteg, 2, d0(0x123, ea), 0x0234_5000 | c2);
        let t = translate(ea, Access::Load, &s, table.read()).expect("hit");
        assert_eq!(t.changed, c2 != 0, "slot 2 gives C, not slot 1");
    }
}

#[test]
fn a_mismatched_avpn_invalid_entry_or_l_bit_is_no_translation() {
    let ea = 0x1004_5678;
    let s = thread(ea, 0x123, 0, false);
    let pteg = HTAB + 0x166 * 128;
    let not_found = Err(PpuInterrupt::DataStorage {
        ea,
        store: false,
        protection: false,
    });
    for d0 in [
        d0(0x124, ea),
        d0(0x123, ea) & !PTE_V,
        d0(0x123, ea) | PTE_L,
        // VA bit 52 is offset bit 0, compared for a 4 KB page.
        d0(0x123, ea) | (1 << 11),
        // VA bit 56, the last AVPN bit: q = 5 for a 4 KB page.
        d0(0x123, ea) | (1 << 7),
    ] {
        let mut table = Table::default();
        table.put(pteg, 0, d0, 0x0234_5000);
        assert_eq!(
            translate(ea, Access::Load, &s, table.read()),
            not_found,
            "{d0:#x}"
        );
    }
    // PTE bits 0:14 are reserved and unimplemented on the PPE: a match
    // ignores them.
    let mut table = Table::default();
    table.put(pteg, 0, d0(0x123, ea) | (0x7FFF << 49), 0x0234_5000);
    assert_eq!(
        translate(ea, Access::Load, &s, table.read()).map(|t| t.real(ea)),
        Ok(Some(0x0234_5678))
    );
}

/// [CBE-Handbook p:86 s:4.2.6.2] Table 4-2: PTE bits 15:56 are the AVPN.
/// [CBE-Handbook p:84 s:4.2.5.2] The SLB keeps VSID bits 15:51.
#[test]
fn vsid_bit_15_takes_part_in_the_avpn_match() {
    // VSID bit 15 of 0:51 is 1 << 36: above the 11 hash bits HTABSIZE 0
    // takes, so both VSIDs below hash to group 0x166.
    let ea = 0x1004_5678;
    let vsid = (1 << 36) | 0x123;
    let s = thread(ea, vsid, 0, false);
    let pteg = HTAB + 0x166 * 128;
    let mut table = Table::default();
    table.put(pteg, 0, d0(vsid, ea), 0x0234_5000);
    assert_eq!(
        translate(ea, Access::Load, &s, table.read()).map(|t| t.ra),
        Ok(0x0234_5000)
    );
    // The entry for VSID 0x123 differs only in AVPN bit 15.
    let mut table = Table::default();
    table.put(pteg, 0, d0(0x123, ea), 0x0234_5000);
    assert_eq!(d0(vsid, ea) ^ d0(0x123, ea), 1 << 48);
    assert_eq!(
        translate(ea, Access::Load, &s, table.read()),
        Err(PpuInterrupt::DataStorage {
            ea,
            store: false,
            protection: false,
        })
    );
}

/// [CBE-Handbook p:79 s:4.2.1] Table 4-1: the PPE does not implement real address bits 2:21 and treats them as 0.
#[test]
fn a_page_drops_the_rpn_bits_past_the_ppes_42_bit_real_address() {
    let ea = 0x1004_5678;
    let s = thread(ea, 0x123, 0, false);
    let mut table = Table::default();
    let d1 = (1 << 61) | (1 << 42) | 0x0234_5000 | 0b10;
    table.put(HTAB + 0x166 * 128, 0, d0(0x123, ea), d1);
    let t = translate(ea, Access::Load, &s, table.read()).expect("hit");
    assert_eq!((t.ra, t.real(ea)), (0x0234_5000, Some(0x0234_5678)));
}

#[test]
fn the_pp_bits_and_the_key_give_figure_30s_authority() {
    // (key, PP) -> (load permitted, store permitted).
    let figure = [
        ((false, 0b00), (true, true)),
        ((false, 0b01), (true, true)),
        ((false, 0b10), (true, true)),
        ((false, 0b11), (true, false)),
        ((true, 0b00), (false, false)),
        ((true, 0b01), (true, false)),
        ((true, 0b10), (true, true)),
        ((true, 0b11), (true, false)),
    ];
    let ea = 0x1004_5678;
    for ((key, pp), (load, store)) in figure {
        // The key is Kp in problem state and Ks otherwise; set the other
        // key opposite, so the wrong pick shows.
        for pr in [false, true] {
            let (ks, kp) = if pr { (!key, key) } else { (key, !key) };
            let flags = if ks { SLB_KS } else { 0 } | if kp { SLB_KP } else { 0 };
            let s = thread(ea, 0x123, flags, pr);
            let mut table = Table::default();
            table.put(HTAB + 0x166 * 128, 0, d0(0x123, ea), 0x0234_5000 | pp);
            let got_load = translate(ea, Access::Load, &s, table.read());
            let got_store = translate(ea, Access::Store, &s, table.read());
            let case = format!("key {key} pp {pp:02b} pr {pr}");
            assert_eq!(got_load.is_ok(), load, "{case}: load");
            assert_eq!(got_store.is_ok(), store, "{case}: store");
            if let Ok(t) = got_load {
                assert_eq!(t.writable, store, "{case}: writable");
            }
            if !store {
                assert_eq!(
                    got_store,
                    Err(PpuInterrupt::DataStorage {
                        ea,
                        store: true,
                        protection: true,
                    }),
                    "{case}"
                );
            }
            let fetch = translate(ea, Access::Fetch, &s, table.read());
            assert_eq!(fetch.is_ok(), load, "{case}: a fetch needs any access");
            if !load {
                // [PPC-Book3 p:68 s:5.5.5] A fetch the storage protection mechanism refuses sets SRR1 bit 36.
                assert_eq!(
                    fetch,
                    Err(PpuInterrupt::InstructionStorage {
                        no_execute: false,
                        protection: true,
                    }),
                    "{case}: fetch"
                );
            }
        }
    }
}

#[test]
fn a_fetch_from_a_no_execute_segment_or_page_or_a_guarded_page_is_refused() {
    let ea = 0x1004_5678;
    let no_execute = Err(PpuInterrupt::InstructionStorage {
        no_execute: true,
        protection: false,
    });
    for (slb_flags, d1_flags) in [(SLB_N, 0), (0, PTE_N), (0, PTE_G)] {
        let s = thread(ea, 0x123, slb_flags, false);
        let mut table = Table::default();
        table.put(HTAB + 0x166 * 128, 0, d0(0x123, ea), 0x0234_5000 | d1_flags);
        assert_eq!(translate(ea, Access::Fetch, &s, table.read()), no_execute);
        assert_eq!(
            translate(ea, Access::Load, &s, table.read()).map(|t| t.real(ea)),
            Ok(Some(0x0234_5678)),
            "N and G do not refuse a load"
        );
    }
}

#[test]
fn a_page_with_i_set_is_caching_inhibited() {
    let ea = 0x1004_5678;
    let s = thread(ea, 0x123, 0, false);
    let mut table = Table::default();
    table.put(HTAB + 0x166 * 128, 0, d0(0x123, ea), 0x0234_5000 | PTE_I);
    assert!(
        translate(ea, Access::Load, &s, table.read())
            .expect("hit")
            .inhibited
    );
}

#[test]
fn a_16_mb_page_ignores_the_avpn_bit_inside_the_page_and_needs_a_matching_lp() {
    // Initial LB: LP = 0 selects 16 MB. Page index = offset >> 24 = 0x3,
    // so the hash is 0x123 ^ 0x3 = 0x120.
    let ea = 0x1300_0000;
    let s = thread(ea, 0x123, SLB_L, false);
    let pteg = HTAB + 0x120 * 128;
    let mut table = Table::default();
    table.put(pteg, 0, d0(0x123, ea) | PTE_L, 0x0500_0000 | 0b10);
    let t = translate(ea + 0x0080_1234, Access::Load, &s, table.read()).expect("hit");
    assert_eq!((t.ea, t.ra, t.len), (0x1300_0000, 0x0500_0000, 1 << 24));
    // VA bit 56 (offset 0x0080_0000) lies inside the page: q = 4, so the
    // match does not compare AVPN bit 56.
    let mut table = Table::default();
    table.put(pteg, 0, d0(0x123, ea + 0x0080_0000) | PTE_L, 0x0500_0000);
    assert_eq!(
        translate(ea, Access::Load, &s, table.read()).map(|t| t.ra),
        Ok(0x0500_0000)
    );
    // A PTE with LP 1 does not match an SLB entry with LP 0.
    let mut table = Table::default();
    table.put(pteg, 0, d0(0x123, ea) | PTE_L, 0x0500_0000 | PPE_PTE_LP);
    assert_eq!(
        translate(ea, Access::Load, &s, table.read()),
        Err(PpuInterrupt::DataStorage {
            ea,
            store: false,
            protection: false,
        })
    );
}

#[test]
fn a_64_kb_page_takes_its_size_from_lb_bits_2_to_3_and_its_lp_from_the_rpn() {
    // Initial LB: LP = 1 selects 64 KB. Page index = 0x0004_5678 >> 16 = 4,
    // so the hash is 0x123 ^ 0x4 = 0x127.
    let ea = 0x1004_5678;
    let s = thread(ea, 0x123, SLB_L | PPE_SLB_LP, false);
    let mut table = Table::default();
    table.put(
        HTAB + 0x127 * 128,
        0,
        d0(0x123, ea) | PTE_L,
        0x0234_0000 | PPE_PTE_LP,
    );
    let t = translate(ea, Access::Load, &s, table.read()).expect("hit");
    // The LP bit is the RPN's low bit and is no part of the real address.
    assert_eq!((t.ea, t.ra, t.len), (0x1004_0000, 0x0234_0000, 1 << 16));
    assert_eq!(t.real(ea), Some(0x0234_5678));
}

#[test]
fn htabsize_takes_more_hash_bits_into_the_group_index() {
    // A 2^12-group table at 16 MB: HTABSIZE 1 adds hash bit 11.
    let ea = 0x1000_0000;
    let mut s = thread(ea, 0x0800, 0, false);
    s.partition = Partition::new(HTAB | 1, 1 << 62, 0, Partition::INITIAL.lb()).expect("p");
    // Hash = VSID 0x800 XOR page index 0 = 0x800: the upper half.
    let mut table = Table::default();
    table.put(HTAB + 0x800 * 128, 0, d0(0x0800, ea), 0x0300_0000);
    let t = translate(ea, Access::Load, &s, table.read()).expect("upper half");
    assert_eq!(t.ra, 0x0300_0000);
    // With HTABSIZE 0 the same hash indexes group 0.
    let small = thread(ea, 0x0800, 0, false);
    assert_eq!(
        translate(ea, Access::Load, &small, table.read()),
        Err(PpuInterrupt::DataStorage {
            ea,
            store: false,
            protection: false,
        })
    );
}

#[test]
fn partition_new_refuses_what_the_architecture_does_not_allow() {
    use PartitionRefusal as R;
    let lb = Partition::INITIAL.lb();
    assert_eq!(Partition::new(0, 1 << 38, 0, lb), Ok(Partition::INITIAL));
    assert_eq!(
        Partition::new(1 << 62, 1 << 62, 0, lb),
        Err(R::Sdr1(1 << 62))
    );
    assert_eq!(Partition::new(0x20, 1 << 62, 0, lb), Err(R::Sdr1(0x20)));
    assert_eq!(Partition::new(29, 1 << 62, 0, lb), Err(R::Sdr1(29)));
    // HTABSIZE 1 needs HTABORG bit 45 (0x4_0000) clear.
    assert_eq!(
        Partition::new(0x4_0001, 1 << 62, 0, lb),
        Err(R::Sdr1(0x4_0001))
    );
    assert_eq!(
        Partition::new(0x8_0001, 1 << 62, 0, lb).map(|p| p.sdr1()),
        Ok(0x8_0001)
    );
    for limit in [0, 3 << 20, 1 << 11, 1 << 63] {
        assert_eq!(
            Partition::new(0, limit, 0, lb),
            Err(R::RealModeLimit(limit))
        );
    }
    assert_eq!(
        Partition::new(0, 1 << 12, 0, lb).map(|p| p.rm_limit()),
        Ok(1 << 12)
    );
    assert_eq!(
        Partition::new(0, 1 << 28, 0x0800_0000, lb),
        Err(R::RealModeOffset(0x0800_0000))
    );
    assert_eq!(
        Partition::new(0, 1 << 61, 1 << 62, lb),
        Err(R::RealModeOffset(1 << 62))
    );
    for bad in [0b1100, 0b0011, 0x10] {
        assert_eq!(Partition::new(0, 1 << 62, 0, bad), Err(R::LargePages(bad)));
    }
}

/// [CBE-Handbook p:87 s:4.2.6.4] The PPE allows at most HTABSIZE 24 and HTABORG x'FFFFFF'.
#[test]
fn partition_new_refuses_a_page_table_past_the_ppes_42_bit_real_space() {
    use PartitionRefusal as R;
    let lb = Partition::INITIAL.lb();
    assert_eq!(Partition::new(24, 1 << 62, 0, lb).map(|p| p.sdr1()), Ok(24));
    assert_eq!(Partition::new(25, 1 << 62, 0, lb), Err(R::Sdr1(25)));
    let top = 0xFF_FFFF << 18;
    assert_eq!(
        Partition::new(top, 1 << 62, 0, lb).map(|p| p.sdr1()),
        Ok(top)
    );
    assert_eq!(
        Partition::new(1 << 42, 1 << 62, 0, lb),
        Err(R::Sdr1(1 << 42))
    );
}

/// [PPC-Book3 p:30 s:4.2.6.2] Offset real mode takes the low-order m bits of EA bits 2:63 ORed with the RMOR as the real address.
/// [CBE-Handbook p:87 s:4.2.6.4] The PPE supports only a 42-bit real address.
#[test]
fn real_mode_drops_the_real_address_bits_past_the_ppes_42() {
    let read = Table::default();
    // A limit past the 42 bits passes EA bit 42; the real address drops it.
    let mut s = SupervisorState::new(MSR_SF);
    s.partition = Partition::new(0, 1 << 62, 0, Partition::INITIAL.lb()).expect("p");
    let ea = 0x0000_0400_0000_5678;
    let t = translate(ea, Access::Load, &s, read.read()).expect("inside the limit");
    assert_eq!((t.ea, t.ra, t.len), (0x0000_0400_0000_0000, 0, 1 << 42));
    assert_eq!(t.real(ea), Some(0x5678));
    // An offset bit past the 42 drops out too.
    s.partition =
        Partition::new(0, 1 << 28, (1 << 42) | 0x1000_0000, Partition::INITIAL.lb()).expect("p");
    let t = translate(0x1234, Access::Store, &s, read.read()).expect("in limit");
    assert_eq!(t.real(0x1234), Some(0x1000_1234));
}

/// [CBE-Handbook p:336 s:11.2] Table 11-1: RMLS 0000 selects the 256 GB limit.
#[test]
fn the_initial_partition_has_sdr1_0_real_mode_256_gb_at_0_and_large_pages_16_mb_and_64_kb() {
    assert_eq!(Partition::INITIAL.large_page_log2(false), 24);
    assert_eq!(Partition::INITIAL.large_page_log2(true), 16);
    assert_eq!(Partition::INITIAL.rm_limit(), 256 << 30);
    assert_eq!(Partition::INITIAL.rm_offset(), 0);
    assert_eq!(Partition::INITIAL.sdr1(), 0);
}

/// [CBE-Handbook p:89 s:4.2.6.6] LB bits 0:1 size the pages with LP = 0 and bits 2:3 those with LP = 1.
#[test]
fn an_lb_of_1001_sizes_lp_0_at_64_kb_and_lp_1_at_1_mb() {
    let p = Partition::new(0, 1 << 62, 0, 0b1001).expect("LB 10 and 01 are sizes");
    assert_eq!(p.large_page_log2(false), 16);
    assert_eq!(p.large_page_log2(true), 20);
}
