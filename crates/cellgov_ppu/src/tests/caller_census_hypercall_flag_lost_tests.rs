use super::hypercall_constant_join_tests::{
    addi, bctr, build, mtctr, sites, twenty_pass_loop, EVERY_UNRESOLVED, NOWHERE, WINDOW_RESOLVED,
};
use super::hypercall_ctr_copy_tests::mfctr;
use super::hypercall_ctr_decrement_tests::VECTOR;
use super::hypercall_entry_tests::word;
use super::hypercall_skip_return_tests::mtlr;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

use PpuInstruction as I;

/// The loop that marks r3 lost, then `tail`.
fn after_loop(tail: &[u32]) -> Vec<u32> {
    let mut words = twenty_pass_loop();
    words.extend_from_slice(tail);
    words
}

/// `addc rt,ra,rb`.
fn addc(rt: u8, ra: u8, rb: u8) -> u32 {
    word(I::Addc {
        rt,
        ra,
        rb,
        oe: false,
        rc: false,
    })
}

/// `adde rt,ra,rb`.
fn adde(rt: u8, ra: u8, rb: u8) -> u32 {
    word(I::Adde {
        rt,
        ra,
        rb,
        oe: false,
        rc: false,
    })
}

/// `addze rt,ra`.
fn addze(rt: u8, ra: u8) -> u32 {
    word(I::Addze {
        rt,
        ra,
        oe: false,
        rc: false,
    })
}

/// `add rt,ra,rb`, with OE = `oe` and Rc = `rc`.
fn add(rt: u8, ra: u8, rb: u8, oe: bool, rc: bool) -> u32 {
    word(I::Add { rt, ra, rb, oe, rc })
}

/// 64 rounds of `addc r0,r3,r3; adde r4,r4,r4; add r3,r3,r3`, which
/// shift each bit of r3 out through CA and into r4, so r4 ends as r3.
///
/// [PPC-Book1 p:53 s:3.3.8] `addc` places (RA) + (RB) into RT and sets CA.
/// [PPC-Book1 p:54 s:3.3.8] `adde` places (RA) + (RB) + CA into RT.
fn carry_chain() -> Vec<u32> {
    (0..64)
        .flat_map(|_| [addc(0, 3, 3), adde(4, 4, 4), add(3, 3, 3, false, false)])
        .collect()
}

/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_branch_to_a_lost_gpr_rebuilt_through_a_carry_chain_leaves_every_site_unresolved() {
    for tail in [[mtctr(4), bctr()], [mtlr(4), PPC_BLR]] {
        let mut words = after_loop(&carry_chain());
        words.extend(tail);
        assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED, "{tail:08x?}");
    }
}

/// Each word that sets CA from the lost r3, and writes r0.
///
/// [PPC-Book1 p:51 s:3.3.8] `addic`, `addic.`, `subfic`, `addc`, `subfc`, `adde`, `subfe`, `addme`, `subfme`, `addze` and `subfze` always set CA from the carry of the result.
/// [PPC-Book1 p:79 s:3.3.12.2] `sradi` and `srawi` set CA from RS and the bits that they shift out.
/// [PPC-Book1 p:80 s:3.3.12.2] `srad` and `sraw` set CA from RS and the bits that they shift out.
fn ca_writers() -> Vec<u32> {
    vec![
        addc(0, 3, 3),
        word(I::Addic {
            rt: 0,
            ra: 3,
            imm: 1,
        }),
        word(I::AddicDot {
            rt: 0,
            ra: 3,
            imm: 1,
        }),
        word(I::Subfic {
            rt: 0,
            ra: 3,
            imm: 1,
        }),
        word(I::Subfc {
            rt: 0,
            ra: 3,
            rb: 5,
            oe: false,
            rc: false,
        }),
        word(I::Sraw {
            ra: 0,
            rs: 3,
            rb: 5,
            rc: false,
        }),
        word(I::Srawi {
            ra: 0,
            rs: 3,
            sh: 1,
            rc: false,
        }),
        word(I::Srad {
            ra: 0,
            rs: 3,
            rb: 5,
            rc: false,
        }),
        word(I::Sradi {
            ra: 0,
            rs: 3,
            sh: 1,
            rc: false,
        }),
        adde(0, 3, 5),
        word(I::Subfe {
            rt: 0,
            ra: 3,
            rb: 5,
            oe: false,
            rc: false,
        }),
        addze(0, 3),
        word(I::Addme {
            rt: 0,
            ra: 3,
            oe: false,
            rc: false,
        }),
        word(I::Subfze {
            rt: 0,
            ra: 3,
            oe: false,
            rc: false,
        }),
        word(I::Subfme {
            rt: 0,
            ra: 3,
            oe: false,
            rc: false,
        }),
    ]
}

/// Each word that puts CA into r4, with r5 as its GPR source.
///
/// [PPC-Book1 p:54 s:3.3.8] `adde`, `subfe`, `addme` and `subfme` add CA into the sum that they place into RT.
/// [PPC-Book1 p:55 s:3.3.8] `addze` and `subfze` add CA into the sum that they place into RT.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` of XER places XER into RT.
fn ca_readers() -> Vec<u32> {
    vec![
        adde(4, 5, 5),
        word(I::Subfe {
            rt: 4,
            ra: 5,
            rb: 5,
            oe: false,
            rc: false,
        }),
        word(I::Addme {
            rt: 4,
            ra: 5,
            oe: false,
            rc: false,
        }),
        word(I::Subfme {
            rt: 4,
            ra: 5,
            oe: false,
            rc: false,
        }),
        addze(4, 5),
        word(I::Subfze {
            rt: 4,
            ra: 5,
            oe: false,
            rc: false,
        }),
        word(I::Mfxer { rt: 4 }),
    ]
}

/// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr` copy RS into LR or CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_carry_set_from_a_lost_gpr_leaves_every_site_unresolved() {
    let writers = ca_writers();
    assert_eq!(writers.len(), 15);
    for writer in writers {
        let words = after_loop(&[writer, addze(4, 5), mtctr(4), bctr()]);
        assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED, "{writer:08x}");
    }
    let readers = ca_readers();
    assert_eq!(readers.len(), 7);
    for reader in readers {
        let words = after_loop(&[addc(0, 3, 3), reader, mtctr(4), bctr()]);
        assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED, "{reader:08x}");
    }
}

/// Each word that sets a CR field from the lost r3, and writes no GPR or
/// writes r0.
///
/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB.
/// [PPC-Book1 p:18 s:2.3.1] A fixed-point instruction with Rc = 1, and `addic.`, `andi.` and `andis.`, set CR0 by a compare of the result with 0.
/// [PPC-Book2 p:25 s:3.3] `stwcx.` and `stdcx.` set CR0 EQ from whether the reservation address is the real address of EA = (RA|0) + (RB).
fn cr_writers() -> Vec<u32> {
    vec![
        word(I::Cmpd {
            bf: 0,
            ra: 3,
            rb: 5,
        }),
        word(I::Cmplw {
            bf: 7,
            ra: 5,
            rb: 3,
        }),
        word(I::Cmpdi {
            bf: 1,
            ra: 3,
            imm: 0,
        }),
        word(I::Cmpwi {
            bf: 6,
            ra: 3,
            imm: 0,
        }),
        add(0, 3, 3, false, true),
        word(I::And {
            ra: 0,
            rs: 3,
            rb: 3,
            rc: true,
        }),
        word(I::Or {
            ra: 0,
            rs: 3,
            rb: 3,
            rc: true,
        }),
        word(I::AndiDot {
            ra: 0,
            rs: 3,
            imm: 1,
        }),
        word(I::Rlwinm {
            ra: 0,
            rs: 3,
            sh: 1,
            mb: 0,
            me: 31,
            rc: true,
        }),
        word(I::Rldicl {
            ra: 0,
            rs: 3,
            sh: 0,
            mb: 0,
            rc: true,
        }),
        word(I::Stwcx {
            rs: 5,
            ra: 0,
            rb: 3,
        }),
    ]
}

/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT.
/// [PPC-Book1 p:124 s:5.1.1] `mfocrf` moves one CR field from CR into a GPR.
/// [PPC-Book1 p:32 s:3.2.2] An XO-form add with OE = 1 sets XER\[OV\] from the carries of its sum.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` of XER places XER into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_cr_or_xer_read_after_a_compare_or_record_of_a_lost_gpr_leaves_every_site_unresolved() {
    let writers = cr_writers();
    assert_eq!(writers.len(), 11);
    for writer in writers {
        let words = after_loop(&[writer, word(I::Mfcr { rt: 4 }), mtctr(4), bctr()]);
        assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED, "{writer:08x}");
    }
    let mfocrf = word(I::Mfocrf { rt: 4, crm: 0x80 });
    let mfxer = word(I::Mfxer { rt: 4 });
    for [writer, reader] in [
        [cr_writers()[0], mfocrf],
        [add(0, 3, 3, true, false), mfxer],
    ] {
        let words = after_loop(&[writer, reader, mtctr(4), bctr()]);
        assert_eq!(
            sites(VECTOR, &words),
            EVERY_UNRESOLVED,
            "{writer:08x} {reader:08x}"
        );
    }
}

/// r5 holds the constant that the `addi` builds, and the lost r3 sets no
/// CA and no CR field. So CA, CR and r4 take no mark.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to 0 when RA is 0.
/// [PPC-Book1 p:53 s:3.3.8] `addc` places (RA) + (RB) into RT and sets CA.
/// [PPC-Book1 p:54 s:3.3.8] `adde` places (RA) + (RB) + CA into RT.
/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB.
/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_a_known_gpr_through_a_carry_or_a_compare_after_the_loop_keeps_the_sites_resolved() {
    let known = addi(5, 0, NOWHERE.cast_signed());
    let compare = word(I::Cmpd {
        bf: 0,
        ra: 5,
        rb: 5,
    });
    for pair in [
        [addc(0, 5, 5), adde(4, 5, 5)],
        [compare, word(I::Mfcr { rt: 4 })],
    ] {
        let words = after_loop(&[known, pair[0], pair[1], mtctr(4), bctr()]);
        assert_eq!(sites(VECTOR, &words), WINDOW_RESOLVED, "{pair:08x?}");
    }
}

/// r4 holds a copy of the CTR constant 0x5000, so the constant scan marks
/// no GPR. The compare or the carry puts a part of the copy into the
/// cell of its CR field or of XER\[CA\], and the copy that comes back out
/// is one that the scan cannot prove.
///
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
/// [PPC-Book1 p:60 s:3.3.9] A compare sets the CR field BF from a comparison of RA with SI, UI or RB.
/// [PPC-Book1 p:53 s:3.3.8] `addc` places (RA) + (RB) into RT and sets CA.
/// [PPC-Book1 p:83 s:3.3.13] `mfcr` places CR into RT.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_bctr_of_cr_or_xer_read_after_a_compare_or_carry_of_a_known_ctr_copy_leaves_every_site_unresolved(
) {
    let compare = word(I::Cmpd {
        bf: 0,
        ra: 4,
        rb: 4,
    });
    for pair in [
        [compare, word(I::Mfcr { rt: 5 })],
        [addc(0, 4, 4), word(I::Mfxer { rt: 5 })],
    ] {
        let mut words = build(3, NOWHERE).to_vec();
        words.extend([mtctr(3), mfctr(4), pair[0], pair[1], mtctr(5), bctr()]);
        assert_eq!(sites(VECTOR, &words), EVERY_UNRESOLVED, "{pair:08x?}");
    }
}
