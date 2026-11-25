use super::hypercall_ctr_decrement_tests::{bc, into_window, quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_ctr_virtual_tests::{bctr, build, sites, LOADER, NO_ENTRY, OUTSIDE};
use super::hypercall_entry_tests::{code, ordinals, pointers, word, Segment};
use super::hypercall_tests::{li, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};

pub(super) fn mfctr(rt: u8) -> u32 {
    word(PpuInstruction::Mfctr { rt })
}

pub(super) fn mtctr(rs: u8) -> u32 {
    word(PpuInstruction::Mtctr { rs })
}

/// A `bl` to a routine that the image does not hold.
pub(super) fn call() -> u32 {
    word(PpuInstruction::B {
        offset: 0x1000,
        aa: false,
        link: true,
    })
}

/// A stub at [`OUTSIDE`]: [`build`] of 0x300C, `mtctr r3`, then
/// `between`, then `bctr`. The built address is the window's `blr`.
pub(super) fn copy_stub(between: &[u32]) -> Segment {
    let mut words = build(0x300C).to_vec();
    words.push(mtctr(3));
    words.extend_from_slice(between);
    words.push(bctr());
    code(OUTSIDE, &words)
}

/// The ordinals of the `sc` at 4 and of the quiet window, when the path
/// at [`OUTSIDE`] is a `bdnz` to a `blr`, then `between`, then `bctr`.
///
/// A path at [`LOADER`] loads 75 and branches to address 0, so the `sc`
/// at 4 resolves unless a `bctr` enters address 0. A code pointer names
/// [`OUTSIDE`].
pub(super) fn after_bdnz_falls_through(between: &[u32]) -> Vec<(u64, Option<u16>)> {
    let to_zero = word(PpuInstruction::B {
        offset: 0,
        aa: true,
        link: false,
    });
    let to_blr = i16::try_from((between.len() + 2) * 4).expect("in reach");
    let mut words = vec![bc(0b10000, to_blr)];
    words.extend_from_slice(between);
    words.extend([bctr(), PPC_BLR]);
    let segments = [
        code(0, &[NOP, PPC_SC_1]),
        code(LOADER, &[li(11, 75), to_zero]),
        code(OUTSIDE, &words),
        quiet(),
        pointers(&[OUTSIDE]),
    ];
    ordinals(NO_ENTRY, &segments)
}

/// The routine that the `bl` calls can return with any CTR, and the
/// copy in r3 keeps that CTR.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book1 p:81 s:3.3.13] `mtctr`, a form of `mtspr`, copies RS into CTR.
#[test]
fn a_bctr_after_a_call_and_a_copy_of_ctr_through_a_gpr_leaves_every_site_unresolved() {
    let between = [call(), mfctr(3), mtctr(3)];
    assert_eq!(
        sites(vec![copy_stub(&between)]),
        [(SC, None), (QUIET_SC, None)]
    );
}

/// A loop can run the `bdnz` more than once, so the CTR that r3 copies
/// can hold any value.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
#[test]
fn a_bctr_after_a_bdnz_and_a_copy_of_ctr_through_a_gpr_leaves_every_site_unresolved() {
    let between = [bc(0b10000, 4), mfctr(3), mtctr(3)];
    assert_eq!(
        sites(vec![copy_stub(&between)]),
        [(SC, None), (QUIET_SC, None)]
    );
}

/// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
#[test]
fn a_bctr_after_a_call_and_a_copy_of_ctr_through_two_gprs_leaves_every_site_unresolved() {
    let mr = word(PpuInstruction::Or {
        ra: 4,
        rs: 3,
        rb: 3,
        rc: false,
    });
    let between = [call(), mfctr(3), mr, mtctr(4)];
    assert_eq!(
        sites(vec![copy_stub(&between)]),
        [(SC, None), (QUIET_SC, None)]
    );
}

/// The handler of the `sc` can return r3 unchanged.
///
/// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt sets SRR0, SRR1 and MSR, and execution resumes at EA 0xC00.
#[test]
fn a_copy_of_ctr_after_a_call_that_crosses_an_sc_leaves_every_site_unresolved() {
    let sc = word(PpuInstruction::Sc { lev: 0 });
    let between = [call(), mfctr(3), sc, mtctr(3)];
    assert_eq!(
        sites(vec![copy_stub(&between)]),
        [(SC, None), (QUIET_SC, None)]
    );
}

/// The built address overwrites the copy in r3, so CTR at the `bctr` is
/// the window's `blr`.
///
/// [PPC-Book1 p:51 s:3.3.8] `addis` places the sum (RA|0) + (SI || 0x0000) into RT.
#[test]
fn a_built_address_over_a_copy_of_ctr_after_a_call_keeps_each_site_resolved() {
    let mut between = vec![call(), mfctr(3)];
    between.extend(build(0x300C));
    between.push(mtctr(3));
    assert_eq!(
        sites(vec![copy_stub(&between)]),
        [(SC, Some(75)), (QUIET_SC, Some(77))]
    );
}

/// The fall-through path builds an address over the copy in r3, but the
/// taken path of the `blt` brings the copy to the `mtctr`.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if CR bit BI is 1.
#[test]
fn a_copy_of_ctr_that_a_branch_brings_past_a_built_address_leaves_every_site_unresolved() {
    let mut between = vec![call(), mfctr(3), bc(0b01100, 16)];
    between.extend(build(0x300C));
    between.push(mtctr(3));
    assert_eq!(
        sites(vec![copy_stub(&between)]),
        [(SC, None), (QUIET_SC, None)]
    );
}

/// A real-mode stub writes the real address 0x5000 into CTR, then copies
/// CTR into r4 and back. A later path, at the next interrupt vector,
/// writes the real address 0x3010 into CTR, copies CTR into r4 and
/// branches back to the same `mtctr r4`. The `bctr` thus reaches both
/// addresses. 0x3010 is the path past the window's `blr`.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
/// [CBE-Handbook p:245 s:9.5] Table 9-3 puts the machine check vector at 0x200.
#[test]
fn a_bctr_after_a_copy_of_ctr_that_a_later_branch_widens_reaches_each_written_address() {
    // From the `b` at VECTOR + 0x10C to the `mtctr r4` at VECTOR + 0xC.
    let back = word(PpuInstruction::B {
        offset: -0x100,
        aa: false,
        link: false,
    });
    let words = [li(3, 0x5000), mtctr(3), mfctr(4), mtctr(4), bctr()];
    let later = [li(3, 0x3010), mtctr(3), mfctr(4), back];
    let segments = [
        code(VECTOR, &words),
        code(VECTOR + 0x100, &later),
        window(&into_window()),
        quiet(),
    ];
    assert_eq!(ordinals(0, &segments), [(SC, None), (QUIET_SC, Some(77))]);
}

/// The `bdnz` falls through only when CTR is 0, so the `bctr` reaches
/// address 0 through the copy in r3.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
#[test]
fn a_bctr_through_a_copy_of_ctr_after_a_bdnz_falls_through_enters_address_zero() {
    assert_eq!(
        after_bdnz_falls_through(&[mfctr(3), mtctr(3)]),
        [(4, None), (QUIET_SC, Some(77))]
    );
}

/// `ori 0,0,0` keeps 0 in the copy in r0.
///
/// [PPC-Book1 p:66 s:3.3.11] The preferred no-op is `ori 0,0,0`.
#[test]
fn a_no_op_on_a_copy_of_ctr_after_a_bdnz_falls_through_enters_address_zero() {
    assert_eq!(
        after_bdnz_falls_through(&[mfctr(0), NOP, mtctr(0)]),
        [(4, None), (QUIET_SC, Some(77))]
    );
}

/// A loop can run the `addi` more than once, so the copy in r3 can hold
/// any value.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
#[test]
fn an_addi_on_a_copy_of_ctr_leaves_every_site_unresolved() {
    let addi = word(PpuInstruction::Addi {
        rt: 3,
        ra: 3,
        imm: 4,
    });
    assert_eq!(
        after_bdnz_falls_through(&[mfctr(3), addi, mtctr(3)]),
        [(4, None), (QUIET_SC, None)]
    );
}

/// With a byte count of 0 in XER, the `lswx` leaves r3 undefined, so r3
/// can still hold the copy.
///
/// [PPC-Book1 p:48 s:3.3.6] `lswx` loads the number of bytes that XER\[57:63\] gives, and with a count of 0 the contents of RT are undefined.
#[test]
fn a_copy_of_ctr_after_a_call_that_crosses_an_lswx_leaves_every_site_unresolved() {
    let lswx = word(PpuInstruction::Lswx {
        rt: 3,
        ra: 4,
        rb: 5,
    });
    let between = [call(), mfctr(3), lswx, mtctr(3)];
    assert_eq!(
        sites(vec![copy_stub(&between)]),
        [(SC, None), (QUIET_SC, None)]
    );
}

/// With a divisor of 0 in r4, each divide leaves r3 undefined, so r3 can
/// still hold the copy.
///
/// [PPC-Book1 p:58 s:3.3.8] For `divd` and `divw`, a divisor of 0 leaves the contents of RT undefined.
/// [PPC-Book1 p:59 s:3.3.8] For `divdu` and `divwu`, a divisor of 0 leaves the contents of RT undefined.
#[test]
fn a_copy_of_ctr_after_a_call_that_crosses_a_divide_by_zero_leaves_every_site_unresolved() {
    let (rt, ra, rb, oe, rc) = (3, 3, 4, false, false);
    let divides = [
        ("divw", PpuInstruction::Divw { rt, ra, rb, oe, rc }),
        ("divwu", PpuInstruction::Divwu { rt, ra, rb, oe, rc }),
        ("divd", PpuInstruction::Divd { rt, ra, rb, oe, rc }),
        ("divdu", PpuInstruction::Divdu { rt, ra, rb, oe, rc }),
    ];
    for (name, divide) in divides {
        let between = [call(), mfctr(3), li(4, 0), word(divide), mtctr(3)];
        assert_eq!(
            sites(vec![copy_stub(&between)]),
            [(SC, None), (QUIET_SC, None)],
            "{name}"
        );
    }
}

/// `lq r3,0(r3)`, an `lq` with RT = RA.
const ILLEGAL_LQ_R3: u32 = 0xE063_0000;

/// The error handler of the illegal `lq` need not emulate it, so r3 can
/// still hold the copy.
///
/// [PPC-Book3 p:92 s:8.4] An `lq` with RT = RA invokes the illegal instruction error handler.
#[test]
fn a_copy_of_ctr_after_a_call_that_crosses_an_illegal_word_leaves_every_site_unresolved() {
    assert!(matches!(
        crate::decode::decode(ILLEGAL_LQ_R3),
        Ok(PpuInstruction::PpeIllegal { .. })
    ));
    let between = [call(), mfctr(3), ILLEGAL_LQ_R3, mtctr(3)];
    assert_eq!(
        sites(vec![copy_stub(&between)]),
        [(SC, None), (QUIET_SC, None)]
    );
}
