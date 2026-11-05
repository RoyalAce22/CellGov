//! Each `PpuShadow` choice builds the shadow it names over the title's
//! executable code.

use cellgov_core::Runtime;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_time::Budget;

use super::super::firmware::MemoryPlacement;
use super::super::types::PpuShadow;
use super::build_instruction_shadow;
use crate::prx::TlsArea;
use crate::NullSink;

/// Where the title's text starts.
const TEXT: u64 = 0x1_0000;

/// `lwz r9, 0x40(0)` then `cmpwi cr7, r9, 0`: a pair the full shadow
/// fuses.
const PAIR: [u32; 2] = [0x8120_0040, 0x2F89_0000];

/// An ELF64 big-endian header with one executable PT_LOAD over the pair.
fn elf() -> Vec<u8> {
    const EHDR: usize = 64;
    const PHENT: usize = 56;
    let mut out = vec![0u8; EHDR + PHENT];
    out[..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    out[4] = 2; // ELFCLASS64
    out[5] = 2; // ELFDATA2MSB
    out[6] = 1; // EV_CURRENT
    out[18..20].copy_from_slice(&21u16.to_be_bytes()); // EM_PPC64
    out[32..40].copy_from_slice(&(EHDR as u64).to_be_bytes()); // e_phoff
    out[54..56].copy_from_slice(&(PHENT as u16).to_be_bytes()); // e_phentsize
    out[56..58].copy_from_slice(&1u16.to_be_bytes()); // e_phnum
    out[EHDR..EHDR + 4].copy_from_slice(&1u32.to_be_bytes()); // PT_LOAD
    out[EHDR + 4..EHDR + 8].copy_from_slice(&0b101u32.to_be_bytes()); // PF_R | PF_X
    out[EHDR + 16..EHDR + 24].copy_from_slice(&TEXT.to_be_bytes()); // p_vaddr
    out[EHDR + 40..EHDR + 48].copy_from_slice(&8u64.to_be_bytes()); // p_memsz
    out
}

/// The shadow `passes` builds over a runtime that holds the pair at
/// [`TEXT`].
fn shadow(passes: PpuShadow) -> Option<PredecodedShadow> {
    let mut memory = GuestMemory::new(0x2_0000);
    let code: Vec<u8> = PAIR.iter().flat_map(|w| w.to_be_bytes()).collect();
    let range = ByteRange::new(GuestAddr::new(TEXT), code.len() as u64).expect("range");
    memory.apply_commit(range, &code).expect("mapped");
    let rt = Runtime::new(memory, Budget::new(1), 1);
    let placement = MemoryPlacement {
        alloc_floor: 0x2_0000,
        tls: TlsArea::above(0x2_0000).expect("in range"),
        alloc_base: 0x3_0000,
        user_region_end: 0x2_0000,
        prx_region_end: 0,
    };
    build_instruction_shadow(&rt, &elf(), &placement, 0, passes, &NullSink).expect("built")
}

#[test]
fn the_fused_choice_fuses_the_pair() {
    let shadow = shadow(PpuShadow::Fused).expect("a shadow");
    assert_eq!(shadow.get(TEXT + 4), Some(PpuInstruction::Consumed));
}

#[test]
fn the_quickened_choice_leaves_the_pair_unfused() {
    let shadow = shadow(PpuShadow::Quickened).expect("a shadow");
    assert_eq!(
        shadow.get(TEXT + 4),
        Some(PpuInstruction::CmpwZero { bf: 7, ra: 9 })
    );
}

#[test]
fn the_off_choice_builds_no_shadow() {
    assert!(shadow(PpuShadow::Off).is_none());
}
