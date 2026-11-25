use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};

/// A word in the interrupt vectors, where control enters in real mode.
///
/// [PPC-Book3 p:62 s:5.4] An interrupt sets MSR bits IR and DR to 0.
const VECTOR: u64 = KERNEL + 0x100;

/// A word outside the interrupt vectors.
const OUTSIDE: u64 = KERNEL + 0x5000;

/// The kernel address of the segment that [`window`] gives.
const WINDOW: u64 = KERNEL + 0x3000;

/// The kernel address of the window's `sc`.
const SC: u64 = WINDOW + 8;

/// The real address of the path past the window's `blr`.
const PAST: u16 = 0x3010;

/// A window at [`WINDOW`] that loads 75 and runs `sc`, and past its `blr`
/// a path that loads `other` and branches to the real address of the
/// word after the window's `li r11`.
fn window(other: i16) -> Segment {
    let ba = word(PpuInstruction::B {
        offset: 0x3004,
        aa: true,
        link: false,
    });
    code(
        WINDOW,
        &[li(11, 75), NOP, PPC_SC_1, PPC_BLR, li(11, other), ba],
    )
}

/// The real address of the window's `nop`, between its `li r11` and `sc`.
const BETWEEN: u16 = 0x3004;

/// `lis r3,0; ori r3,r3,PAST`.
fn build_past() -> [u32; 2] {
    build(PAST)
}

/// `lis r3,0; ori r3,r3,address`.
fn build(address: u16) -> [u32; 2] {
    [
        word(PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: 0,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: address,
        }),
    ]
}

fn mtctr_r3() -> u32 {
    word(PpuInstruction::Mtctr { rs: 3 })
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// The window's site, with `other` in the window and `stubs` as more
/// segments.
fn site(other: i16, stubs: Vec<Segment>) -> Vec<(u64, Option<u16>)> {
    let mut segments = stubs;
    segments.push(window(other));
    ordinals(0, &segments)
}

#[test]
fn a_real_mode_bctr_to_a_built_address_puts_its_absolute_branch_in_real_mode() {
    let stub = || {
        let [lis, ori] = build_past();
        code(VECTOR, &[lis, ori, mtctr_r3(), bctr()])
    };
    assert_eq!(site(76, vec![stub()]), [(SC, None)]);
    assert_eq!(site(75, vec![stub()]), [(SC, Some(75))]);
}

#[test]
fn a_real_mode_blr_to_a_built_address_puts_its_absolute_branch_in_real_mode() {
    let stub = || {
        let [lis, ori] = build_past();
        code(
            VECTOR,
            &[lis, ori, word(PpuInstruction::Mtlr { rs: 3 }), PPC_BLR],
        )
    };
    assert_eq!(site(76, vec![stub()]), [(SC, None)]);
    assert_eq!(site(75, vec![stub()]), [(SC, Some(75))]);
}

#[test]
fn a_real_mode_bctr_to_a_loaded_address_adds_no_path_to_an_unnamed_word() {
    let ld = word(PpuInstruction::Ld {
        rt: 3,
        ra: 4,
        imm: 0,
    });
    let stub = code(VECTOR, &[ld, mtctr_r3(), bctr()]);
    assert_eq!(site(76, vec![stub]), [(SC, Some(75))]);
}

#[test]
fn a_bctr_to_a_built_address_outside_real_mode_adds_no_path() {
    let [lis, ori] = build_past();
    let stub = code(OUTSIDE, &[lis, ori, mtctr_r3(), bctr()]);
    assert_eq!(site(76, vec![stub]), [(SC, Some(75))]);
}

#[test]
fn a_real_mode_bctr_whose_mtctr_runs_outside_real_mode_adds_no_path() {
    // The vector word branches straight to the `bctr`, so the `bctr` runs
    // in real mode with a CTR that the `mtctr` did not write.
    let [lis, ori] = build_past();
    let built = code(OUTSIDE, &[lis, ori, mtctr_r3(), bctr()]);
    let to_bctr = word(PpuInstruction::B {
        offset: i32::try_from(OUTSIDE + 0xC - VECTOR).expect("a near branch"),
        aa: false,
        link: false,
    });
    let vector = code(VECTOR, &[to_bctr]);
    assert_eq!(site(76, vec![vector, built]), [(SC, Some(75))]);
}

/// A real-mode `mtctr`, then `bdnz` to the next word, then `bctr`.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR is not 0.
#[test]
fn a_bdnz_to_the_next_word_between_mtctr_and_bctr_leaves_the_site_unresolved() {
    let [lis, ori] = build_past();
    let bdnz = word(PpuInstruction::Bc {
        bo: 0b10000,
        bi: 0,
        offset: 4,
        aa: false,
        link: false,
    });
    let stub = code(VECTOR, &[lis, ori, mtctr_r3(), bdnz, bctr()]);
    assert_eq!(site(76, vec![stub]), [(SC, None)]);
}

/// A `bctr` to [`BETWEEN`] enters the window after its `li r11` only
/// when it runs in real mode, where it reaches the word at that real
/// address.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_real_mode_bctr_to_the_real_address_of_a_word_before_sc_leaves_the_site_unresolved() {
    let stub = |at: u64| {
        let [lis, ori] = build(BETWEEN);
        code(at, &[lis, ori, mtctr_r3(), bctr()])
    };
    assert_eq!(site(75, vec![stub(VECTOR)]), [(SC, None)]);
    assert_eq!(site(75, vec![stub(OUTSIDE)]), [(SC, Some(75))]);
}
