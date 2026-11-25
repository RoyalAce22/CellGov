use super::hypercall_ctr_decrement_tests::SC;
use super::hypercall_entry_tests::word;
use super::hypercall_skip_return_tests::{mflr, mtlr, ordinals_with_routine};
use super::hypercall_tests::li;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The routine adds 3 to a copy of LR with `addi`, then adds 1 more with
/// `add`, and returns there, which reaches the `lis`. The scan follows no
/// constant through `add`, so the lost mark of the first step must hold
/// through the second step.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA, or to 0 when RA is 0.
/// [PPC-Book1 p:52 s:3.3.8] `add` places the sum of RA and RB into RT.
#[test]
fn a_return_to_a_copy_of_lr_stepped_twice_leaves_every_site_unresolved() {
    let add_3 = word(PpuInstruction::Addi {
        rt: 11,
        ra: 11,
        imm: 3,
    });
    let add_1 = word(PpuInstruction::Add {
        rt: 11,
        ra: 11,
        rb: 12,
        oe: false,
        rc: false,
    });
    let routine = [mflr(11), add_3, li(12, 1), add_1, mtlr(11), PPC_BLR];
    assert_eq!(ordinals_with_routine(&routine), [(SC, None)]);
}

/// The routine loads the word after the return point with `lwzu`, which
/// also places the address of that word into its base register, a copy of
/// LR. It returns to that address, which reaches the `lis`.
///
/// [PPC-Book1 p:33 s:3.3.2] A load with update places the EA into RA.
#[test]
fn a_return_to_a_copy_of_lr_that_a_load_with_update_steps_leaves_every_site_unresolved() {
    let load_next = word(PpuInstruction::Lwzu {
        rt: 0,
        ra: 11,
        imm: 4,
    });
    let routine = [mflr(11), load_next, mtlr(11), PPC_BLR];
    assert_eq!(ordinals_with_routine(&routine), [(SC, None)]);
}
