//! The three SPU barriers decode apart, and the interpreter shows a
//! store to the next fetch without a `sync`.

use super::*;
use crate::decode::decode;

/// [SPU-ISA p:242 s:10] sync is 00000000010 with the C feature bit at bit 11; [SPU-ISA p:243 s:10] dsync is 00000000011.
#[test]
fn sync_sync_c_and_dsync_decode_apart() {
    assert_eq!(decode(0x002 << 21), Ok(SpuInstruction::Sync { c: false }));
    assert_eq!(
        decode((0x002 << 21) | 0x0010_0000),
        Ok(SpuInstruction::Sync { c: true })
    );
    assert_eq!(decode(0x003 << 21), Ok(SpuInstruction::Dsync));
}

/// A store into the instruction stream runs as the stored instruction,
/// with no barrier after the store or with any of the three: CellGov
/// picks the outcome in which the fetch sees the store.
///
/// [SPU-ISA p:255 s:13.3] without a sync the SPU might or might not execute the newly stored instruction.
#[test]
fn a_store_into_the_instruction_stream_is_fetched_with_or_without_a_barrier() {
    /// `sync`, `sync.c` and `dsync`.
    const BARRIERS: [u32; 3] = [0x002 << 21, (0x002 << 21) | 0x0010_0000, 0x003 << 21];
    for barrier in [0x201 << 21].into_iter().chain(BARRIERS) {
        run_stored_instruction(barrier);
    }
}

fn run_stored_instruction(after_store: u32) {
    use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    /// `stqd r4, 16(r0)`: RI10 opcode 0x24 with I10 = 1.
    const STQD_R4_16: u32 = (0x24 << 24) | (1 << 14) | 4;
    /// `nop`: RR opcode 0x201.
    const NOP: u32 = 0x201 << 21;
    /// `il r3, 9`: RI16 opcode 0x081.
    const IL_R3_9: u32 = (0x081 << 23) | (9 << 7) | 3;

    let mut unit = crate::SpuExecutionUnit::new(UnitId::new(1));
    for (i, word) in [STQD_R4_16, after_store, NOP, NOP, NOP].iter().enumerate() {
        unit.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    // r4 holds `il r3, 9` then three zero words, which are `stop 0`.
    unit.state_mut().regs[4][..4].copy_from_slice(&IL_R3_9.to_be_bytes());

    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(20), &ExecutionContext::new(&mem), &mut effects);
    assert_eq!(
        result.yield_reason,
        YieldReason::Finished,
        "0x{after_store:08x}"
    );
    assert_eq!(unit.state().reg_word(3), 9, "0x{after_store:08x}");
}
