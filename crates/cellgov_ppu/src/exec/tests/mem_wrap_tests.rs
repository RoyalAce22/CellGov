//! Stores whose last byte lies past the end of the address space fault as unmapped.

use super::*;

use crate::store_buffer::StoreBuffer;
use cellgov_mem::MemError;

const TOP_LINE: u64 = 0xFFFF_FFFF_FFFF_FF80;

fn fault_addr(verdict: &ExecuteVerdict) -> Option<u64> {
    match verdict {
        ExecuteVerdict::MemFault(MemError::Unmapped(context)) => Some(context.addr),
        _ => None,
    }
}

#[test]
fn a_dcbz_whose_block_ends_past_the_address_space_faults_as_unmapped() {
    let mut s = PpuState::new();
    s.set_gpr(1, TOP_LINE);
    s.set_gpr(2, 0);
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let mem = vec![0u8; 0x40];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let v = execute(
        &PpuInstruction::Dcbz { ra: 1, rb: 2 },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(fault_addr(&v), Some(0xFFFF_FFFF_FFFF_FFF8), "{v:?}");
    // The dcbz staged the fifteen doublewords below the last one, then
    // faulted. The runtime's fault discard clears them with the batch.
    assert_eq!(store_buf.len(), 15);
}

#[test]
fn a_word_store_at_the_last_bytes_of_the_address_space_faults_as_unmapped() {
    let mut s = PpuState::new();
    s.set_gpr(3, 0xDEAD_BEEF);
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let mem = vec![0u8; 0x40];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    // RA = 0 reads as zero, so the displacement alone forms the address.
    let v = execute(
        &PpuInstruction::Stw {
            rs: 3,
            ra: 0,
            imm: -2,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(fault_addr(&v), Some(0xFFFF_FFFF_FFFF_FFFE), "{v:?}");
    assert!(store_buf.is_empty());
    assert!(effects.is_empty());
}

#[test]
fn a_vector_store_on_the_last_line_stages_its_first_half_then_faults() {
    let mut s = PpuState::new();
    s.set_gpr(1, 0xFFFF_FFFF_FFFF_FFF0);
    s.set_gpr(2, 0);
    s.set_vr(3, u128::MAX);
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let mem = vec![0u8; 0x40];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    let v = execute(
        &PpuInstruction::Stvx {
            vs: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(fault_addr(&v), Some(0xFFFF_FFFF_FFFF_FFF8), "{v:?}");
    assert_eq!(store_buf.len(), 1);
}
