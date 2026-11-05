//! The loader declares each executable PT_LOAD as code, and no other.

use super::*;
use crate::state::PpuState;
use cellgov_mem::GuestMemory;

const PF_X: u32 = 1;
/// `PF_R | PF_W`.
const PF_RW: u32 = 6;

/// An ELF64 big-endian PPC64 image whose PT_LOADs are `(vaddr, memsz,
/// flags)`, each with no file bytes.
fn elf(loads: &[(u64, u64, u32)]) -> Vec<u8> {
    let mut data = vec![0u8; 64 + 56 * loads.len()];
    data[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    data[4] = 2;
    data[5] = 2;
    data[6] = 1;
    data[18..20].copy_from_slice(&21u16.to_be_bytes());
    data[32..40].copy_from_slice(&64u64.to_be_bytes());
    data[54..56].copy_from_slice(&56u16.to_be_bytes());
    data[56..58].copy_from_slice(&(loads.len() as u16).to_be_bytes());
    for (slot, &(vaddr, memsz, flags)) in loads.iter().enumerate() {
        let ph = 64 + 56 * slot;
        data[ph..ph + 4].copy_from_slice(&1u32.to_be_bytes());
        data[ph + 4..ph + 8].copy_from_slice(&flags.to_be_bytes());
        data[ph + 16..ph + 24].copy_from_slice(&vaddr.to_be_bytes());
        data[ph + 40..ph + 48].copy_from_slice(&memsz.to_be_bytes());
    }
    data
}

#[test]
fn only_the_executable_segments_become_code() {
    let data = elf(&[
        (0x100, 0x40, PF_X),
        (0x200, 0x80, PF_RW),
        (0x300, 0x10, PF_X | PF_RW),
    ]);
    let mut mem = GuestMemory::new(0x1000);
    load_ppu_elf(&data, &mut mem, &mut PpuState::new()).expect("loads");
    assert_eq!(mem.code_spans(), [(0x100, 0x140), (0x300, 0x310)]);
}
