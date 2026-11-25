use super::hypercall_constant_join_tests::{addi, bctr, mtctr};
use super::hypercall_ctr_decrement_tests::bc;
use super::hypercall_entry_tests::{code, ordinals, pointers, word};
use super::hypercall_real_mode_switch_tests::mtmsrd;
use super::hypercall_tests::{li, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_IR;

/// The address of a window below 4 GiB: `li r11,75; nop; sc; blr`.
const LOW_WINDOW: u64 = 0x3000;

/// The address of a second window, `li r11,77; sc`. Its `sc` is at 4 past
/// a multiple of 0x20.
const QUIET: u64 = 0x4000;

/// The address of the stub, which a code pointer names.
const STUB: u64 = 0x6000;

/// The sites, by address, when the stub runs `li r5,MSR_IR; mtmsrd r5,0`,
/// builds 0x1_0000_0000 | `low` in r3 with `li r3,1; rldicr r3,r3,32,31;
/// ori r3,r3,low`, then runs `loop: addi r3,r3,0x20; blt loop; mtctr r3;
/// bctr`.
///
/// The `mtmsrd` clears SF, so the `bctr` runs in 32-bit mode and enters
/// the low-order 32 bits of each value of the class. No word from the
/// `rldicr` to the `mtctr` is at 0 or 8 past a multiple of 0x20.
///
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and MSR\[SF\] to RS bit 0 | RS bit 1.
/// [PPC-Book1 p:72 s:3.3.12.1] `rldicr` rotates RS left by SH and ANDs it with MASK(0, ME).
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1 and does not decrement CTR.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
fn sites(low: u16) -> Vec<(u64, Option<u16>)> {
    let stub = [
        li(5, i16::try_from(MSR_IR).expect("a low MSR bit")),
        mtmsrd(5, false),
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
            imm: low,
        }),
        addi(3, 3, 0x20),
        bc(0b01100, -4),
        mtctr(3),
        bctr(),
    ];
    let segments = [
        code(LOW_WINDOW, &[li(11, 75), NOP, PPC_SC_1, PPC_BLR]),
        code(QUIET, &[li(11, 77), PPC_SC_1]),
        code(STUB, &stub),
        pointers(&[STUB]),
    ];
    let mut sites = ordinals(0, &segments);
    sites.sort_unstable();
    sites
}

/// The loop steps r3 by 0x20 from 0x1_0000_3000 or from 0x1_0000_4008.
/// No value that the scan steps the loop through has the low-order word
/// of the window's `sc`. In 32-bit mode the `bctr` enters each word at the
/// low-order 32 bits of a value of the class:
/// - from the first base, each word at a multiple of 0x20;
/// - from the second base, each word at 8 past a multiple of 0x20, the
///   window's `sc` among them.
///
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_in_32_bit_mode_of_a_loop_gpr_above_4_gib_enters_the_class_words_below_4_gib() {
    let quiet = (QUIET + 4, Some(77));
    assert_eq!(sites(0x3000), [(LOW_WINDOW + 8, Some(75)), quiet]);
    assert_eq!(sites(0x4008), [(LOW_WINDOW + 8, None), quiet]);
}
