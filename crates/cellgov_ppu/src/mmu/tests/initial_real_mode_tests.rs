//! Real addressing mode under [`Partition::INITIAL`]: EA bits 2:63 below
//! 256 GB translate, and 256 GB and above take the storage interrupt.
//!
//! [CBE-Handbook p:336 s:11.2] Table 11-1: the largest RMLS limit is 256 GB, and an access past the limit takes the ISI or DSI with SRR1\[36\] = 1.

use std::collections::BTreeMap;

use cellgov_ps3_abi::hw::ppu_mmu::PTEG_BYTES;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_SF;

use super::*;
use crate::exec::{PpuFault, PpuInterrupt};
use crate::supervisor::SupervisorState;

/// 256 GB: the first EA (bits 2:63) the initial partition refuses.
const LIMIT: u64 = 0x0000_0040_0000_0000;

/// The translation or the interrupt for `ea` on a real-mode thread with
/// the initial partition. A CellGov stop fails the test.
fn translate(ea: u64, access: Access) -> Result<Translation, PpuInterrupt> {
    let table: BTreeMap<u64, [u8; PTEG_BYTES]> = BTreeMap::new();
    let sup = SupervisorState::new(MSR_SF);
    translate_entry(ea, access, &sup, |a| table.get(&a).copied())
        .0
        .map_err(|fault| match fault {
            PpuFault::Interrupt(interrupt) => interrupt,
            stop => panic!("translation stopped where an interrupt was due: {stop}"),
        })
}

#[test]
fn the_initial_partition_translates_the_last_doubleword_below_256_gb_to_itself() {
    for high in [0, 0x8000_0000_0000_0000] {
        let ea = high | (LIMIT - 8);
        for access in [Access::Load, Access::Store, Access::Fetch] {
            let t = translate(ea, access).expect("below the limit");
            assert_eq!((t.ea, t.ra, t.len), (high, 0, LIMIT), "{ea:#x} {access:?}");
            assert_eq!(t.real(ea), Some(LIMIT - 8), "{ea:#x} {access:?}");
        }
    }
}

#[test]
fn the_initial_partition_refuses_256_gb_with_the_protection_cause() {
    for ea in [LIMIT, 0x8000_0040_0000_0000, 0x3FFF_FFFF_FFFF_FFF8] {
        for store in [false, true] {
            let access = if store { Access::Store } else { Access::Load };
            assert_eq!(
                translate(ea, access),
                Err(PpuInterrupt::DataStorage {
                    ea,
                    store,
                    protection: true,
                }),
                "{ea:#x} {access:?}"
            );
        }
        assert_eq!(
            translate(ea, Access::Fetch),
            Err(PpuInterrupt::InstructionStorage {
                no_execute: false,
                protection: true,
            }),
            "{ea:#x} fetch"
        );
    }
}
