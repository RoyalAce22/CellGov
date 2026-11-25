use super::hypercall_ctr_decrement_tests::VECTOR;
use super::hypercall_entry_tests::{code, ordinals, pointers, word, Segment};
use super::hypercall_real_mode_switch_tests::{lis_sf, ori, shift_left};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};

/// The kernel address of the window `li r11,75; nop; sc; blr`.
const WINDOW: u64 = KERNEL + 0x3000;

/// The window's `sc`.
const SC: u64 = WINDOW + 8;

/// The kernel address of a window that no path enters: `li r11,77; sc`.
const QUIET: u64 = KERNEL + 0x4000;

/// The address of low code that only a real-mode branch to a kernel
/// address reaches: `li r4,0x1000` at [`LOW`], then `mtctr r4; bctr`.
///
/// The `mtctr r4` is at 0x1_7000. A load of 1, 2, 4 or 8 bytes of the
/// fixture's file bytes gives no value with that real address, so only
/// the vector's `bctr` names it.
const LOW: u64 = 0x1_6FFC;

/// The real address of the `mtctr r4` after [`LOW`].
const MTCTR: u64 = LOW + 4;

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// A stub at [`VECTOR`], where control enters in real mode, that builds
/// `KERNEL + offset` in r3 and runs `mtctr r3; bctr`. The `bctr` reaches
/// only that constant, so it is closed. No segment holds the kernel
/// address, and real mode fetches from its real address, `offset`.
fn vector(offset: u64) -> Segment {
    code(
        VECTOR,
        &[
            lis_sf(3),
            shift_left(3, 32),
            word(PpuInstruction::Oris {
                ra: 3,
                rs: 3,
                imm: u16::try_from(offset >> 16).expect("a low address"),
            }),
            ori(3, offset & 0xFFFF),
            word(PpuInstruction::Mtctr { rs: 3 }),
            bctr(),
        ],
    )
}

/// The sites, with the vector's `bctr` to the kernel address whose real
/// address is `offset`.
///
/// A code pointer names [`LOW`], so a path falls through it into
/// `mtctr r4` with r4 = 0x1000. The 32-bit file word 0x3004 names the
/// window's `nop` at its real address. Only an indirect branch in real
/// mode whose target the bound does not hold enters that `nop`.
fn sites(offset: u64) -> Vec<(u64, Option<u16>)> {
    let low = code(
        LOW,
        &[li(4, 0x1000), word(PpuInstruction::Mtctr { rs: 4 }), bctr()],
    );
    let window = code(WINDOW, &[li(11, 75), NOP, PPC_SC_1, PPC_BLR]);
    let quiet = code(QUIET, &[li(11, 77), PPC_SC_1]);
    let mut sites = ordinals(
        0,
        &[vector(offset), low, window, quiet, pointers(&[LOW, 0x3004])],
    );
    sites.sort_unstable();
    sites
}

/// The vector's closed `bctr` in real mode enters the `mtctr r4` at real
/// address [`MTCTR`] with the r4 of the interrupt, which the scan does not
/// know. So the low `bctr` can reach any named address in real mode, and
/// the window's `nop` is an entry. Entered at [`LOW`], the low code
/// loads r4 itself, so its `bctr` reaches only 0x1000 and the window
/// keeps r11 = 75.
///
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
/// [PPC-Book3 p:62 s:5.4] An interrupt sets MSR bits IR and DR to 0 and MSR bit SF to 1.
#[test]
fn a_closed_real_mode_bcctr_enters_the_real_alias_of_its_target_with_unknown_registers() {
    assert_eq!(sites(MTCTR), [(SC, None), (QUIET + 4, Some(77))]);
    assert_eq!(sites(LOW), [(SC, Some(75)), (QUIET + 4, Some(77))]);
}
