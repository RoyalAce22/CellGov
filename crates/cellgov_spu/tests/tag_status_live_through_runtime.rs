//! Tag-group status is live: a tag reused after a completed wait reads
//! incomplete until its new transfer lands.

// [CBEA p:128 s:9.3.6] a set bit means the group has no outstanding operations; a clear bit means it has some.

use cellgov_core::Runtime;
use cellgov_exec::YieldReason;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_PUT, MFC_RD_TAG_STAT, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_UPDATE,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const TAG: u32 = 1;

/// `wrch MFC_Cmd, r2`: RR opcode 0x10D.
const PUT: u32 = (0x10D << 21) | ((MFC_CMD as u32) << 7) | 2;
/// `wrch MFC_WrTagUpdate, r7`: request an update once all masked groups complete.
const REQUEST_ALL: u32 = (0x10D << 21) | ((MFC_WR_TAG_UPDATE as u32) << 7) | 7;
/// `rdch r5, MFC_RdTagStat`: RR opcode 0x00D.
const WAIT_R5: u32 = (0x00D << 21) | ((MFC_RD_TAG_STAT as u32) << 7) | 5;
/// `rdch r6, MFC_RdTagStat`.
const WAIT_R6: u32 = (0x00D << 21) | ((MFC_RD_TAG_STAT as u32) << 7) | 6;

#[test]
fn a_reused_tag_waits_for_its_second_transfer() {
    let mut rt = Runtime::new(GuestMemory::new(0x2000), Budget::new(1), 400);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        for (i, word) in [PUT, REQUEST_ALL, WAIT_R5, PUT, REQUEST_ALL, WAIT_R6, 0]
            .iter()
            .enumerate()
        {
            spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        let state = spu.state_mut();
        state.set_reg_word_splat(2, MFC_PUT);
        state.set_reg_word_splat(7, MFC_TAG_UPDATE_ALL);
        state.channels.mfc_lsa = 0x100;
        state.channels.mfc_eal = 0x1000;
        state.channels.mfc_size = 16;
        state.channels.mfc_tag_id = TAG;
        state.channels.tag_mask = 1 << TAG;
        spu
    });

    let mut reasons = Vec::new();
    for _ in 0..400 {
        let Ok(step) = rt.step() else { break };
        reasons.push(step.result.yield_reason);
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
        if step.result.yield_reason == YieldReason::Finished {
            break;
        }
    }

    let submits: Vec<usize> = reasons
        .iter()
        .enumerate()
        .filter(|(_, r)| **r == YieldReason::DmaSubmitted)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(submits.len(), 2, "two puts: {reasons:?}");
    assert_eq!(reasons.last(), Some(&YieldReason::Finished), "{reasons:?}");
    assert!(
        reasons[submits[1]..].contains(&YieldReason::DmaWait),
        "the wait after the second put must park until that put lands: {reasons:?}"
    );
    // The park check above also passes for a status that never reads
    // complete, so this check proves the group reads complete again.
    let spu = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit");
    assert_eq!(
        spu.state().channels.tag_status & (1 << TAG),
        1 << TAG,
        "the reused group reads complete after its second transfer lands",
    );
}
