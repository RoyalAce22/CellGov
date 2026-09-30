//! The encoding-form, effect and outcome classifiers, and the sequence-flow helper.

use cellgov_effects::EffectKind;
use cellgov_ps3_abi::hw::spu;

use crate::instruction::{SpuInstruction, SpuInstructionKind};

use super::types::{SpuEncodingForm, SpuOutcomeClass, SpuSequenceFlow};

const NO_EFFECTS: &[EffectKind] = &[];
const RDCH_EFFECTS: &[EffectKind] = &[EffectKind::MailboxReceiveAttempt];
const WRCH_EFFECTS: &[EffectKind] = &[EffectKind::DmaEnqueue, EffectKind::ConditionalStore];
// The fuzz engine rejects outcomes outside these executor-derived sets.
const CONTINUE: &[SpuOutcomeClass] = &[SpuOutcomeClass::Continue];
const FAULT: &[SpuOutcomeClass] = &[SpuOutcomeClass::Fault];
const YIELD: &[SpuOutcomeClass] = &[SpuOutcomeClass::Yield];
const STOP: &[SpuOutcomeClass] = &[SpuOutcomeClass::Stop];
const CONTINUE_OR_STOP: &[SpuOutcomeClass] = &[SpuOutcomeClass::Continue, SpuOutcomeClass::Stop];
const LOAD_STORE: &[SpuOutcomeClass] = &[SpuOutcomeClass::Continue, SpuOutcomeClass::Fault];
// A tag-status read has three outcomes:
// - a latched status reads at once;
// - a waiting request parks the SPU;
// - a read with no request faults.
const TAG_STATUS_READ: &[SpuOutcomeClass] = &[
    SpuOutcomeClass::Continue,
    SpuOutcomeClass::Yield,
    SpuOutcomeClass::Fault,
];
const CONDITIONAL_BRANCH: &[SpuOutcomeClass] =
    &[SpuOutcomeClass::Continue, SpuOutcomeClass::Branch];
const UNCONDITIONAL_BRANCH: &[SpuOutcomeClass] = &[SpuOutcomeClass::Branch];
pub(super) const WRCH: &[SpuOutcomeClass] = &[
    SpuOutcomeClass::Continue,
    SpuOutcomeClass::Yield,
    SpuOutcomeClass::MemoryRead,
    SpuOutcomeClass::Fault,
];

pub(super) fn exact_kind(raw: u32) -> Option<SpuInstructionKind> {
    crate::decode::decode(raw)
        .ok()
        .map(SpuInstructionKind::from)
}

pub(super) fn sequence_flow(outcomes: &[SpuOutcomeClass]) -> SpuSequenceFlow {
    if outcomes.contains(&SpuOutcomeClass::Branch) {
        SpuSequenceFlow::ControlTransfer
    } else if outcomes == FAULT || outcomes == YIELD || outcomes == STOP {
        SpuSequenceFlow::Terminal
    } else if outcomes != CONTINUE {
        SpuSequenceFlow::StateDependent
    } else {
        SpuSequenceFlow::Linear
    }
}

pub(super) fn effect_and_outcome(
    instruction: &SpuInstruction,
) -> (&'static [EffectKind], &'static [SpuOutcomeClass]) {
    match *instruction {
        SpuInstruction::Lqd { .. }
        | SpuInstruction::Lqx { .. }
        | SpuInstruction::Lqa { .. }
        | SpuInstruction::Lqr { .. }
        | SpuInstruction::Stqd { .. }
        | SpuInstruction::Stqx { .. }
        | SpuInstruction::Stqa { .. }
        | SpuInstruction::Stqr { .. } => (NO_EFFECTS, LOAD_STORE),
        SpuInstruction::Rdch {
            channel: spu::MFC_RD_TAG_STAT,
            ..
        } => (NO_EFFECTS, TAG_STATUS_READ),
        SpuInstruction::Rdch {
            channel: spu::SPU_RD_IN_MBOX,
            ..
        } => (RDCH_EFFECTS, YIELD),
        SpuInstruction::Rdch {
            channel: spu::MFC_RD_ATOMIC_STAT | spu::SPU_RD_MACH_STAT | spu::MFC_RD_TAG_MASK,
            ..
        } => (NO_EFFECTS, CONTINUE),
        SpuInstruction::Rdch { channel, .. }
            if spu::channel_direction(channel) == Some(spu::ChannelDirection::Write) =>
        {
            (NO_EFFECTS, STOP)
        }
        SpuInstruction::Rdch { channel, .. } if spu::is_reserved_channel(channel) => {
            (NO_EFFECTS, CONTINUE)
        }
        SpuInstruction::Rdch { .. } => (NO_EFFECTS, FAULT),
        SpuInstruction::Wrch {
            channel: spu::MFC_CMD,
            ..
        } => (WRCH_EFFECTS, WRCH),
        SpuInstruction::Wrch {
            channel:
                spu::MFC_LSA
                | spu::MFC_EAH
                | spu::MFC_EAL
                | spu::MFC_SIZE
                | spu::MFC_TAG_ID
                | spu::MFC_WR_TAG_MASK
                | spu::SPU_WR_OUT_MBOX,
            ..
        } => (NO_EFFECTS, CONTINUE),
        // The channel refuses a reserved update request.
        SpuInstruction::Wrch {
            channel: spu::MFC_WR_TAG_UPDATE,
            ..
        } => (NO_EFFECTS, LOAD_STORE),
        SpuInstruction::Wrch { channel, .. }
            if spu::channel_direction(channel) == Some(spu::ChannelDirection::Read) =>
        {
            (NO_EFFECTS, STOP)
        }
        SpuInstruction::Wrch { channel, .. } if spu::is_reserved_channel(channel) => {
            (NO_EFFECTS, CONTINUE)
        }
        SpuInstruction::Wrch { .. } => (NO_EFFECTS, FAULT),
        SpuInstruction::Rchcnt { channel, .. }
            if super::support::RCHCNT_CHANNELS.contains(&u32::from(channel))
                || spu::is_reserved_channel(channel) =>
        {
            (NO_EFFECTS, CONTINUE)
        }
        SpuInstruction::Rchcnt { .. } => (NO_EFFECTS, FAULT),
        SpuInstruction::Br { .. }
        | SpuInstruction::Brsl { .. }
        | SpuInstruction::Bi { .. }
        | SpuInstruction::Bisl { .. } => (NO_EFFECTS, UNCONDITIONAL_BRANCH),
        SpuInstruction::Brz { .. }
        | SpuInstruction::Brnz { .. }
        | SpuInstruction::Brhnz { .. }
        | SpuInstruction::Brhz { .. }
        | SpuInstruction::Biz { .. }
        | SpuInstruction::Binz { .. }
        | SpuInstruction::Bihz { .. }
        | SpuInstruction::Bihnz { .. } => (NO_EFFECTS, CONDITIONAL_BRANCH),
        SpuInstruction::Stop { .. } | SpuInstruction::Stopd => (NO_EFFECTS, STOP),
        // [SPU-ISA p:149 s:7 Compare, Branch, and Halt Instructions] a halt stops the SPU only when its condition holds.
        SpuInstruction::Heq { .. }
        | SpuInstruction::Heqi { .. }
        | SpuInstruction::Hgt { .. }
        | SpuInstruction::Hgti { .. }
        | SpuInstruction::Hlgt { .. }
        | SpuInstruction::Hlgti { .. } => (NO_EFFECTS, CONTINUE_OR_STOP),
        _ => (NO_EFFECTS, CONTINUE),
    }
}

pub(super) fn form_for_kind(kind: SpuInstructionKind) -> SpuEncodingForm {
    use SpuInstructionKind as K;
    match kind {
        K::Selb | K::Shufb | K::Mpya => SpuEncodingForm::Rrrr,
        // [SPU-ISA p:29 s:2.3 Instruction Formats] RI10 carries I10 between its opcode and RA fields.
        K::Lqd
        | K::Stqd
        | K::Ai
        | K::Ahi
        | K::Mpyi
        | K::Mpyui
        | K::Andbi
        | K::Andhi
        | K::Orbi
        | K::Orhi
        | K::Xorbi
        | K::Xorhi
        | K::Xori
        | K::Sfhi
        | K::Sfi
        | K::Ori
        | K::Andi
        | K::Ceqi
        | K::Ceqbi
        | K::Cgti
        | K::Heqi
        | K::Hgti
        | K::Hlgti => SpuEncodingForm::Ri10,
        K::Cbd
        | K::Chd
        | K::Cwd
        | K::Cdd
        | K::Shlqbyi
        | K::Rotqbyi
        | K::Rotqmbyi
        | K::Shli
        | K::Rotmi
        | K::Rotmai
        | K::Roti
        | K::Shlqbii
        | K::Rotqbii
        | K::Rotqmbii
        | K::Shlhi
        | K::Rothi
        | K::Rothmi
        | K::Rotmahi => SpuEncodingForm::Ri7,
        K::Lqa | K::Stqa | K::Lqr | K::Stqr | K::Il | K::Ilh | K::Ilhu | K::Iohl | K::Fsmbi => {
            SpuEncodingForm::Ri16
        }
        K::Ila => SpuEncodingForm::Ri18,
        K::Br
        | K::Brsl
        | K::Brz
        | K::Brnz
        | K::Bi
        | K::Bisl
        | K::Brhnz
        | K::Brhz
        | K::Biz
        | K::Binz
        | K::Bihz
        | K::Bihnz => SpuEncodingForm::Branch,
        K::Rdch | K::Wrch | K::Rchcnt => SpuEncodingForm::Channel,
        K::Nop
        | K::Lnop
        | K::Hbr
        | K::Hbra
        | K::Hbrr
        | K::Sync
        | K::Dsync
        | K::Stop
        | K::Stopd
        | K::Mfspr
        | K::Mtspr => SpuEncodingForm::Control,
        _ => SpuEncodingForm::Rrr,
    }
}

pub(super) fn classify_kind(kind: SpuInstructionKind) {
    match kind {
        SpuInstructionKind::Lqd
        | SpuInstructionKind::Lqx
        | SpuInstructionKind::Lqa
        | SpuInstructionKind::Stqd
        | SpuInstructionKind::Stqx
        | SpuInstructionKind::Stqa
        | SpuInstructionKind::Lqr
        | SpuInstructionKind::Stqr
        | SpuInstructionKind::Il
        | SpuInstructionKind::Ila
        | SpuInstructionKind::Ilh
        | SpuInstructionKind::Ilhu
        | SpuInstructionKind::Iohl
        | SpuInstructionKind::Fsmbi
        | SpuInstructionKind::A
        | SpuInstructionKind::Ai
        | SpuInstructionKind::Sf
        | SpuInstructionKind::Avgb
        | SpuInstructionKind::Absdb
        | SpuInstructionKind::Sumb
        | SpuInstructionKind::Mpy
        | SpuInstructionKind::Mpyu
        | SpuInstructionKind::Mpyi
        | SpuInstructionKind::Mpyui
        | SpuInstructionKind::Mpya
        | SpuInstructionKind::Mpyh
        | SpuInstructionKind::Mpys
        | SpuInstructionKind::Mpyhh
        | SpuInstructionKind::Mpyhha
        | SpuInstructionKind::Mpyhhu
        | SpuInstructionKind::Mpyhhau
        | SpuInstructionKind::Addx
        | SpuInstructionKind::Cg
        | SpuInstructionKind::Cgx
        | SpuInstructionKind::Sfx
        | SpuInstructionKind::Bg
        | SpuInstructionKind::Bgx
        | SpuInstructionKind::Ah
        | SpuInstructionKind::Ahi
        | SpuInstructionKind::Sfh
        | SpuInstructionKind::Sfhi
        | SpuInstructionKind::Sfi
        | SpuInstructionKind::And
        | SpuInstructionKind::Or
        | SpuInstructionKind::Selb
        | SpuInstructionKind::Xsbh
        | SpuInstructionKind::Xshw
        | SpuInstructionKind::Xswd
        | SpuInstructionKind::Clz
        | SpuInstructionKind::Cntb
        | SpuInstructionKind::Fsmb
        | SpuInstructionKind::Fsmh
        | SpuInstructionKind::Fsm
        | SpuInstructionKind::Gbb
        | SpuInstructionKind::Gb
        | SpuInstructionKind::Gbh
        | SpuInstructionKind::Ori
        | SpuInstructionKind::Andc
        | SpuInstructionKind::Orc
        | SpuInstructionKind::Xor
        | SpuInstructionKind::Nand
        | SpuInstructionKind::Eqv
        | SpuInstructionKind::Orx
        | SpuInstructionKind::Andbi
        | SpuInstructionKind::Andhi
        | SpuInstructionKind::Orbi
        | SpuInstructionKind::Orhi
        | SpuInstructionKind::Xorbi
        | SpuInstructionKind::Xorhi
        | SpuInstructionKind::Xori
        | SpuInstructionKind::Nor
        | SpuInstructionKind::Andi
        | SpuInstructionKind::Shufb
        | SpuInstructionKind::Shlqbyi
        | SpuInstructionKind::Rotqby
        | SpuInstructionKind::Rotqbyi
        | SpuInstructionKind::Rotqmbyi
        | SpuInstructionKind::Shlqbi
        | SpuInstructionKind::Shlqbii
        | SpuInstructionKind::Rotqbi
        | SpuInstructionKind::Rotqbii
        | SpuInstructionKind::Rotqmbi
        | SpuInstructionKind::Rotqmbii
        | SpuInstructionKind::Shlqby
        | SpuInstructionKind::Shlqbybi
        | SpuInstructionKind::Rotqbybi
        | SpuInstructionKind::Rotqmby
        | SpuInstructionKind::Rotqmbybi
        | SpuInstructionKind::Shl
        | SpuInstructionKind::Shli
        | SpuInstructionKind::Rotmi
        | SpuInstructionKind::Rotmai
        | SpuInstructionKind::Rot
        | SpuInstructionKind::Roti
        | SpuInstructionKind::Rotm
        | SpuInstructionKind::Rotma
        | SpuInstructionKind::Shlh
        | SpuInstructionKind::Shlhi
        | SpuInstructionKind::Roth
        | SpuInstructionKind::Rothi
        | SpuInstructionKind::Rothm
        | SpuInstructionKind::Rothmi
        | SpuInstructionKind::Rotmah
        | SpuInstructionKind::Rotmahi
        | SpuInstructionKind::Cbd
        | SpuInstructionKind::Cbx
        | SpuInstructionKind::Chd
        | SpuInstructionKind::Chx
        | SpuInstructionKind::Cwd
        | SpuInstructionKind::Cwx
        | SpuInstructionKind::Cdd
        | SpuInstructionKind::Cdx
        | SpuInstructionKind::Ceq
        | SpuInstructionKind::Ceqi
        | SpuInstructionKind::Ceqbi
        | SpuInstructionKind::Cgti
        | SpuInstructionKind::Clgt
        | SpuInstructionKind::Br
        | SpuInstructionKind::Brsl
        | SpuInstructionKind::Brz
        | SpuInstructionKind::Brnz
        | SpuInstructionKind::Bi
        | SpuInstructionKind::Bisl
        | SpuInstructionKind::Brhnz
        | SpuInstructionKind::Brhz
        | SpuInstructionKind::Biz
        | SpuInstructionKind::Binz
        | SpuInstructionKind::Bihz
        | SpuInstructionKind::Bihnz
        | SpuInstructionKind::Rdch
        | SpuInstructionKind::Wrch
        | SpuInstructionKind::Rchcnt
        | SpuInstructionKind::Nop
        | SpuInstructionKind::Lnop
        | SpuInstructionKind::Hbr
        | SpuInstructionKind::Hbra
        | SpuInstructionKind::Hbrr
        | SpuInstructionKind::Sync
        | SpuInstructionKind::Dsync
        | SpuInstructionKind::Heq
        | SpuInstructionKind::Heqi
        | SpuInstructionKind::Hgt
        | SpuInstructionKind::Hgti
        | SpuInstructionKind::Hlgt
        | SpuInstructionKind::Hlgti
        | SpuInstructionKind::Stop
        | SpuInstructionKind::Stopd
        | SpuInstructionKind::Mfspr
        | SpuInstructionKind::Mtspr => {}
    }
}
