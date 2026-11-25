use super::hypercall_ctr_decrement_tests::VECTOR;
use super::hypercall_entry_tests::{code, data_bytes, ordinals, pointers, word, Segment};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;

/// A word outside the interrupt vectors, where no real-mode entry starts.
pub(super) const CODE: u64 = KERNEL + 0x1_0000;

/// A table entry that reaches no word of the code, which ends the run.
pub(super) const END: u32 = 0x0010_0000;

/// `li r11,74; nop; sc 1` at `at`. Its `sc` is at `at + 8`.
pub(super) fn window(at: u64) -> Segment {
    code(at, &[li(11, 74), NOP, PPC_SC_1])
}

/// A relative jump table at `at` with one entry, which `offset` gives,
/// and an [`END`] entry.
fn table(at: u64, offset: u32) -> Segment {
    let bytes = [offset, END].into_iter().flat_map(u32::to_be_bytes);
    data_bytes(at, bytes.collect())
}

/// `build` puts the table's base in r3. Then `lwz r4,0(r3);
/// add r4,r4,r3; mtctr r4; bctr` branches through the table's first
/// entry.
pub(super) fn branch_through_table(at: u64, build: &[u32]) -> Segment {
    let mut words = build.to_vec();
    words.extend([
        word(PpuInstruction::Lwz {
            rt: 4,
            ra: 3,
            imm: 0,
        }),
        word(PpuInstruction::Add {
            rt: 4,
            ra: 4,
            rb: 3,
            oe: false,
            rc: false,
        }),
        word(PpuInstruction::Mtctr { rs: 4 }),
        word(PpuInstruction::Bcctr {
            bo: 0b10100,
            bi: 0,
            link: false,
        }),
    ]);
    code(at, &words)
}

/// `lis r3,0; ori r3,r3,low`.
pub(super) fn build_low(low: u16) -> [u32; 2] {
    [
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
    ]
}

/// `li r3,1; sldi r3,r3,32; ori r3,r3,0x4000`, which builds
/// 0x1_0000_4000.
fn build_above_4_gib() -> [u32; 3] {
    [
        li(3, 1),
        word(PpuInstruction::Rldicr {
            ra: 3,
            rs: 3,
            sh: 32,
            me: 31,
            rc: false,
        }),
        word(PpuInstruction::Ori {
            ra: 3,
            rs: 3,
            imm: 0x4000,
        }),
    ]
}

/// Real-mode code builds the base 0x4000, and the file bytes hold the
/// table at KERNEL + 0x4000. The entry reaches the real address of the
/// window's `li r11` or of its `nop`, with the window at KERNEL + 0x5000
/// or at 0x5000.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
/// [PPC-Book3 p:62 s:5.4] An interrupt sets MSR bits IR and DR to 0.
#[test]
fn a_jump_table_at_the_real_alias_of_its_base_enters_its_targets() {
    for at in [KERNEL + 0x5000, 0x5000] {
        let reached = |offset: u32| {
            let segments = [
                branch_through_table(VECTOR, &build_low(0x4000)),
                table(KERNEL + 0x4000, offset),
                window(at),
            ];
            ordinals(0, &segments)
        };
        assert_eq!(reached(0x1000), [(at + 8, Some(74))], "{at:#x}");
        assert_eq!(reached(0x1004), [(at + 8, None)], "{at:#x}");
    }
}

/// The code builds 0x1_0000_4000, and the file bytes hold the table only
/// at KERNEL + 0x1_0000_4000, so the low-order 32 bits name no table.
/// The window is at KERNEL + 0x1_0000_5000 or at 0x1_0000_5000, whose
/// low-order 32 bits name no word. A code pointer names the code.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_jump_table_at_the_real_alias_of_a_base_above_4_gib_enters_its_targets() {
    for at in [KERNEL + 0x1_0000_5000, 0x1_0000_5000] {
        let reached = |offset: u32| {
            let segments = [
                branch_through_table(CODE, &build_above_4_gib()),
                table(KERNEL + 0x1_0000_4000, offset),
                window(at),
                pointers(&[CODE]),
            ];
            ordinals(0, &segments)
        };
        assert_eq!(reached(0x1000), [(at + 8, Some(74))], "{at:#x}");
        assert_eq!(reached(0x1004), [(at + 8, None)], "{at:#x}");
    }
}

/// The code builds 0x1_0000_4000, and the file bytes hold the table only
/// at KERNEL + 0x4000: the real address of the low-order 32 bits. A code
/// pointer names the code.
///
/// [PPC-Book1 p:15 s:1.12.2] In 32-bit mode the high-order 32 bits of a computed effective address are set to 0 for a data access.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
#[test]
fn a_jump_table_at_the_real_alias_of_the_low_word_of_its_base_enters_its_targets() {
    let at = KERNEL + 0x5000;
    let reached = |offset: u32| {
        let segments = [
            branch_through_table(CODE, &build_above_4_gib()),
            table(KERNEL + 0x4000, offset),
            window(at),
            pointers(&[CODE]),
        ];
        ordinals(0, &segments)
    };
    assert_eq!(reached(0x1000), [(at + 8, Some(74))]);
    assert_eq!(reached(0x1004), [(at + 8, None)]);
}

/// The code builds 0xFFFF_FFFF_FFFF_F000, and the file bytes hold the
/// table at 0xFFFF_F000. The table's start plus its entry is
/// 0x1_0000_5000 or 0x1_0000_5004, whose low-order 32 bits are the
/// window's `li r11` or its `nop`. A code pointer names the code.
///
/// [PPC-Book1 p:15 s:1.12.2] In 32-bit mode the high-order 32 bits of a computed effective address are set to 0 for a data access.
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
#[test]
fn a_jump_table_at_the_low_word_of_its_base_enters_the_low_word_of_each_target() {
    let at = 0x5000;
    let reached = |offset: u32| {
        let segments = [
            branch_through_table(CODE, &[li(3, -0x1000)]),
            table(0xFFFF_F000, offset),
            window(at),
            pointers(&[CODE]),
        ];
        ordinals(0, &segments)
    };
    assert_eq!(reached(0x6000), [(at + 8, Some(74))]);
    assert_eq!(reached(0x6004), [(at + 8, None)]);
}
