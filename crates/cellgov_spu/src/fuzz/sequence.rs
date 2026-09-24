//! The bounded multi-instruction interactions and the programs that stage them.

use crate::state::{SpuState, SPU_LS_SIZE};
use cellgov_ps3_abi::hw::spu;
use cellgov_sync::ReservedLine;

use crate::instruction::{SpuInstruction, SpuInstructionKind};

use super::types::{SpuGenerationDescriptor, SpuGenerationError, SpuOperandClass};

/// A bounded interaction that requires a specific SPU instruction order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpuSequenceInteraction {
    /// Stage a channel value before reading channel status.
    Channel,
    /// Produce a pending inbound-mailbox read.
    Mailbox,
    /// Stage an address and then enqueue DMA.
    Dma,
    /// Stage a DMA GET that leaves an unresolved transfer.
    DmaGet,
    /// Request an atomic memory read for the caller to service.
    MemoryRead,
    /// Stage an address and attempt a conditional store.
    Reservation,
    /// Store a quadword and then load that address.
    LocalStore,
    /// Cross the local-store bound after a legal instruction.
    LocalStoreFault,
    /// Branch across a decoy word to the next fetchable word.
    Branch,
    /// Reach a terminating STOP after a preceding instruction.
    Stop,
}

impl SpuSequenceInteraction {
    /// Lists the interaction families in stable generator order.
    pub const ALL: [Self; 10] = [
        Self::Channel,
        Self::Mailbox,
        Self::Dma,
        Self::DmaGet,
        Self::MemoryRead,
        Self::Reservation,
        Self::LocalStore,
        Self::LocalStoreFault,
        Self::Branch,
        Self::Stop,
    ];

    /// Removes one trailing decoy without deleting the interaction's required instructions.
    pub fn shrink_words(self, words: &[u32]) -> Option<Vec<u32>> {
        let essential = if matches!(self, Self::Branch | Self::Channel) {
            3
        } else {
            2
        };
        if words.len() <= essential
            || crate::decode::decode(*words.last()?).ok() != Some(SpuInstruction::Nop)
        {
            return None;
        }
        Some(words[..words.len() - 1].to_vec())
    }

    /// Prepares register and channel inputs for the selected dependency.
    pub fn prepare_state(self, state: &mut SpuState, data_base: u32) {
        // [Wang2024 p:340:16 s:3.8] Every generation step establishes the preconditions its instructions need, so the program is well defined.
        match self {
            Self::Channel | Self::Dma | Self::DmaGet | Self::MemoryRead | Self::Reservation => {
                state.set_reg_word_splat(1, data_base);
                state.set_reg_word_splat(
                    2,
                    match self {
                        Self::Dma => spu::MFC_PUT,
                        Self::DmaGet => spu::MFC_GET,
                        Self::MemoryRead => spu::MFC_GETLLAR,
                        Self::Channel => spu::MFC_TAG_UPDATE_IMMEDIATE,
                        Self::Reservation => spu::MFC_PUTLLC,
                        _ => data_base,
                    },
                );
                state.channels.mfc_lsa = data_base + cellgov_sync::RESERVATION_LINE_BYTES as u32;
                state.channels.mfc_eal = data_base;
                state.reservation = Some(ReservedLine::containing(u64::from(data_base)));
            }
            Self::LocalStore => {
                state.set_reg_word_splat(2, data_base);
                state.regs[1] = std::array::from_fn(|index| index as u8 + 1);
            }
            Self::LocalStoreFault => {
                state.set_reg_word_splat(2, spu::MFC_PUTLLC);
                state.channels.mfc_lsa = (SPU_LS_SIZE - 64) as u32;
                state.channels.mfc_eal = data_base;
                state.reservation = Some(ReservedLine::containing(u64::from(data_base)));
            }
            Self::Mailbox | Self::Branch | Self::Stop => {}
        }
    }

    /// Encodes the minimal program and its decoy words from typed descriptors.
    ///
    /// # Errors
    ///
    /// Returns a generation error if a required kind or operand is unavailable.
    pub fn words(
        self,
        descriptors: &[SpuGenerationDescriptor],
    ) -> Result<Vec<u32>, SpuGenerationError> {
        use SpuInstructionKind as K;
        let word = |kind, operands: &[(SpuOperandClass, usize, u32)]| {
            let descriptor = descriptors
                .iter()
                .find(|descriptor| descriptor.kind == kind)
                .ok_or(SpuGenerationError::MissingSequenceKind { kind })?;
            let mut values = descriptor.canonical_parameters();
            for &(class, occurrence, value) in operands {
                let index = descriptor
                    .operands
                    .iter()
                    .enumerate()
                    .filter(|(_, field)| field.class == class)
                    .nth(occurrence)
                    .map(|(index, _)| index)
                    .ok_or(SpuGenerationError::InvalidOperands)?;
                values[index] = value;
            }
            descriptor.encode(&values)
        };
        let reg = SpuOperandClass::Register;
        let channel = SpuOperandClass::Channel;
        let immediate = SpuOperandClass::Immediate;
        let wrch = |source, selector| word(K::Wrch, &[(reg, 0, source), (channel, 0, selector)]);
        let program = match self {
            Self::Channel => vec![
                wrch(1, u32::from(spu::MFC_LSA))?,
                // [CBE-Handbook p:460 s:17.10.4] Read tag status only after a tag-update request.
                wrch(2, u32::from(spu::MFC_WR_TAG_UPDATE))?,
                word(
                    K::Rdch,
                    &[(reg, 0, 3), (channel, 0, u32::from(spu::MFC_RD_TAG_STAT))],
                )?,
            ],
            Self::Mailbox => vec![
                word(K::Nop, &[])?,
                word(
                    K::Rdch,
                    &[(reg, 0, 3), (channel, 0, u32::from(spu::SPU_RD_IN_MBOX))],
                )?,
            ],
            Self::Dma | Self::DmaGet | Self::MemoryRead => vec![
                wrch(1, u32::from(spu::MFC_LSA))?,
                wrch(2, u32::from(spu::MFC_CMD))?,
            ],
            Self::Reservation => vec![
                wrch(1, u32::from(spu::MFC_LSA))?,
                wrch(2, u32::from(spu::MFC_CMD))?,
            ],
            Self::LocalStore => vec![
                word(K::Stqd, &[(reg, 0, 1), (reg, 1, 2)])?,
                word(K::Lqd, &[(reg, 0, 3), (reg, 1, 2)])?,
            ],
            Self::LocalStoreFault => vec![word(K::Nop, &[])?, wrch(2, u32::from(spu::MFC_CMD))?],
            Self::Branch => vec![
                word(K::Br, &[(immediate, 0, 2)])?,
                word(K::Il, &[(reg, 0, 5), (immediate, 0, 7)])?,
                word(K::Nop, &[])?,
            ],
            Self::Stop => vec![word(K::Nop, &[])?, word(K::Stop, &[])?],
        };
        Ok(program)
    }
}
