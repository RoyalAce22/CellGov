use super::hypercall_entry_tests::{code, ordinals};
use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use crate::hypercall_cfg::{Code, CodeSegment};
use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;

/// The ordinal of the site of `li r11,75; nop; sc 1` with the `li` at
/// kernel address `at`.
fn window_at(at: u64) -> Vec<(u64, Option<u16>)> {
    ordinals(0, &[code(at, &[li(11, 75), NOP, PPC_SC_1])])
}

/// [CBE-Handbook p:245 s:9.5] Table 9-3 puts the VXU unavailable vector at 0xF20 and lists no vector at 0xF24 or 0xF28.
#[test]
fn a_site_past_the_first_word_of_a_vector_resolves() {
    assert_eq!(window_at(KERNEL + 0xF20), [(KERNEL + 0xF28, Some(75))]);
}

/// [CBE-Handbook p:245 s:9.5] Table 9-3 puts the system error vector at 0x1200.
#[test]
fn a_vector_between_the_load_and_the_site_leaves_the_site_unresolved() {
    assert_eq!(window_at(KERNEL + 0x11FC), [(KERNEL + 0x1204, None)]);
}

/// [CBE-Handbook p:245 s:9.5] Table 9-3 lists no vector at 0x1FFC, and each vector that it does not list is reserved.
#[test]
fn a_site_in_the_implementation_specific_pages_off_a_vector_resolves() {
    assert_eq!(window_at(KERNEL + 0x1FF8), [(KERNEL + 0x2000, Some(75))]);
}

/// [CBE-Handbook p:245 s:9.5] Table 9-3 lists the PPE interrupt vectors, and the PPE does not implement the vector at 0xF00.
/// [CBE-Handbook p:246 s:9.5.1] The system reset vector of thread 1 is fixed at 0x100.
#[test]
fn the_real_mode_entries_of_the_vector_pages_are_the_vectors_of_the_ppe() {
    let words = vec![NOP; 0x3000 / 4];
    let code = Code::new(vec![CodeSegment {
        vaddr: KERNEL,
        words,
    }]);
    let vectors = [
        0x100, 0x200, 0x300, 0x380, 0x400, 0x480, 0x500, 0x600, 0x700, 0x800, 0x900, 0x980, 0xC00,
        0xD00, 0xF20, 0x1200, 0x1600, 0x1800,
    ];
    assert_eq!(
        real_mode_entries(&[], &code),
        vectors
            .iter()
            .map(|&vector| KERNEL + vector)
            .collect::<BTreeSet<u64>>()
    );
}
