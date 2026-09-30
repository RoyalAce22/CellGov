//! A list get with a stall-and-notify element, run through the runtime.

// [CBEA p:129 s:9.3.7] the stall occurs once the flagged element completes; the MFC reads no element past it until the acknowledgment, so software may change the later elements.

use cellgov_core::Runtime;
use cellgov_exec::YieldReason;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_GETL, MFC_LIST_STALL_NOTIFY, MFC_RD_LIST_STALL_STAT, MFC_RD_TAG_STAT,
    MFC_TAG_UPDATE_ALL, MFC_WR_LIST_STALL_ACK, MFC_WR_TAG_UPDATE,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const TAG: u32 = 3;
const LIST: usize = 0x200;
const DATA: u32 = 0x400;

/// `wrch $ch<channel>, rt`.
fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | (u32::from(channel) << 7) | rt
}

/// `rdch rt, $ch<channel>`.
fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | (u32::from(channel) << 7) | rt
}

/// `stqd r8, 0x210(r0)`: RI10 opcode 0x24, I10 in quadwords.
const STORE_ELEMENT_3: u32 = (0x24 << 24) | (0x21 << 14) | 8;

#[test]
fn a_stalled_list_resumes_with_the_element_software_rewrote() {
    let mut memory = GuestMemory::new(0x2000);
    for (ea, fill) in [
        (0x1000, 0xA1),
        (0x1100, 0xB2),
        (0x1200, 0xC3),
        (0x1300, 0xD4),
    ] {
        let range = ByteRange::new(GuestAddr::new(ea), 16).expect("in range");
        memory.apply_commit(range, &[fill; 16]).expect("writable");
    }
    let mut rt = Runtime::new(memory, Budget::new(1), 400);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let program = [
            wrch(MFC_CMD, 2),
            rdch(MFC_RD_LIST_STALL_STAT, 5),
            STORE_ELEMENT_3,
            wrch(MFC_WR_LIST_STALL_ACK, 9),
            wrch(MFC_WR_TAG_UPDATE, 7),
            rdch(MFC_RD_TAG_STAT, 6),
            0,
        ];
        let state = spu.state_mut();
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        let elements = [
            (0x10, 0x1000u32),
            (0x10 | MFC_LIST_STALL_NOTIFY, 0x1100),
            (0x10, 0x1200),
        ];
        for (i, (head, eal)) in elements.iter().enumerate() {
            let at = LIST + 8 * i;
            state.ls[at..at + 4].copy_from_slice(&head.to_be_bytes());
            state.ls[at + 4..at + 8].copy_from_slice(&eal.to_be_bytes());
        }
        state.set_reg_word_splat(2, MFC_GETL);
        state.set_reg_word_splat(7, MFC_TAG_UPDATE_ALL);
        state.set_reg_word_splat(9, TAG);
        // The replacement for element 3: 16 bytes from 0x1300.
        state.regs[8][..4].copy_from_slice(&0x10u32.to_be_bytes());
        state.regs[8][4..8].copy_from_slice(&0x1300u32.to_be_bytes());
        state.regs[8][8..].fill(0);
        let c = &mut state.channels;
        c.mfc_lsa = DATA;
        c.mfc_eal = LIST as u32;
        c.mfc_size = 24;
        c.mfc_tag_id = TAG;
        c.tag_mask = 1 << TAG;
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
    assert_eq!(reasons.last(), Some(&YieldReason::Finished), "{reasons:?}");
    let submits: Vec<usize> = reasons
        .iter()
        .enumerate()
        .filter(|(_, r)| **r == YieldReason::DmaSubmitted)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(submits.len(), 2, "the list, then the ack: {reasons:?}");
    assert!(
        reasons[submits[0]..submits[1]].contains(&YieldReason::ChannelStall),
        "the stall read parks until the flagged element lands: {reasons:?}"
    );

    let spu = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit");
    let state = spu.state();
    assert_eq!(
        state.reg_word(5),
        1 << TAG,
        "the stall read names the group"
    );
    assert_eq!(
        state.reg_word(6),
        1 << TAG,
        "the list completes after the ack"
    );
    let data = |offset: usize| &state.ls[DATA as usize + offset..DATA as usize + offset + 16];
    assert_eq!(data(0), [0xA1; 16]);
    assert_eq!(data(0x10), [0xB2; 16]);
    assert_eq!(data(0x20), [0xD4; 16], "the rewritten element, not 0x1200");
}
