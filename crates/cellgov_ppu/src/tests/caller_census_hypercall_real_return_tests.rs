use super::hypercall_ctr_decrement_tests::{bc, quiet, window, QUIET_SC, SC, VECTOR};
use super::hypercall_entry_tests::{code, data_bytes, ordinals, word, Segment};
use super::hypercall_real_mode_switch_tests::{lis_sf, ori, shift_left};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};

/// The address of a data segment that holds one 32-bit word.
const DATA: u64 = 0x2000_0000;

/// The real address of the window's `nop`, between its `li r11` and `sc`.
const BETWEEN: u32 = 0x3004;

/// A real address with no word of the code at it.
const NOWHERE: i16 = 0x5000;

/// `sc` with LEV = 0, which is not a hypercall site.
///
/// [PPC-Book1 p:26 s:2.4.2] sc LEV-field encoding.
const SC_0: u32 = 0x4400_0002;

/// A word that does not decode.
const UNDECODED: u32 = 0;

/// `b` with a byte displacement of `offset`, which writes LR when `link`
/// is set.
fn b(offset: i32, link: bool) -> u32 {
    word(PpuInstruction::B {
        offset,
        aa: false,
        link,
    })
}

/// `tw 31,r0,r0`, which traps for all operand values.
///
/// [PPC-Book1 p:62 s:3.3.10] TO bits 0 to 4 select signed less than, signed greater than, equal, unsigned less than and unsigned greater than.
fn trap() -> u32 {
    word(PpuInstruction::Tw {
        to: 31,
        ra: 0,
        rb: 0,
    })
}

/// `bl` to the word two words on, then a trap: a call whose routine
/// starts at the third word, and a return point that no branch targets.
fn call() -> [u32; 2] {
    [b(8, true), trap()]
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

/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_real_mode_blr_that_returns_from_a_real_mode_call_enters_no_named_word() {
    let [bl, stay] = call();
    assert_eq!(sites(&[bl, stay, PPC_BLR]), RESOLVED);
}

/// [PPC-Book1 p:81 s:3.3.13] `mtlr` copies RS into LR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_real_mode_blr_after_an_mtlr_of_a_loaded_value_can_enter_a_named_real_address() {
    let [bl, stay] = call();
    let load = word(PpuInstruction::Ld {
        rt: 3,
        ra: 4,
        imm: 0,
    });
    let stub = [
        bl,
        stay,
        load,
        word(PpuInstruction::Mtlr { rs: 3 }),
        PPC_BLR,
    ];
    assert_eq!(sites(&stub), UNRESOLVED);
}

/// [PPC-Book3 p:62 s:5.4] An interrupt sets MSR bits IR and DR to 0 and MSR bit SF to 1.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
#[test]
fn a_real_mode_blr_at_an_interrupt_vector_can_enter_a_named_real_address() {
    assert_eq!(sites(&[PPC_BLR]), UNRESOLVED);
}

/// The `beq` skips to the call of the `blr`. The `b +12` takes the
/// vector's own LR to the `blr`, past the call and its return point.
///
/// [PPC-Book1 p:20 s:2.4.1] BO 011at branches if the CR bit is 1.
/// [PPC-Book3 p:62 s:5.4] An interrupt sets MSR bits IR and DR to 0 and MSR bit SF to 1.
#[test]
fn a_real_mode_blr_that_a_branch_reaches_with_the_lr_of_the_interrupt_can_enter_a_named_real_address(
) {
    let [bl, returned] = call();
    let stub = [bc(0b01100, 8), b(12, false), bl, returned, PPC_BLR];
    assert_eq!(sites(&stub), UNRESOLVED);
}

/// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt sets SRR0 and SRR1.
#[test]
fn a_real_mode_blr_after_an_sc_can_enter_a_named_real_address() {
    let [bl, stay] = call();
    assert_eq!(sites(&[bl, stay, SC_0, PPC_BLR]), UNRESOLVED);
}

/// [CBE-Handbook p:739 s:A.2.4.1] An attempt to execute `popcntb`, `mcrxr` or another word that the PPE does not implement causes an illegal instruction exception.
#[test]
fn a_real_mode_blr_after_a_word_that_does_not_decode_can_enter_a_named_real_address() {
    let [bl, stay] = call();
    assert_eq!(sites(&[bl, stay, UNDECODED, PPC_BLR]), UNRESOLVED);
}

/// The sites, with the vector's call and `blr` at [`VECTOR`] and `low`
/// at real address 0x100, where control also enters in real mode.
///
/// The call's return point is the vector's second word. Its real address
/// is 0x104, so a real-mode return can fetch the second word of `low`.
fn aliased_sites(low: Segment, more: Vec<Segment>) -> Vec<(u64, Option<u16>)> {
    let [bl, stay] = call();
    let mut segments = vec![code(VECTOR, &[bl, stay, PPC_BLR]), low];
    segments.extend(more);
    let mut sites = ordinals(0, &segments);
    sites.sort_unstable();
    sites
}

/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_real_mode_return_enters_the_word_at_the_real_address_of_its_return_point() {
    let low = code(0x100, &[li(11, 78), NOP, PPC_SC_1]);
    assert_eq!(aliased_sites(low, Vec::new()), [(0x108, None)]);
}

/// Entered at 0x100, the low code builds r3 itself, so its `blr`
/// reaches only [`NOWHERE`]. Entered at 0x104 by the vector's return,
/// r3 holds what the interrupt left, so the `blr` can reach any named
/// address in real mode.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtlr` copies RS into LR.
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_real_mode_return_enters_the_word_at_the_real_address_of_its_return_point_with_unknown_registers(
) {
    let low = code(
        0x100,
        &[
            li(3, NOWHERE),
            word(PpuInstruction::Mtlr { rs: 3 }),
            PPC_BLR,
        ],
    );
    let named = data_bytes(DATA, BETWEEN.to_be_bytes().to_vec());
    let more = vec![window(&[]), quiet(), named];
    assert_eq!(aliased_sites(low, more), UNRESOLVED);
}

/// [CBE-Handbook p:738 s:A.2.4.1] The PPE does not implement `popcntb`.
/// [CBE-Handbook p:739 s:A.2.4.1] An attempt to execute `popcntb`, `mcrxr` or another word that the PPE does not implement causes an illegal instruction exception.
#[test]
fn a_real_mode_blr_after_a_popcntb_can_enter_a_named_real_address() {
    let [bl, stay] = call();
    let popcntb = word(PpuInstruction::Popcntb { ra: 3, rs: 4 });
    assert_eq!(sites(&[bl, stay, popcntb, PPC_BLR]), UNRESOLVED);
}

/// `mfsr r3,0`: primary opcode 31 with extended opcode 595.
const MFSR: u32 = 0x7C60_04A6;

/// [CBE-Handbook p:739 s:A.2.4.3] The PPE does not implement `mfsr`.
#[test]
fn a_real_mode_blr_after_a_word_that_the_ppe_does_not_implement_can_enter_a_named_real_address() {
    assert!(matches!(
        crate::decode::decode(MFSR),
        Ok(PpuInstruction::PpeIllegal { .. })
    ));
    let [bl, stay] = call();
    assert_eq!(sites(&[bl, stay, MFSR, PPC_BLR]), UNRESOLVED);
}

/// The kernel address of code that runs only outside real mode: it
/// builds the kernel address of the vector's routine, `KERNEL + 0x108`,
/// and runs `mtctr r3; bctr`.
const WRITER: u64 = KERNEL + 0x6000;

/// The address of a data segment that names [`WRITER`].
const WRITER_POINTER: u64 = 0x2100_0000;

/// The vector calls its routine at `KERNEL + 0x108`, a target of the
/// writer's closed `bctr`. The walk does not reach the writer in real
/// mode, but a `bctr` like it can enter the routine in real mode with
/// any LR. So the routine's `blr` can reach any named real address.
///
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book1 p:25 s:2.4] The registers that `bclr` and `bcctr` alter are CTR and LR, so the MSR at the target is the MSR at the branch.
#[test]
fn a_real_mode_blr_at_a_target_of_a_writer_outside_real_mode_can_enter_a_named_real_address() {
    let [bl, stay] = call();
    let writer = code(
        WRITER,
        &[
            lis_sf(3),
            shift_left(3, 32),
            ori(3, 0x108),
            word(PpuInstruction::Mtctr { rs: 3 }),
            word(PpuInstruction::Bcctr {
                bo: 0b10100,
                bi: 0,
                link: false,
            }),
        ],
    );
    let named = data_bytes(DATA, BETWEEN.to_be_bytes().to_vec());
    let pointer = data_bytes(WRITER_POINTER, WRITER.to_be_bytes().to_vec());
    let segments = [
        code(VECTOR, &[bl, stay, PPC_BLR]),
        window(&[]),
        quiet(),
        named,
        writer,
        pointer,
    ];
    assert_eq!(ordinals(0, &segments), UNRESOLVED);
}
