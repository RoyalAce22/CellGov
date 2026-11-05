//! An enabled FP exception raises no interrupt in a PS3 process, held to
//! two captures from the reference console. `ppu_fpscr_enabled_moves`
//! sets an exception bit and its enable bit with the FPSCR moves;
//! `ppu_fp_enabled_results` raises each exception from an arithmetic
//! instruction under its enable. Both processes run to their end.

use super::console_tests::{captured, mtfsb1, mtfsfi, Replay};
use cellgov_ps3_abi::hw::ppc_isa::PPC_FPSCR_FEX;

/// `ppu_fpscr_enabled_moves`, in the order its `main` runs. Each
/// instruction must continue: [`Replay::run`] refuses any other verdict.
fn replay_enabled_moves() -> Vec<u32> {
    let mut r = Replay::new(9);
    for (at, value) in [(2, 0x0400_0010), (3, 0x0100_0080)] {
        r.clear();
        r.mtfsf(0xFF, value);
        r.out[at] = r.fpscr();
        r.out[1] += 1;
    }
    let pairs = [
        (4, mtfsb1(5), mtfsfi(6, 1)),
        (5, mtfsfi(6, 1), mtfsb1(5)),
        (6, mtfsb1(28), mtfsb1(6)),
        (7, mtfsb1(25), mtfsb1(3)),
        (8, mtfsb1(26), mtfsb1(4)),
    ];
    for (at, first, second) in pairs {
        r.clear();
        r.run(first);
        r.run(second);
        r.out[at] = r.fpscr();
        r.out[1] += 1;
    }
    r.out
}

/// [PPC-Book1 p:95 s:4.4] a Move To FPSCR that sets an exception bit and its enable bit together causes an enabled exception; in Ignore Exceptions mode the system handler is not invoked.
#[test]
fn an_fpscr_move_that_enables_its_exception_continues_like_the_console() {
    let console = captured("ppu_fpscr_enabled_moves");
    let cellgov = replay_enabled_moves();
    assert_eq!(console.len(), cellgov.len(), "payload length");
    assert_eq!(console[1], cellgov[1], "steps that ran");
    let mut compared = 0;
    for w in 2..console.len() {
        assert_eq!(
            cellgov[w], console[w],
            "word {w}: CellGov 0x{:08x}, console 0x{:08x}",
            cellgov[w], console[w]
        );
        assert_ne!(console[w] & PPC_FPSCR_FEX, 0, "word {w} has FEX");
        compared += 1;
    }
    assert_eq!(compared, 7);
}

/// [PPC-Book1 p:95 s:4.4] in Ignore Exceptions mode an enabled exception invokes no handler.
/// [PPC-Book1 p:96 s:4.4] whether a result is stored depends only on the enable bits.
/// The ppu_fp_enabled_results console process runs past every enabled invalid operation and zero divide to its end.
#[test]
fn an_enabled_zero_over_zero_continues_and_keeps_frt() {
    assert_eq!(captured("ppu_fp_enabled_results")[2], 56);
    let mut r = Replay::new(0);
    r.clear();
    // VE and ZE: FPSCR bits 24 and 27.
    r.mtfsf(0xFF, 0x90);
    r.set_fpr(0, 0);
    r.set_fpr(1, 0);
    r.set_fpr(2, 0x1234_5678_1122_3344);
    // [PPC-Book1 p:112 s:4.6.5.1] fdiv is 63/18, A-form.
    r.run((63 << 26) | (2 << 21) | (1 << 11) | (18 << 1));
    assert_eq!(r.fpr(2), 0x1234_5678_1122_3344);
    assert_ne!(r.fpscr() & PPC_FPSCR_FEX, 0);
}
