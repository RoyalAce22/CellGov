//! An sdcrz run through the runtime zeroes main storage.

// [CBEA p:64 s:7.7.3] sdcrz sets to zero every byte of the data block that contains an addressed byte.

use cellgov_core::Runtime;
use cellgov_exec::YieldReason;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_RD_TAG_STAT, MFC_SDCRZ, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_UPDATE,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const TAG: u32 = 3;

/// `wrch $ch<channel>, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | (u32::from(channel) << 7) | rt
}

/// `rdch rt, $ch<channel>`.
fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | (u32::from(channel) << 7) | rt
}

#[test]
fn sdcrz_zeroes_the_blocks_its_range_touches_and_nothing_else() {
    let mut memory = GuestMemory::new(0x2000);
    let span = ByteRange::new(GuestAddr::new(0xF00), 0x300).expect("in range");
    memory.apply_commit(span, &[0xEE; 0x300]).expect("writable");
    let mut rt = Runtime::new(memory, Budget::new(1), 100);
    rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let program = [
            wrch(MFC_CMD, 2),
            wrch(MFC_WR_TAG_UPDATE, 7),
            rdch(MFC_RD_TAG_STAT, 6),
            0,
        ];
        let state = spu.state_mut();
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        state.set_reg_word_splat(2, MFC_SDCRZ);
        state.set_reg_word_splat(7, MFC_TAG_UPDATE_ALL);
        let c = &mut state.channels;
        c.mfc_eal = 0x1010;
        c.mfc_size = 0x80;
        c.mfc_tag_id = TAG;
        c.tag_mask = 1 << TAG;
        spu
    });

    let mut reasons = Vec::new();
    for _ in 0..100 {
        let Ok(step) = rt.step() else { break };
        reasons.push(step.result.yield_reason);
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        if step.result.yield_reason == YieldReason::Finished {
            break;
        }
    }
    assert_eq!(reasons.last(), Some(&YieldReason::Finished), "{reasons:?}");

    let read = |ea: u64, len: u64| {
        rt.memory()
            .read(ByteRange::new(GuestAddr::new(ea), len).expect("in range"))
            .expect("readable")
            .to_vec()
    };
    assert_eq!(read(0xF00, 0x100), [0xEE; 0x100], "below the first block");
    assert_eq!(read(0x1000, 0x100), [0; 0x100], "both touched blocks");
    assert_eq!(read(0x1100, 0x100), [0xEE; 0x100], "past the last block");
}
