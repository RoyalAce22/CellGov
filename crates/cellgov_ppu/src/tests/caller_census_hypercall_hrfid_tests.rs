use super::hypercall_ctr_decrement_tests::VECTOR;
use super::hypercall_entry_tests::{code, ordinals, pointers, word, Segment};
use super::hypercall_real_mode_switch_tests::{mtspr, window, SC};
use super::hypercall_tests::{b, li, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};
use cellgov_ps3_abi::hw::ppu_supervisor::SPR_HSRR0;

/// [PPC-Book3 p:119 s:Appendix G] `rfid` and `hrfid` are XL-form words with primary opcode 19 and extended opcodes 18 and 274.
const HRFID: u32 = (19 << 26) | (274 << 1);

/// The address of a window below 4 GiB: `li r11,75; nop; sc; blr`.
const LOW_WINDOW: u64 = 0x3000;

/// The address of the low window's `sc`.
const LOW_SC: u64 = LOW_WINDOW + 8;

/// The address of a stub below 4 GiB.
const STUB: u64 = 0x6000;

/// A segment at [`VECTOR`], where control enters in real mode, that holds
/// only `last`.
fn vector(last: u32) -> Segment {
    code(VECTOR, &[last])
}

/// A real-mode `hrfid` can fetch from any HSRR0 under any HSRR1, so each
/// word can run in real mode after it. The path past the window's `blr`
/// then reaches the window's `nop` through its absolute branch. A `nop`
/// in place of the `hrfid` puts no word past the vector in real mode.
///
/// [PPC-Book3 p:13 s:2.3] `hrfid` sets NIA to HSRR0 0:61 || 0b00 and takes the new MSR from HSRR1.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn an_hrfid_can_put_any_word_into_real_mode() {
    let site = |last: u32, other: i16| ordinals(0, &[vector(last), window(other)]);
    assert_eq!(site(HRFID, 76), [(SC, None)]);
    assert_eq!(site(HRFID, 75), [(SC, Some(75))]);
    assert_eq!(site(NOP, 76), [(SC, Some(75))]);
}

/// The stub builds 0x1_0000_3004 in r3 and runs `bctr` to it. No real
/// alias of that address is a word below 4 GiB, so only a `bctr` in
/// 32-bit mode enters the low window's `nop`. An `hrfid` can set
/// MSR\[SF\] to 0 from an HSRR1 that the scan does not follow, so the
/// `bctr` can run in 32-bit mode.
///
/// [PPC-Book3 p:13 s:2.3] `hrfid` places the OR of HSRR1 bits 0 and 1 into MSR\[SF\].
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
#[test]
fn an_hrfid_can_put_a_bcctr_into_32_bit_mode() {
    let site = |last: u32| {
        let stub = [
            li(3, 1),
            word(PpuInstruction::Rldicr {
                ra: 3,
                rs: 3,
                sh: 32,
                me: 31,
                rc: false,
            }),
            word(PpuInstruction::Ori {
                ra: 3,
                rs: 3,
                imm: 0x3004,
            }),
            word(PpuInstruction::Mtctr { rs: 3 }),
            word(PpuInstruction::Bcctr {
                bo: 0b10100,
                bi: 0,
                link: false,
            }),
        ];
        let low_window = code(LOW_WINDOW, &[li(11, 75), NOP, PPC_SC_1, PPC_BLR]);
        ordinals(
            0,
            &[
                vector(last),
                code(STUB, &stub),
                low_window,
                pointers(&[STUB]),
            ],
        )
    };
    assert_eq!(site(HRFID), [(LOW_SC, None)]);
    assert_eq!(site(NOP), [(LOW_SC, Some(75))]);
}

/// The number of paths that bring a different constant in r3 to the join
/// of [`lost_hsrr0`]: one more than the join keeps before its bound rises.
const PATHS: u64 = 17;

/// The address of the first of [`PATHS`] windows below 4 GiB, each
/// `li r11,75; nop; sc; b .`.
const WINDOWS: u64 = 0x3000;

/// The address of a window below 4 GiB that no path enters:
/// `li r11,77; sc`.
const QUIET: u64 = 0x5100;

/// The sites of the windows and of the window at [`QUIET`], by address,
/// with a stub that runs `hrfid` with an HSRR0 that the join dropped.
///
/// Each path of the stub loads the address of one window in r3 and
/// branches to the join. The join adds 4 to r3 and copies it to HSRR0.
/// So the `hrfid` can enter the `nop` of each window, and the join drops
/// one of those addresses until its bound rises. With `ends` set, the
/// stub ends in `blr` in place of `hrfid`. Code pointers name the first
/// word of each path.
fn lost_hsrr0(ends: u32) -> Vec<(u64, Option<u16>)> {
    let join = STUB + 8 * PATHS;
    let starts: Vec<u64> = (0..PATHS).map(|path| STUB + 8 * path).collect();
    let mut stub = Vec::new();
    for path in 0..PATHS {
        let at = STUB + 8 * path + 4;
        let window = WINDOWS + 16 * path;
        stub.push(li(3, i16::try_from(window).expect("a low address")));
        stub.push(b(i32::try_from(join - at).expect("a near branch")));
    }
    stub.push(word(PpuInstruction::Addi {
        rt: 3,
        ra: 3,
        imm: 4,
    }));
    stub.extend([mtspr(SPR_HSRR0, 3), ends]);
    let windows: Vec<u32> = (0..PATHS)
        .flat_map(|_| [li(11, 75), NOP, PPC_SC_1, b(0)])
        .collect();
    let quiet = code(QUIET, &[li(11, 77), PPC_SC_1]);
    let segments = [
        code(STUB, &stub),
        code(WINDOWS, &windows),
        quiet,
        pointers(&starts),
    ];
    let mut sites = ordinals(0, &segments);
    sites.sort_unstable();
    sites
}

/// The `hrfid` takes an HSRR0 that can hold a constant that the join
/// dropped, so the bound of the join of r3 rises, and the join then keeps
/// each window's address. The join's `addi` names each window's `nop`.
/// So each window's `sc` is unresolved, and the `sc` at [`QUIET`] keeps
/// r11 = 77. With `blr` in place of the `hrfid`, no bound rises, and the
/// window whose address the join dropped keeps r11 = 75.
///
/// [PPC-Book3 p:13 s:2.3] `hrfid` sets NIA to HSRR0 0:61 || 0b00 and takes the new MSR from HSRR1.
#[test]
fn an_hrfid_to_an_hsrr0_that_the_join_dropped_leaves_each_window_site_unresolved() {
    let window_sc = |path: u64| WINDOWS + 16 * path + 8;
    let mut expected: Vec<(u64, Option<u16>)> =
        (0..PATHS).map(|path| (window_sc(path), None)).collect();
    expected.push((QUIET + 4, Some(77)));
    assert_eq!(lost_hsrr0(HRFID), expected);
    let resolved = lost_hsrr0(PPC_BLR)
        .into_iter()
        .filter(|&(at, ordinal)| at != QUIET + 4 && ordinal.is_some())
        .count();
    assert_eq!(resolved, 1);
}
