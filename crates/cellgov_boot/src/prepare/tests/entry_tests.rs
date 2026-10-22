use super::{seed_process_start_registers, shadow_extent, ProcessStart};
use crate::prx::TlsArea;

/// An ELF64 big-endian header and one PT_LOAD per `(vaddr, memsz,
/// p_flags)`. The program headers carry no file bytes, which
/// `shadow_extent` never reads.
fn elf_with_loads(loads: &[(u64, u64, u32)]) -> Vec<u8> {
    const EHDR: usize = 64;
    const PHENT: usize = 56;
    const PT_LOAD: u32 = 1;
    let mut out = vec![0u8; EHDR + PHENT * loads.len()];
    out[..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    out[4] = 2; // ELFCLASS64
    out[5] = 2; // ELFDATA2MSB
    out[6] = 1; // EV_CURRENT
    out[18..20].copy_from_slice(&21u16.to_be_bytes()); // EM_PPC64
    out[32..40].copy_from_slice(&(EHDR as u64).to_be_bytes()); // e_phoff
    out[54..56].copy_from_slice(&(PHENT as u16).to_be_bytes()); // e_phentsize
    out[56..58].copy_from_slice(&(loads.len() as u16).to_be_bytes()); // e_phnum
    for (i, &(vaddr, memsz, flags)) in loads.iter().enumerate() {
        let ph = EHDR + i * PHENT;
        out[ph..ph + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        out[ph + 4..ph + 8].copy_from_slice(&flags.to_be_bytes());
        out[ph + 16..ph + 24].copy_from_slice(&vaddr.to_be_bytes());
        out[ph + 40..ph + 48].copy_from_slice(&memsz.to_be_bytes()); // p_memsz
    }
    out
}

const PF_RX: u32 = 0b101;
const PF_RW: u32 = 0b110;

/// The PSL1GHT layout: text at 0x10000, data and OPDs at 0x10000000.
fn text_low_data_high() -> Vec<u8> {
    elf_with_loads(&[(0x1_0000, 0x1_0090, PF_RX), (0x1000_0000, 0x1_1000, PF_RW)])
}

#[test]
fn the_shadow_ends_at_the_text_when_a_data_segment_sits_above_it() {
    let extent = shadow_extent(&text_low_data_high(), 0, 0x1001_1000, 0x4000_0000).unwrap();
    assert_eq!(extent, 0x2_0090);
}

#[test]
fn the_shadow_reaches_the_firmware_region_above_the_data() {
    let extent =
        shadow_extent(&text_low_data_high(), 0x1080_0000, 0x1080_0000, 0x4000_0000).unwrap();
    assert_eq!(extent, 0x1080_0000);
}

#[test]
fn the_heap_floor_and_the_memory_cap_the_shadow() {
    let elf = elf_with_loads(&[(0x1_0000, 0x10_0000, PF_RX)]);
    assert_eq!(
        shadow_extent(&elf, 0, 0x8_0000, 0x4000_0000).unwrap(),
        0x8_0000
    );
    assert_eq!(
        shadow_extent(&elf, 0, 0x4000_0000, 0x4_0000).unwrap(),
        0x4_0000
    );
}

#[test]
fn an_image_with_no_program_headers_is_refused_by_name() {
    let err = shadow_extent(&elf_with_loads(&[]), 0, 0x1000, 0x1000).unwrap_err();
    assert!(
        matches!(err, super::EntryError::ProgramHeaders { .. }),
        "{err}"
    );
}

fn start() -> ProcessStart {
    ProcessStart {
        entry: 0x1_0000,
        tls: TlsArea::above(0x10a5_7708).expect("in range"),
    }
}

#[test]
fn a_primary_entry_without_a_termination_function_seeds_r7_zero() {
    let mut state = cellgov_ppu::state::PpuState::new();
    seed_process_start_registers(&mut state, None, 0x10_0000, start());
    assert_eq!(state.gpr[7], 0);
}

#[test]
fn the_primary_entry_r13_points_into_the_placed_tls_area() {
    let mut state = cellgov_ppu::state::PpuState::new();
    seed_process_start_registers(&mut state, None, 0x10_0000, start());
    assert_eq!(state.gpr[13], 0x10a6_0000 + 0x7030);
}
