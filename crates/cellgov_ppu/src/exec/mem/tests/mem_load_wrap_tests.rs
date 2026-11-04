//! Loads whose last byte lies past the end of the address space fault as
//! unmapped, through the store-forwarding scan.

use super::*;

use crate::store_buffer::StoreBuffer;
use cellgov_mem::MemError;

#[test]
fn a_word_load_at_the_last_bytes_of_the_address_space_faults_as_unmapped() {
    let mut s = PpuState::new();
    s.set_gpr(1, 0x10);
    s.set_gpr(3, 0xDEAD_BEEF);
    s.set_gpr(4, 0x5A5A);
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let mem = vec![0u8; 0x40];
    let views = [cellgov_mem::RegionView::plain(0, &mem)];
    // A pending store puts an entry in front of the forwarding scan.
    let staged = execute(
        &PpuInstruction::Stw {
            rs: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(staged, ExecuteVerdict::Continue);
    // RA = 0 reads as zero, so the displacement alone forms the address.
    let v = execute(
        &PpuInstruction::Lwz {
            rt: 4,
            ra: 0,
            imm: -2,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    match v {
        ExecuteVerdict::MemFault(MemError::Unmapped(context)) => {
            assert_eq!(context.addr, 0xFFFF_FFFF_FFFF_FFFE);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(s.gpr[4], 0x5A5A);
}
