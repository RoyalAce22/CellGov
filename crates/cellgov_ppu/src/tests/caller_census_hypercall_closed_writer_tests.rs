use super::hypercall_ctr_decrement_tests::{quiet, window, QUIET_SC, SC};
use super::hypercall_entry_tests::{code, data_bytes, ordinals, pointers, word};
use super::hypercall_real_mode_switch_tests::{lis_sf, mtmsrd, ori, shift_left};
use super::hypercall_tests::{KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The kernel address of the stub, which a code pointer names, so control
/// enters it outside real mode.
const STUB: u64 = KERNEL + 0x5000;

/// The kernel address of the target of the stub's indirect branch.
const TARGET: u64 = KERNEL + 0x6000;

/// The address of a data segment that holds one 32-bit word.
const DATA: u64 = 0x2100_0000;

/// The real address of the window's `nop`, between its `li r11` and `sc`.
const BETWEEN: u32 = 0x3004;

/// A stub that builds [`TARGET`] in r3 and copies it with `write`, then
/// runs `set` with an MSR value in r5 that has IR at 0, then runs
/// `branch`.
fn stub(write: PpuInstruction, set: u32, branch: u32) -> [u32; 7] {
    [
        lis_sf(3),
        shift_left(3, 32),
        ori(3, TARGET & 0xFFFF),
        word(write),
        lis_sf(5),
        set,
        branch,
    ]
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// The ordinals with `stub` at [`STUB`] and `b .` at [`TARGET`]. A word
/// of the file bytes names [`BETWEEN`], so a `bcctr` or `bclr` in real
/// mode with a target that the scan does not hold can enter the window's
/// `nop`.
fn sites(stub: &[u32]) -> Vec<(u64, Option<u16>)> {
    let named = data_bytes(DATA, BETWEEN.to_be_bytes().to_vec());
    let stay = word(PpuInstruction::B {
        offset: 0,
        aa: false,
        link: false,
    });
    ordinals(
        0,
        &[
            code(STUB, stub),
            code(TARGET, &[stay]),
            window(&[]),
            quiet(),
            named,
            pointers(&[STUB]),
        ],
    )
}

const RESOLVED: [(u64, Option<u16>); 2] = [(SC, Some(75)), (QUIET_SC, Some(77))];

const UNRESOLVED: [(u64, Option<u16>); 2] = [(SC, None), (QUIET_SC, Some(77))];

/// The `mtmsrd` puts the `bctr` in real mode. The bound takes the switch
/// target as an entry. Control can enter it with any CTR, so the CTR
/// write before the switch does not close the `bctr`.
///
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\].
/// [PPC-Book1 p:81 s:3.3.13] `mtctr` copies RS into CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_at_a_real_mode_switch_target_after_a_ctr_write_can_enter_a_named_real_address() {
    let stub = stub(PpuInstruction::Mtctr { rs: 3 }, mtmsrd(5, false), bctr());
    assert_eq!(sites(&stub), UNRESOLVED);
}

/// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\].
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` copies RS into LR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_blr_at_a_real_mode_switch_target_after_an_lr_write_can_enter_a_named_real_address() {
    let stub = stub(PpuInstruction::Mtlr { rs: 3 }, mtmsrd(5, false), PPC_BLR);
    assert_eq!(sites(&stub), UNRESOLVED);
}

/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_with_no_switch_before_it_enters_no_named_real_address() {
    let stub = stub(PpuInstruction::Mtctr { rs: 3 }, NOP, bctr());
    assert_eq!(sites(&stub), RESOLVED);
}
