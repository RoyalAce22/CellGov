//! The execution-support checks, the channel tables and the staged state inputs.

use cellgov_ps3_abi::hw::spu;

use crate::instruction::{SpuInstruction, SpuInstructionKind};

use super::classify::exact_kind;
use super::fields::operand_combination_is_valid;
use super::types::SpuStateInput;

/// Tests a decoded word for undefined operand combinations.
pub fn encoding_has_undefined_operands(raw: u32) -> bool {
    exact_kind(raw).is_some_and(|kind| !operand_combination_is_valid(kind, raw))
}

/// Tests whether the executor supports a decoded word.
pub fn encoding_execution_is_supported(raw: u32) -> bool {
    let Ok(instruction) = crate::decode::decode(raw) else {
        return false;
    };
    if !execution_supported(instruction) {
        return false;
    }
    let kind = SpuInstructionKind::from(instruction);
    let controls_interrupts = matches!(
        kind,
        SpuInstructionKind::Bi
            | SpuInstructionKind::Bisl
            | SpuInstructionKind::Biz
            | SpuInstructionKind::Binz
            | SpuInstructionKind::Bihz
            | SpuInstructionKind::Bihnz
    ) && raw & 0x000c_0000 != 0;
    // [SPU-ISA p:178 s:7 Compare, Branch, and Halt Instructions] BI's E and D
    // options replace interrupt-enable state, which the executor does not model.
    // [SPU-ISA p:181 s:7 Compare, Branch, and Halt Instructions] BISL has the
    // same interrupt-control options.
    // [SPU-ISA p:186 s:7 Compare, Branch, and Halt Instructions] BIZ has the
    // same interrupt-control options.
    // [SPU-ISA p:187 s:7 Compare, Branch, and Halt Instructions] BINZ has the
    // same interrupt-control options.
    // [SPU-ISA p:188 s:7 Compare, Branch, and Halt Instructions] BIHZ has the
    // same interrupt-control options.
    // [SPU-ISA p:189 s:7 Compare, Branch, and Halt Instructions] BIHNZ has the
    // same interrupt-control options.
    !controls_interrupts
}

const MFC_COMMAND_INPUTS: &[u32] = &[
    spu::MFC_PUT,
    spu::MFC_GET,
    spu::MFC_GETLLAR,
    spu::MFC_PUTLLC,
];
const MFC_TAG_UPDATE_INPUTS: &[u32] = &[
    spu::MFC_TAG_UPDATE_IMMEDIATE,
    spu::MFC_TAG_UPDATE_ANY,
    spu::MFC_TAG_UPDATE_ALL,
];

pub(super) fn state_input(instruction: SpuInstruction) -> Option<SpuStateInput> {
    match instruction {
        SpuInstruction::Wrch {
            channel: spu::MFC_CMD,
            rt,
        } => Some(SpuStateInput {
            register: rt,
            values: MFC_COMMAND_INPUTS,
            preferred: Some(spu::MFC_PUTLLC),
        }),
        SpuInstruction::Wrch {
            channel: spu::MFC_WR_TAG_UPDATE,
            rt,
        } => Some(SpuStateInput {
            register: rt,
            values: MFC_TAG_UPDATE_INPUTS,
            preferred: Some(spu::MFC_TAG_UPDATE_IMMEDIATE),
        }),
        _ => None,
    }
}

pub(super) fn execution_supported(instruction: SpuInstruction) -> bool {
    match instruction {
        SpuInstruction::Rdch { channel, .. } => RDCH_CHANNELS.contains(&u32::from(channel)),
        SpuInstruction::Wrch { channel, .. } => WRCH_CHANNELS.contains(&u32::from(channel)),
        SpuInstruction::Rchcnt { channel, .. } => RCHCNT_CHANNELS.contains(&u32::from(channel)),
        // [SPU-ISA p:150 s:7 Compare, Branch, and Halt Instructions] HEQ can
        // stop execution, but the current instruction representation retains
        // neither source register needed to decide that outcome.
        SpuInstruction::Heq => false,
        // [SPU-ISA p:238 s:10 Control Instructions] STOP signals its 14-bit
        // value to the external environment, while the executor only records
        // that the unit finished.
        SpuInstruction::Stop { .. } => false,
        _ => true,
    }
}

const RDCH_CHANNELS: &[u32] = &[
    spu::MFC_RD_TAG_STAT as u32,
    spu::MFC_RD_ATOMIC_STAT as u32,
    spu::SPU_RD_IN_MBOX as u32,
    spu::SPU_RD_MACH_STAT as u32,
];
// [CBE-Handbook p:463 s:17.12 SPU Mailbox Channels] Outbound mailbox writes
// send guest-visible messages, so they stay unsupported until execution emits them.
const WRCH_CHANNELS: &[u32] = &[
    spu::MFC_LSA as u32,
    spu::MFC_EAH as u32,
    spu::MFC_EAL as u32,
    spu::MFC_SIZE as u32,
    spu::MFC_TAG_ID as u32,
    spu::MFC_CMD as u32,
    spu::MFC_WR_TAG_MASK as u32,
    spu::MFC_WR_TAG_UPDATE as u32,
];
const RCHCNT_CHANNELS: &[u32] = &[spu::SPU_RD_MACH_STAT as u32];

pub(super) fn channel_values(kind: SpuInstructionKind) -> &'static [u32] {
    match kind {
        SpuInstructionKind::Rdch => RDCH_CHANNELS,
        SpuInstructionKind::Wrch => WRCH_CHANNELS,
        SpuInstructionKind::Rchcnt => RCHCNT_CHANNELS,
        _ => &[],
    }
}
