//! Conversions between the vector-set schema and the SPU's own types.

use cellgov_dma::{DmaDirection, InvalidMfcCommand, MfcCommandError, MfcOrdering};
use cellgov_effects::Effect;
use cellgov_ps3_abi::hw::spu::{MfcTagId, MFC_TAG_UPDATE_ALL, MFC_TAG_UPDATE_ANY};
use cellgov_spu::state::{
    ChannelState, ListCursor, SignalNotifyMode, SignalNotifyRegister, TagUpdateCondition,
};
use cellgov_spu::stop::{SpuStop, SpuStopKind};

use crate::reference::is_lower_hex;

use super::set_types::{
    SpuReferenceBytes, SpuReferenceChannelState, SpuReferenceDirection, SpuReferenceEffect,
    SpuReferenceInvalidCommand, SpuReferenceList, SpuReferenceMfcCause, SpuReferenceOrdering,
    SpuReferenceSignal, SpuReferenceSignalMode, SpuReferenceStop, SpuReferenceStopKind,
};

/// The bytes a hex run names, or `None` for an empty or malformed run.
pub(super) fn parse_hex_bytes(hex: &str) -> Option<Vec<u8>> {
    if hex.is_empty() || !is_lower_hex(hex, hex.len()) || !hex.len().is_multiple_of(2) {
        return None;
    }
    hex.as_bytes()
        .chunks(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some((high << 4 | low) as u8)
        })
        .collect()
}

/// Writes each run that lies inside `bytes`, which start at `base`.
pub(super) fn write_runs(bytes: &mut [u8], base: u64, runs: &[SpuReferenceBytes]) {
    for run in runs {
        let Some(data) = parse_hex_bytes(&run.hex) else {
            continue;
        };
        let Some(offset) = run
            .at
            .checked_sub(base)
            .and_then(|offset| usize::try_from(offset).ok())
        else {
            continue;
        };
        if let Some(slot) = offset
            .checked_add(data.len())
            .and_then(|end| bytes.get_mut(offset..end))
        {
            slot.copy_from_slice(&data);
        }
    }
}

/// `bytes` as lowercase hex.
pub(super) fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl Default for SpuReferenceChannelState {
    fn default() -> Self {
        Self::from(&ChannelState::new())
    }
}

impl From<&ChannelState> for SpuReferenceChannelState {
    fn from(value: &ChannelState) -> Self {
        let ChannelState {
            mfc_lsa,
            mfc_eah,
            mfc_eal,
            mfc_size,
            mfc_tag_id,
            tag_mask,
            tag_status,
            atomic_status,
            cmd_queue_free,
            tag_update,
            tag_status_read,
            atomic_status_ready,
            in_mbox,
            out_mbox,
            lists,
            list_stall_status,
            mssync_tracking,
            mssync_horizon,
            pending_events,
            event_mask,
            event_count,
            event_levels,
        } = value;
        Self {
            mfc_lsa: *mfc_lsa,
            mfc_eah: *mfc_eah,
            mfc_eal: *mfc_eal,
            mfc_size: *mfc_size,
            mfc_tag_id: *mfc_tag_id,
            tag_mask: *tag_mask,
            tag_status: *tag_status,
            atomic_status: *atomic_status,
            mfc_cmd_count: *cmd_queue_free,
            tag_update: tag_update.map(|condition| match condition {
                TagUpdateCondition::Any => MFC_TAG_UPDATE_ANY,
                TagUpdateCondition::All => MFC_TAG_UPDATE_ALL,
            }),
            tag_status_read: *tag_status_read,
            atomic_status_ready: *atomic_status_ready,
            in_mbox: in_mbox.clone(),
            out_mbox: *out_mbox,
            lists: lists.iter().map(SpuReferenceList::from).collect(),
            list_stall_status: *list_stall_status,
            mssync_tracking: *mssync_tracking,
            mssync_horizon: *mssync_horizon,
            pending_events: *pending_events,
            event_mask: *event_mask,
            event_count: *event_count,
            event_levels: *event_levels,
        }
    }
}

impl SpuReferenceChannelState {
    /// The channel state this names, or `None` for a tag-update code
    /// other than any or all, or a list tag of 32 or more.
    pub(super) fn to_channels(&self) -> Option<ChannelState> {
        let Self {
            mfc_lsa,
            mfc_eah,
            mfc_eal,
            mfc_size,
            mfc_tag_id,
            tag_mask,
            tag_status,
            atomic_status,
            mfc_cmd_count,
            tag_update,
            tag_status_read,
            atomic_status_ready,
            in_mbox,
            out_mbox,
            lists,
            list_stall_status,
            mssync_tracking,
            mssync_horizon,
            pending_events,
            event_mask,
            event_count,
            event_levels,
        } = self;
        let mut channels = ChannelState::new();
        channels.mfc_lsa = *mfc_lsa;
        channels.mfc_eah = *mfc_eah;
        channels.mfc_eal = *mfc_eal;
        channels.mfc_size = *mfc_size;
        channels.mfc_tag_id = *mfc_tag_id;
        channels.tag_mask = *tag_mask;
        channels.tag_status = *tag_status;
        channels.atomic_status = *atomic_status;
        channels.cmd_queue_free = *mfc_cmd_count;
        channels.tag_update = match *tag_update {
            None => None,
            Some(MFC_TAG_UPDATE_ANY) => Some(TagUpdateCondition::Any),
            Some(MFC_TAG_UPDATE_ALL) => Some(TagUpdateCondition::All),
            Some(_) => return None,
        };
        channels.tag_status_read = *tag_status_read;
        channels.atomic_status_ready = *atomic_status_ready;
        channels.in_mbox.clone_from(in_mbox);
        channels.out_mbox = *out_mbox;
        channels.lists = lists
            .iter()
            .map(|list| list.to_cursor())
            .collect::<Option<_>>()?;
        channels.list_stall_status = *list_stall_status;
        channels.mssync_tracking = *mssync_tracking;
        channels.mssync_horizon = *mssync_horizon;
        channels.pending_events = *pending_events;
        channels.event_mask = *event_mask;
        channels.event_count = *event_count;
        channels.event_levels = *event_levels;
        Some(channels)
    }
}

impl From<&ListCursor> for SpuReferenceList {
    fn from(value: &ListCursor) -> Self {
        Self {
            word: value.word,
            tag: value.tag.raw(),
            direction: value.direction.into(),
            ordering: value.ordering.into(),
            eah: value.eah,
            element: value.element,
            remaining: value.remaining,
            data: value.data,
            stalled: value.stalled,
        }
    }
}

impl SpuReferenceList {
    fn to_cursor(self) -> Option<ListCursor> {
        Some(ListCursor {
            word: self.word,
            tag: MfcTagId::new(self.tag)?,
            direction: match self.direction {
                SpuReferenceDirection::Put => DmaDirection::Put,
                SpuReferenceDirection::Get => DmaDirection::Get,
            },
            ordering: match self.ordering {
                SpuReferenceOrdering::None => MfcOrdering::None,
                SpuReferenceOrdering::Fence => MfcOrdering::Fence,
                SpuReferenceOrdering::TagBarrier => MfcOrdering::TagBarrier,
                SpuReferenceOrdering::QueueBarrier => MfcOrdering::QueueBarrier,
            },
            eah: self.eah,
            element: self.element,
            remaining: self.remaining,
            data: self.data,
            stalled: self.stalled,
        })
    }
}

impl From<DmaDirection> for SpuReferenceDirection {
    fn from(value: DmaDirection) -> Self {
        match value {
            DmaDirection::Put => Self::Put,
            DmaDirection::Get => Self::Get,
        }
    }
}

impl From<MfcOrdering> for SpuReferenceOrdering {
    fn from(value: MfcOrdering) -> Self {
        match value {
            MfcOrdering::None => Self::None,
            MfcOrdering::Fence => Self::Fence,
            MfcOrdering::TagBarrier => Self::TagBarrier,
            MfcOrdering::QueueBarrier => Self::QueueBarrier,
        }
    }
}

impl From<&SignalNotifyRegister> for SpuReferenceSignal {
    fn from(value: &SignalNotifyRegister) -> Self {
        Self {
            mode: match value.mode {
                SignalNotifyMode::Overwrite => SpuReferenceSignalMode::Overwrite,
                SignalNotifyMode::LogicalOr => SpuReferenceSignalMode::LogicalOr,
            },
            word: value.word,
            pending: value.pending,
        }
    }
}

impl From<&SpuReferenceSignal> for SignalNotifyRegister {
    fn from(value: &SpuReferenceSignal) -> Self {
        Self {
            mode: match value.mode {
                SpuReferenceSignalMode::Overwrite => SignalNotifyMode::Overwrite,
                SpuReferenceSignalMode::LogicalOr => SignalNotifyMode::LogicalOr,
            },
            word: value.word,
            pending: value.pending,
        }
    }
}

impl From<&SpuStop> for SpuReferenceStop {
    fn from(value: &SpuStop) -> Self {
        Self {
            kind: match value.kind {
                SpuStopKind::Stop => SpuReferenceStopKind::Stop,
                SpuStopKind::Stopd => SpuReferenceStopKind::Stopd,
                SpuStopKind::Halt => SpuReferenceStopKind::Halt,
                SpuStopKind::InvalidInstruction => SpuReferenceStopKind::InvalidInstruction,
                SpuStopKind::InvalidChannel => SpuReferenceStopKind::InvalidChannel,
                SpuStopKind::Requested { waiting: false } => SpuReferenceStopKind::Requested,
                SpuStopKind::Requested { waiting: true } => SpuReferenceStopKind::RequestedWaiting,
            },
            code: value.code,
            npc: value.npc,
            interrupts_enabled: value.interrupts_enabled,
        }
    }
}

impl From<&SpuReferenceStop> for SpuStop {
    fn from(value: &SpuReferenceStop) -> Self {
        Self {
            kind: match value.kind {
                SpuReferenceStopKind::Stop => SpuStopKind::Stop,
                SpuReferenceStopKind::Stopd => SpuStopKind::Stopd,
                SpuReferenceStopKind::Halt => SpuStopKind::Halt,
                SpuReferenceStopKind::InvalidInstruction => SpuStopKind::InvalidInstruction,
                SpuReferenceStopKind::InvalidChannel => SpuStopKind::InvalidChannel,
                SpuReferenceStopKind::Requested => SpuStopKind::Requested { waiting: false },
                SpuReferenceStopKind::RequestedWaiting => SpuStopKind::Requested { waiting: true },
            },
            code: value.code,
            npc: value.npc,
            interrupts_enabled: value.interrupts_enabled,
        }
    }
}

impl From<MfcCommandError> for SpuReferenceMfcCause {
    fn from(value: MfcCommandError) -> Self {
        match value {
            MfcCommandError::ReservedTagBits(value) => Self::ReservedTagBits { value },
            MfcCommandError::ReservedSizeBits(value) => Self::ReservedSizeBits { value },
            MfcCommandError::SizeTooLarge(value) => Self::SizeTooLarge { value },
            MfcCommandError::SizeUnaligned(value) => Self::SizeUnaligned { value },
            MfcCommandError::SendSignalSize(value) => Self::SendSignalSize { value },
            MfcCommandError::LocalStoreUnaligned { lsa, size } => {
                Self::LocalStoreUnaligned { lsa, size }
            }
            MfcCommandError::AddressLowBitsDiffer { lsa, ea } => {
                Self::AddressLowBitsDiffer { lsa, ea }
            }
            MfcCommandError::ListAddressUnaligned(value) => Self::ListAddressUnaligned { value },
            MfcCommandError::ListSizeUnaligned(value) => Self::ListSizeUnaligned { value },
            MfcCommandError::ListElementCrosses4Gb { ea, size } => {
                Self::ListElementCrosses4gb { ea, size }
            }
            MfcCommandError::IllegalOpcode(value) => Self::IllegalOpcode { value },
            MfcCommandError::ReservedOpcode(value) => Self::ReservedOpcode { value },
            MfcCommandError::ProxyOnlyCommand(value) => Self::ProxyOnlyCommand { value },
            MfcCommandError::DataSegment { ea } => Self::DataSegment { ea },
            MfcCommandError::DataStorage { ea } => Self::DataStorage { ea },
        }
    }
}

impl From<&InvalidMfcCommand> for SpuReferenceInvalidCommand {
    fn from(value: &InvalidMfcCommand) -> Self {
        Self {
            word: value.word,
            lsa: value.params.lsa,
            eah: value.params.eah,
            eal: value.params.eal,
            size: value.params.size,
            tag: value.params.tag,
            error: value.error.into(),
        }
    }
}

impl From<&Effect> for SpuReferenceEffect {
    fn from(value: &Effect) -> Self {
        match value {
            Effect::SharedWriteIntent { range, bytes, .. } => Self::SharedWrite {
                ea: range.start().raw(),
                hex: to_hex(bytes.bytes()),
            },
            Effect::ConditionalStore { range, bytes, .. } => Self::ConditionalStore {
                ea: range.start().raw(),
                hex: to_hex(bytes.bytes()),
            },
            Effect::SharedReadIntent { range, .. } => Self::SharedRead {
                ea: range.start().raw(),
                size: range.length(),
            },
            Effect::ReservationAcquire { line_addr, .. } => {
                Self::ReservationAcquire { line: *line_addr }
            }
            Effect::MailboxPop { message, .. } => Self::MailboxPop {
                message: message.raw(),
            },
            Effect::DmaEnqueue { request, payload } => Self::Dma {
                direction: request.direction().into(),
                source: request.source().start().raw(),
                destination: request.destination().start().raw(),
                size: request.length(),
                local_store_source: request.local_store_source(),
                tag: request.tag_id().map(MfcTagId::raw),
                ordering: request.ordering().into(),
                stall_notify: request.stall_notify(),
                holds_slot: request.holds_slot(),
                payload: payload.as_deref().map(to_hex),
            },
            Effect::MfcInvalidCommand { command, .. } => Self::InvalidCommand {
                command: command.into(),
            },
            other => Self::Other {
                effect: format!("{:?}", other.kind()),
            },
        }
    }
}
