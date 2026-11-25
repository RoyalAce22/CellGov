use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_entry_tests::{code, data_bytes, ordinals, word};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

/// The address of a data segment that holds one 32-bit word.
const DATA: u64 = 0x2000_0000;

/// The real address of the window's `nop`, between its `li r11` and `sc`.
const BETWEEN: u32 = 0x3004;

/// A real address with no word of the code at it.
const NOWHERE: u16 = 0x5000;

/// `lis r3,0; ori r3,r3,NOWHERE`.
fn build() -> [u32; 2] {
    [
        word(PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: 0,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: NOWHERE,
        }),
    ]
}

/// `ld r3,0(r4)`.
fn load() -> u32 {
    word(PpuInstruction::Ld {
        rt: 3,
        ra: 4,
        imm: 0,
    })
}

fn bctr() -> u32 {
    word(PpuInstruction::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    })
}

/// The ordinals when a stub at [`VECTOR`], where control enters in real
/// mode, runs `stub`. A word of the file bytes names [`BETWEEN`], so a
/// `bcctr` or `bclr` in real mode with a target that the scan does not
/// hold can enter the window's `nop`.
fn sites(stub: &[u32]) -> Vec<(u64, Option<u16>)> {
    let named = data_bytes(DATA, BETWEEN.to_be_bytes().to_vec());
    ordinals(0, &[code(VECTOR, stub), window(&[]), quiet(), named])
}

const RESOLVED: [(u64, Option<u16>); 2] = [(SC, Some(75)), (QUIET_SC, Some(77))];

const UNRESOLVED: [(u64, Option<u16>); 2] = [(SC, None), (QUIET_SC, Some(77))];

/// [PPC-Book1 p:81 s:3.3.13] `mtlr` copies RS into LR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_real_mode_blr_to_a_built_address_enters_no_named_word() {
    let [lis, ori] = build();
    let stub = [lis, ori, word(PpuInstruction::Mtlr { rs: 3 }), PPC_BLR];
    assert_eq!(sites(&stub), RESOLVED);
}

/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_real_mode_blr_to_a_loaded_address_can_enter_a_named_real_address() {
    let stub = [load(), word(PpuInstruction::Mtlr { rs: 3 }), PPC_BLR];
    assert_eq!(sites(&stub), UNRESOLVED);
}

/// [PPC-Book1 p:81 s:3.3.13] `mtctr` copies RS into CTR.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_real_mode_bctr_to_a_built_address_enters_no_named_word() {
    let [lis, ori] = build();
    let stub = [lis, ori, word(PpuInstruction::Mtctr { rs: 3 }), bctr()];
    assert_eq!(sites(&stub), RESOLVED);
}

/// The `blt` skips the build, so the `mtctr` copies the loaded address on
/// one path and the built one on the other.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1.
/// [PPC-Book1 p:25 s:2.4] `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_real_mode_bctr_of_a_gpr_that_a_branch_brings_loaded_can_enter_a_named_real_address() {
    let [lis, ori] = build();
    let stub = [
        load(),
        bc(0b01100, 12),
        lis,
        ori,
        word(PpuInstruction::Mtctr { rs: 3 }),
        bctr(),
    ];
    assert_eq!(sites(&stub), UNRESOLVED);
}
