use super::hypercall_constant_join_tests::{addi, mflr, EVERY_UNRESOLVED, WINDOW_RESOLVED};
use super::hypercall_ctr_decrement_tests::{quiet, window, VECTOR};
use super::hypercall_entry_tests::{code, ordinals, pointers, word, Segment};
use super::hypercall_tests::KERNEL;
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The epilogue `ld r0,0(r3); mtlr r0; blr`, with its `mtlr` at 0x6E6C
/// past the kernel base.
const EPILOGUE: u64 = KERNEL + 0x6E68;

/// A zero word, then data whose words decode as instructions. No value
/// of the file bytes names it unless a case adds a pointer.
const DATA: u64 = KERNEL + 0x6100;

/// The ASCII bytes "Conn", which decode as `bca 27,15,0x6e6c`.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 1a01t decrements CTR and branches if CTR is then 0.
/// [PPC-Book1 p:24 s:2.4] `bc` with AA = 1 reaches EXTS(displacement).
const CONN: u32 = 0x436F_6E6E;

/// The ordinals with `words` at `at`, `extra`, the window, the second
/// window and `data`. A `b` at [`VECTOR`] goes to `at`.
///
/// An interrupt can enter each word of the interrupt vectors with any
/// value in each GPR, so the cases run past the vectors.
fn sites(at: u64, words: &[u32], extra: Option<Segment>, data: Segment) -> Vec<(u64, Option<u16>)> {
    let jump = word(PpuInstruction::B {
        offset: i32::try_from(at - VECTOR).expect("a short jump"),
        aa: false,
        link: false,
    });
    let mut segments = vec![code(VECTOR, &[jump]), code(at, words)];
    segments.extend(extra);
    segments.extend([window(&[]), quiet(), data]);
    ordinals(0, &segments)
}

/// `ld rt,0(r3)`.
///
/// [PPC-Book1 p:39 s:3.3.2] `ld` loads the doubleword at EA into RT.
fn load(rt: u8) -> u32 {
    word(PpuInstruction::Ld { rt, ra: 3, imm: 0 })
}

/// The ordinals with the epilogue, the data `0; mflr r9; addi r0,r9,8;
/// "Conn"` at [`DATA`], and a pointer to each address of `named`.
fn data_sites(named: &[u64]) -> Vec<(u64, Option<u16>)> {
    let epilogue = [load(0), word(PpuInstruction::Mtlr { rs: 0 }), PPC_BLR];
    let data = code(DATA, &[0, mflr(9), addi(0, 9, 8), CONN]);
    sites(EPILOGUE, &epilogue, Some(data), pointers(named))
}

/// The image names the zero word and the data after it, as a pointer to
/// a string does. The data is an entry, so control can run its words.
/// Its `addi` steps a copy of an LR that the census does not know, so r0
/// is lost. In real mode the `bca` enters the `mtlr` of the epilogue, so
/// the `blr` can reach any word.
///
/// The shape of [OFW-1.02 lv2_kernel a:0x80000000002f4500-0x80000000002f4540] and [OFW-1.02 lv2_kernel a:0x8000000000006e64-0x8000000000006e70]: a string word "Conn" reaches the `mtlr r0` of an epilogue.
/// [PPC-Book1 p:82 s:3.3.13] `mflr`, a form of `mfspr`, copies LR into RT.
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) to RA.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
/// [PPC-Book3 p:30 s:4.2.6.2] The offset real mode mechanism compares EA bits 2:63 with the RMLR limit and ORs them with the RMOR offset.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn an_absolute_bc_in_named_data_brings_a_lost_r0_to_the_mtlr_at_its_real_alias() {
    assert_eq!(
        crate::decode::decode(CONN),
        Ok(PpuInstruction::Bc {
            bo: 27,
            bi: 15,
            offset: 0x6E6C,
            aa: true,
            link: false,
        })
    );
    assert_eq!(data_sites(&[]), WINDOW_RESOLVED);
    assert_eq!(data_sites(&[DATA, DATA + 4]), EVERY_UNRESOLVED);
}
