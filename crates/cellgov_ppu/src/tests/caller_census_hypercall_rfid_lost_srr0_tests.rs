use super::hypercall_entry_tests::{code, ordinals, pointers, word, Segment};
use super::hypercall_real_mode_switch_tests::{lis_sf, mtspr, ori, RFID};
use super::hypercall_tests::{b, li, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_IR, SPR_SRR0, SPR_SRR1};

/// The address of the first window below 4 GiB: `li r11,75; nop; sc; b .`.
const WINDOW: u64 = 0x3000;

/// The address of the stub that runs the `rfid`.
const STUB: u64 = 0x6000;

/// The number of paths that bring a different constant in r3 to the join
/// of [`lost_srr0`]: one more than the join keeps.
const PATHS: u64 = 17;

/// The windows at [`WINDOW`], one after the other, each four words long.
fn windows() -> Segment {
    let words: Vec<u32> = (0..PATHS)
        .flat_map(|_| [li(11, 75), NOP, PPC_SC_1, b(0)])
        .collect();
    code(WINDOW, &words)
}

/// The sites of the windows, with a stub that runs `rfid` with an SRR0
/// that the scan dropped.
///
/// Each path of the stub loads the address of one window in r3 and
/// branches to the join. The join adds 4 to r3 and copies it to SRR0. So
/// the `rfid` can enter the `nop` of each window, and the join drops one
/// of those addresses. With `through` set, a `b` to the next word then
/// makes a second join, which only the join flow reaches. The stub gives
/// SRR1 IR, with SF too when `sf` is set. Code pointers name the first
/// word of each path.
fn lost_srr0(sf: bool, through: bool) -> Vec<(u64, Option<u16>)> {
    let join = STUB + 8 * PATHS;
    let starts: Vec<u64> = (0..PATHS).map(|path| STUB + 8 * path).collect();
    let mut stub = Vec::new();
    for path in 0..PATHS {
        let at = STUB + 8 * path + 4;
        let window = WINDOW + 16 * path;
        stub.push(li(3, i16::try_from(window).expect("a low address")));
        stub.push(b(i32::try_from(join - at).expect("a near branch")));
    }
    stub.push(word(PpuInstruction::Addi {
        rt: 3,
        ra: 3,
        imm: 4,
    }));
    stub.push(mtspr(SPR_SRR0, 3));
    if through {
        stub.push(b(4));
    }
    if sf {
        stub.extend([lis_sf(5), ori(5, MSR_IR)]);
    } else {
        stub.push(li(5, i16::try_from(MSR_IR).expect("a low MSR bit")));
    }
    stub.extend([mtspr(SPR_SRR1, 5), RFID]);
    ordinals(0, &[code(STUB, &stub), windows(), pointers(&starts)])
}

/// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
#[test]
fn an_rfid_to_an_srr0_that_the_join_dropped_leaves_every_site_unresolved() {
    for (sf, through) in [(true, false), (false, false), (true, true), (false, true)] {
        let sites = lost_srr0(sf, through);
        assert_eq!(sites.len(), 17, "SF {sf}, through {through}");
        for (address, ordinal) in sites {
            assert_eq!(ordinal, None, "SF {sf}, through {through}, at {address:#x}");
        }
    }
}
