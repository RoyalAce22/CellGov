use super::hypercall_entry_tests::{code, data_bytes, ordinals, word};
use super::hypercall_tests::{b, li, KERNEL, NOP};
use super::*;
use cellgov_ps3_abi::hw::ppc_isa::{PPC_BLR, PPC_SC_1};

/// The address of the code: above the real pages of the interrupt
/// vectors.
const CODE: u64 = KERNEL + 0x1_0000;

/// The address of the data segment that holds the descriptors.
const DATA: u64 = KERNEL + 0x2_0000;

/// The TOC of the first function. No file byte lies at it.
const TOC: u64 = DATA + 0x8000;

/// The address of the site's `li r11`.
const LOAD: u64 = CODE + 24;

/// The bytes of a function descriptor for the word at `entry` with the
/// TOC `toc`.
fn descriptor(entry: u64, toc: u64) -> Vec<u8> {
    [entry, toc, 0]
        .iter()
        .flat_map(|doubleword| doubleword.to_be_bytes())
        .collect()
}

/// The words `addis r9,r2,..; addi r9,r9,..`, which build `target` in r9
/// when r2 holds `toc`.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` adds EXTS(SI) and `addis` adds EXTS(SI || 0x0000).
fn build_from_toc(target: u64, toc: u64) -> [u32; 2] {
    let offset = target.wrapping_sub(toc).cast_signed();
    let low = offset as i16;
    let high = i16::try_from((offset - i64::from(low)) >> 16).expect("the offset fits addis");
    [
        word(PpuInstruction::Addis {
            rt: 9,
            ra: 2,
            imm: high,
        }),
        word(PpuInstruction::Addi {
            rt: 9,
            ra: 9,
            imm: low,
        }),
    ]
}

#[test]
fn a_site_that_an_address_from_the_second_toc_of_a_shared_word_enters_stays_unresolved() {
    // Words 0 and 1 are functions, and both branch to word 3. From the
    // first TOC, word 3 builds the address of the `li r11`. From a second
    // TOC one word above it, word 3 builds the address of the `nop`.
    let mut words = vec![b(12), b(8), NOP];
    words.extend(build_from_toc(LOAD, TOC));
    words.extend([PPC_BLR, li(11, 74), NOP, PPC_SC_1, PPC_BLR]);
    let site = |second_toc: u64| {
        let descriptors = [descriptor(CODE, TOC), descriptor(CODE + 4, second_toc)].concat();
        ordinals(0, &[code(CODE, &words), data_bytes(DATA, descriptors)])
    };
    assert_eq!(site(TOC), [(LOAD + 8, Some(74))]);
    assert_eq!(site(TOC + 4), [(LOAD + 8, None)]);
}
