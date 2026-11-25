use super::hypercall_ctr_decrement_tests::{bc, into_window, quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_entry_tests::{code, ordinals, word, Segment};
use super::hypercall_tests::{KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// A word outside the interrupt vectors, so that no interrupt starts a
/// path there.
const ROUTINE: u64 = KERNEL + 0x7000;

/// Every site unresolved.
const LOST: [(u64, Option<u16>); 2] = [(SC, None), (QUIET_SC, None)];

/// Both sites resolved, as the windows load them.
const RESOLVED: [(u64, Option<u16>); 2] = [(SC, Some(75)), (QUIET_SC, Some(77))];

/// The ordinals of the two windows, with `segments` and the path past
/// the window's `blr`, which enters the window after its `li r11` in
/// real mode.
fn sites(segments: impl IntoIterator<Item = Segment>) -> Vec<(u64, Option<u16>)> {
    let mut all: Vec<Segment> = segments.into_iter().collect();
    all.extend([window(&into_window()), quiet()]);
    ordinals(0, &all)
}

/// The vector, which branches to [`ROUTINE`], and `words` at
/// [`ROUTINE`]. No start enters a word of `words`.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
fn behind_vector(words: &[u32]) -> [Segment; 2] {
    let offset = i32::try_from(ROUTINE - VECTOR).expect("a near branch");
    let branch = word(PpuInstruction::B {
        offset,
        aa: false,
        link: false,
    });
    [code(VECTOR, &[branch]), code(ROUTINE, words)]
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// `bdnz $+4`: a decrement of CTR on both paths to the next word.
fn bdnz_next() -> u32 {
    bc(0b10000, 4)
}

/// `ld r3,0(r1); mtctr r3`: a CTR that the code loads.
fn loaded_ctr() -> [u32; 2] {
    [
        word(PpuInstruction::Ld {
            rt: 3,
            ra: 1,
            imm: 0,
        }),
        word(PpuInstruction::Mtctr { rs: 3 }),
    ]
}

/// `mfctr r5; mtctr r5`: a copy of CTR through r5 and back.
fn round_trip_r5() -> [u32; 2] {
    [
        word(PpuInstruction::Mfctr { rt: 5 }),
        word(PpuInstruction::Mtctr { rs: 5 }),
    ]
}

/// The `bdnz` leaves CTR at the loaded value less 1 on its taken path,
/// which the scan cannot prove.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_real_mode_bctr_after_a_bdnz_of_a_loaded_ctr_leaves_every_site_unresolved() {
    let mut words = loaded_ctr().to_vec();
    words.extend([bdnz_next(), bctr()]);
    assert_eq!(sites(behind_vector(&words)), LOST);
}

/// With no decrement, the loaded CTR is a value that the bound takes to
/// hold a named address.
#[test]
fn a_real_mode_bctr_of_a_loaded_ctr_keeps_both_sites_resolved() {
    let mut words = loaded_ctr().to_vec();
    words.extend([NOP, bctr()]);
    assert_eq!(sites(behind_vector(&words)), RESOLVED);
}

/// An interrupt enters the vector with a CTR that no word of the scan
/// writes, and the `bdnz` decrements it.
///
/// [PPC-Book3 p:62 s:5.4] An interrupt sets MSR bits IR and DR to 0.
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_real_mode_bctr_after_a_bdnz_of_the_ctr_at_an_interrupt_vector_leaves_every_site_unresolved() {
    assert_eq!(sites([code(VECTOR, &[bdnz_next(), bctr()])]), LOST);
}

/// The vector calls a routine outside the vectors, which decrements the
/// CTR that it starts with.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_real_mode_bctr_after_a_bdnz_of_the_ctr_at_a_call_target_leaves_every_site_unresolved() {
    let offset = i32::try_from(ROUTINE - VECTOR).expect("a near branch");
    let call = word(PpuInstruction::B {
        offset,
        aa: false,
        link: true,
    });
    let segments = [
        code(VECTOR, &[call, PPC_BLR]),
        code(ROUTINE, &[bdnz_next(), bctr()]),
    ];
    assert_eq!(sites(segments), LOST);
}

/// The vector's `bctr` reaches the real address of [`ROUTINE`], which
/// decrements the CTR that it starts with.
///
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
#[test]
fn a_real_mode_bctr_after_a_bdnz_of_the_ctr_at_an_indirect_target_leaves_every_site_unresolved() {
    let low = u16::try_from(ROUTINE - KERNEL).expect("a low address");
    let to_routine = [
        word(PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: 0,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: low,
        }),
        word(PpuInstruction::Mtctr { rs: 3 }),
        bctr(),
    ];
    let segments = [
        code(VECTOR, &to_routine),
        code(ROUTINE, &[bdnz_next(), bctr()]),
    ];
    assert_eq!(sites(segments), LOST);
}

/// A copy of a loaded CTR is a GPR with no copy that the flow records,
/// so the `mtctr` of it copies a value that no word of the scan writes.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr`, forms of `mtspr`, copy RS into LR or CTR.
#[test]
fn a_real_mode_bctr_of_a_copy_of_a_loaded_ctr_keeps_both_sites_resolved() {
    let mut words = loaded_ctr().to_vec();
    words.extend(round_trip_r5());
    words.push(bctr());
    assert_eq!(sites(behind_vector(&words)), RESOLVED);
}

/// The copy of the decremented CTR is a value that the scan cannot
/// prove, and the `mtctr` puts it back into CTR.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
#[test]
fn a_real_mode_bctr_of_a_copy_of_a_decremented_loaded_ctr_leaves_every_site_unresolved() {
    let mut words = loaded_ctr().to_vec();
    words.push(bdnz_next());
    words.extend(round_trip_r5());
    words.push(bctr());
    assert_eq!(sites(behind_vector(&words)), LOST);
}
