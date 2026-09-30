//! The vector-set form of the SPU reference: one architectural unit, its
//! named vectors, the whole architected state on both sides, typed
//! effects and a scripted outside world.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::types::SpuReferenceProvenance;

/// Schema version of the vector-set form.
pub const SPU_REFERENCE_SET_SCHEMA_VERSION: u32 = 6;

/// Most vectors one set holds.
pub const MAX_SPU_REFERENCE_VECTORS: usize = 64;

/// Most instruction words one vector places.
pub const MAX_SPU_REFERENCE_WORDS: usize = 64;

/// Most steps one vector runs.
pub const MAX_SPU_REFERENCE_STEPS: u32 = 4096;

/// Most main-memory bytes one world maps.
pub const MAX_SPU_REFERENCE_WORLD_BYTES: u64 = 1 << 20;

/// One reference file: the architectural unit and its vectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceSet {
    /// Version of this serialized schema.
    pub schema_version: u32,
    /// The architectural unit every vector exercises.
    pub unit: SpuReferenceUnit,
    /// The named vectors, each replayed on its own.
    pub vectors: Vec<SpuReferenceVector>,
}

/// The architectural unit a reference file covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpuReferenceUnit {
    /// One row of the SPU opcode map.
    Instruction {
        /// The row's mnemonic.
        mnemonic: String,
    },
    /// The words no row of the opcode map selects.
    UnassignedOpcodes,
    /// One channel the architecture defines.
    Channel {
        /// The channel number.
        number: u8,
    },
    /// The channel numbers the architecture reserves.
    ReservedChannels,
    /// One MFC command the SPU command queue accepts.
    MfcCommand {
        /// The command's opcode.
        opcode: u16,
    },
    /// The opcodes the SPU command queue does not accept: proxy-only,
    /// illegal and reserved.
    OutsideSpuQueue,
    /// A behaviour no single instruction, channel or command exercises.
    Facility {
        /// Which facility.
        name: SpuReferenceFacility,
    },
}

/// A multi-step SPU behaviour with a reference file of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpuReferenceFacility {
    /// The state a new context starts in.
    StartState,
    /// Local-store address and program-counter wrap by the LSLR.
    LslrWrap,
    /// The MFC SPU command queue.
    CommandQueue,
    /// MFC command parameter checks.
    ParameterValidation,
    /// Effective addresses that do not translate.
    TranslationErrors,
    /// List transfers.
    ListDma,
    /// The lock-line reservation.
    Reservation,
    /// Tag groups and the tag-status update.
    TagGroups,
    /// The event facility.
    Events,
    /// Interrupts and SRR0.
    Interrupts,
    /// Problem-state access from another processor.
    ProblemState,
    /// Transfers through another SPU's aliased local store.
    CrossSpuAliases,
    /// When a transfer reads or writes local store.
    DmaLocalStoreWindow,
}

impl SpuReferenceFacility {
    /// Every facility, in declaration order.
    pub const ALL: [Self; 13] = [
        Self::StartState,
        Self::LslrWrap,
        Self::CommandQueue,
        Self::ParameterValidation,
        Self::TranslationErrors,
        Self::ListDma,
        Self::Reservation,
        Self::TagGroups,
        Self::Events,
        Self::Interrupts,
        Self::ProblemState,
        Self::CrossSpuAliases,
        Self::DmaLocalStoreWindow,
    ];

    /// The name a reference file writes.
    pub const fn name(self) -> &'static str {
        match self {
            Self::StartState => "start_state",
            Self::LslrWrap => "lslr_wrap",
            Self::CommandQueue => "command_queue",
            Self::ParameterValidation => "parameter_validation",
            Self::TranslationErrors => "translation_errors",
            Self::ListDma => "list_dma",
            Self::Reservation => "reservation",
            Self::TagGroups => "tag_groups",
            Self::Events => "events",
            Self::Interrupts => "interrupts",
            Self::ProblemState => "problem_state",
            Self::CrossSpuAliases => "cross_spu_aliases",
            Self::DmaLocalStoreWindow => "dma_local_store_window",
        }
    }
}

/// One named vector: a program, its start state and world, a step
/// bound, and the expected end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceVector {
    /// Name, unique in its set.
    pub name: String,
    /// Source and acquisition information.
    pub provenance: SpuReferenceProvenance,
    /// Instruction words, placed in local store from the start PC.
    pub words: Vec<u32>,
    /// The state the SPU starts in.
    pub initial_state: SpuReferenceStart,
    /// The outside world the replay services the SPU from.
    #[serde(default)]
    pub world: SpuReferenceWorld,
    /// Most steps the replay runs. A step is one instruction, or one
    /// attempt at a blocked channel access.
    pub step_limit: u32,
    /// The expected end.
    pub expected: SpuReferenceFinal,
}

/// A run of bytes at an address, as lowercase hexadecimal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceBytes {
    /// Address of the first byte.
    pub at: u64,
    /// The bytes, two lowercase hexadecimal digits each.
    pub hex: String,
}

/// Why and where a stopped SPU stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceStop {
    /// What stopped the SPU.
    pub kind: SpuReferenceStopKind,
    /// The 14-bit stop code.
    pub code: u16,
    /// The local-store address the SPU resumes at.
    pub npc: u32,
    /// The interrupt-enable state the SPU resumes with.
    pub interrupts_enabled: bool,
}

/// What stopped the SPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpuReferenceStopKind {
    /// A `stop` instruction.
    Stop,
    /// A `stopd` instruction.
    Stopd,
    /// A halt whose condition held.
    Halt,
    /// A word that is not an SPU instruction.
    InvalidInstruction,
    /// A channel instruction the channel does not allow.
    InvalidChannel,
    /// A stop request while the SPU ran.
    Requested,
    /// A stop request while the SPU waited on a blocked channel.
    RequestedWaiting,
}

/// How a signal-notification register takes a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpuReferenceSignalMode {
    /// A write replaces the contents.
    Overwrite,
    /// A write ORs into the contents.
    LogicalOr,
}

/// One signal-notification register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceSignal {
    /// How a write changes `word`.
    pub mode: SpuReferenceSignalMode,
    /// The word the channel reads.
    pub word: u32,
    /// A write is unread, so the channel counts 1.
    pub pending: bool,
}

/// Direction of an MFC transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpuReferenceDirection {
    /// Local store to main storage.
    Put,
    /// Main storage to local store.
    Get,
}

/// The fence or barrier a queued command carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpuReferenceOrdering {
    /// No ordering.
    None,
    /// A tag-specific fence.
    Fence,
    /// A tag-specific barrier.
    TagBarrier,
    /// The barrier command.
    QueueBarrier,
}

/// A list command stopped at a stall-and-notify element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceList {
    /// The command word that queued the list.
    pub word: u32,
    /// The list's tag group, below 32.
    pub tag: u8,
    /// Put or get.
    pub direction: SpuReferenceDirection,
    /// The fence or barrier of the command's form.
    pub ordering: SpuReferenceOrdering,
    /// The effective-address high word every element shares.
    pub eah: u32,
    /// Local-store address of the next list element.
    pub element: u32,
    /// Elements not yet queued.
    pub remaining: u32,
    /// Local-store address the next element's transfer uses.
    pub data: u32,
    /// The stall-and-notify element completed.
    pub stalled: bool,
}

/// Every channel's data and count. A field left out takes its value in
/// a new context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpuReferenceChannelState {
    /// Staged local-store address.
    pub mfc_lsa: u32,
    /// Staged effective-address high word.
    pub mfc_eah: u32,
    /// Staged effective-address low word.
    pub mfc_eal: u32,
    /// Staged transfer size.
    pub mfc_size: u32,
    /// Staged tag identifier, as the program wrote it.
    pub mfc_tag_id: u32,
    /// Tag-query mask.
    pub tag_mask: u32,
    /// Tag groups with no outstanding transfer.
    pub tag_status: u32,
    /// Atomic-command status.
    pub atomic_status: u32,
    /// Free MFC command-queue slots.
    pub mfc_cmd_count: u32,
    /// The TS code of a waiting tag-status update request: 1 for any
    /// enabled group, 2 for all of them.
    pub tag_update: Option<u32>,
    /// The tag status a met update request latched and no read took.
    pub tag_status_read: Option<u32>,
    /// An atomic command's status is waiting to be read.
    pub atomic_status_ready: bool,
    /// Messages in the inbound mailbox, oldest first.
    pub in_mbox: Vec<u32>,
    /// The message in the outbound mailbox.
    pub out_mbox: Option<u32>,
    /// List commands stopped at a stall-and-notify element, oldest first.
    pub lists: Vec<SpuReferenceList>,
    /// Tag groups whose list stalled since the last read of the list
    /// stall-status channel.
    pub list_stall_status: u32,
    /// The DMA queue sequence a multisource synchronization request
    /// tracks up to.
    pub mssync_tracking: Option<u64>,
    /// The sequence a request made in the next step tracks up to.
    pub mssync_horizon: Option<u64>,
    /// The pending-event register.
    pub pending_events: u32,
    /// The event mask.
    pub event_mask: u32,
    /// The event-status channel counts 1.
    pub event_count: bool,
    /// Event sources whose channel count was nonzero when last looked at.
    pub event_levels: u32,
}

/// The state the SPU starts in: overrides of a new context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceStart {
    /// Register overrides as 32 lowercase hexadecimal digits.
    #[serde(default)]
    pub regs_hex: BTreeMap<String, String>,
    /// Local-store bytes, written before the instruction words.
    #[serde(default)]
    pub local_store: Vec<SpuReferenceBytes>,
    /// Initial program counter.
    pub pc: u32,
    /// Local storage limit register; absent means the whole store.
    #[serde(default)]
    pub lslr: Option<u32>,
    /// FPSCR as 32 lowercase hexadecimal digits; absent means zero.
    #[serde(default)]
    pub fpscr: Option<String>,
    /// A stopped state; absent means the SPU can run.
    #[serde(default)]
    pub stop: Option<SpuReferenceStop>,
    /// Interrupt-enable state.
    #[serde(default)]
    pub interrupts_enabled: bool,
    /// State save and restore register 0.
    #[serde(default)]
    pub srr0: u32,
    /// Signal-notification registers 1 and 2; absent means both reset.
    #[serde(default)]
    pub signals: Option<[SpuReferenceSignal; 2]>,
    /// Channel state; absent means a new context's.
    #[serde(default)]
    pub channels: Option<SpuReferenceChannelState>,
    /// Reserved-line address, held locally and in the committed table.
    #[serde(default)]
    pub reservation: Option<u64>,
}

/// The outside world the replay services the SPU from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpuReferenceWorld {
    /// Main-storage regions. Each run maps its bytes; any other address
    /// does not translate.
    pub memory: Vec<SpuReferenceBytes>,
    /// A second SPU in the replayed SPU's thread group.
    pub peer: Option<SpuReferencePeer>,
    /// Problem-state operations another processor makes, in step order.
    pub ppu: Vec<SpuReferencePpuWrite>,
}

/// A second SPU the replayed one reaches through the SPU thread window.
/// It does not run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpuReferencePeer {
    /// Its local-store bytes.
    pub local_store: Vec<SpuReferenceBytes>,
}

/// One problem-state operation and the step before which it lands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferencePpuWrite {
    /// The step it lands before; step 0 is before the first instruction.
    pub step: u32,
    /// What it does.
    pub action: SpuReferencePpuAction,
}

/// A problem-state operation on the replayed SPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpuReferencePpuAction {
    /// Write the inbound mailbox.
    InMbox {
        /// The message.
        value: u32,
    },
    /// Write a signal-notification register.
    Signal {
        /// 1 or 2.
        register: u8,
        /// The value.
        value: u32,
    },
    /// Set a signal-notification register's mode.
    SignalMode {
        /// 1 or 2.
        register: u8,
        /// OR mode when true, overwrite mode when false.
        logical_or: bool,
    },
    /// Read the outbound mailbox.
    ReadOutMbox,
    /// Request a stop.
    StopRequest,
    /// Write SPU_NPC.
    WriteNpc {
        /// The value, interrupt-enable state in bit 31.
        value: u32,
    },
    /// Restart a stopped SPU.
    Restart,
}

/// What a problem-state operation returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpuReferencePpuResult {
    /// The operation took effect.
    Done,
    /// A mailbox read returned this, or nothing.
    Read {
        /// The message, or `None` for an empty mailbox.
        value: Option<u32>,
    },
    /// The SPU refused the operation.
    Refused {
        /// Why.
        reason: SpuReferencePpuRefusal,
    },
}

/// Why the SPU refused a problem-state operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpuReferencePpuRefusal {
    /// The operation needs a stopped SPU.
    Running,
    /// A restart of an SPU that is not stopped.
    NotStopped,
    /// CellGov refused the SPU, so it holds no architected state.
    Refused,
    /// Any other refusal.
    Other,
}

/// How the replay ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpuReferenceEnd {
    /// The SPU stopped; `stop` holds why.
    Stopped,
    /// CellGov refused an instruction.
    Faulted {
        /// The guest fault code, if the fault carries one.
        code: Option<u32>,
    },
    /// The SPU waits on a blocked channel and nothing later wakes it.
    Stalled {
        /// The channel.
        channel: u8,
    },
    /// The step bound ran out.
    StepLimit,
}

/// Why the MFC refused a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpuReferenceMfcCause {
    /// The tag sets a reserved bit.
    ReservedTagBits {
        /// The tag.
        value: u32,
    },
    /// The size sets a reserved bit.
    ReservedSizeBits {
        /// The size.
        value: u32,
    },
    /// A size above 16 KB.
    SizeTooLarge {
        /// The size.
        value: u32,
    },
    /// A transfer size other than 0, 1, 2, 4, 8 or a multiple of 16.
    SizeUnaligned {
        /// The size.
        value: u32,
    },
    /// A sndsig size other than 4.
    SendSignalSize {
        /// The size.
        value: u32,
    },
    /// A local-store address not aligned for the size.
    LocalStoreUnaligned {
        /// The local-store address.
        lsa: u32,
        /// The size.
        size: u32,
    },
    /// The low four address bits differ between the two ends.
    AddressLowBitsDiffer {
        /// The local-store address.
        lsa: u32,
        /// The effective address.
        ea: u64,
    },
    /// A list address not doubleword aligned.
    ListAddressUnaligned {
        /// The list address.
        value: u32,
    },
    /// A list size that is not a multiple of 8.
    ListSizeUnaligned {
        /// The list size.
        value: u32,
    },
    /// A list element that crosses its 4 GB area.
    ListElementCrosses4gb {
        /// The element's effective address.
        ea: u64,
        /// The element's size.
        size: u32,
    },
    /// An opcode neither defined nor reserved.
    IllegalOpcode {
        /// The opcode.
        value: u32,
    },
    /// A reserved opcode.
    ReservedOpcode {
        /// The opcode.
        value: u32,
    },
    /// A proxy-queue command.
    ProxyOnlyCommand {
        /// The opcode.
        value: u32,
    },
    /// An effective address outside every segment.
    DataSegment {
        /// The effective address.
        ea: u64,
    },
    /// An effective address that does not translate for the access.
    DataStorage {
        /// The effective address.
        ea: u64,
    },
}

/// A command the MFC refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceInvalidCommand {
    /// The command word.
    pub word: u32,
    /// The latched local-store address.
    pub lsa: u32,
    /// The latched effective-address high word.
    pub eah: u32,
    /// The latched effective-address low word.
    pub eal: u32,
    /// The latched size.
    pub size: u32,
    /// The latched tag.
    pub tag: u32,
    /// The first check the command fails.
    pub error: SpuReferenceMfcCause,
}

/// One effect the SPU emitted, in emission order.
///
/// The emitting unit, the mailbox id and the guest-time stamp are left
/// out: the replay has one emitting SPU, and the time is the scheduler's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpuReferenceEffect {
    /// A store to main storage.
    SharedWrite {
        /// Effective address.
        ea: u64,
        /// The bytes.
        hex: String,
    },
    /// A conditional store that succeeded.
    ConditionalStore {
        /// Effective address.
        ea: u64,
        /// The bytes.
        hex: String,
    },
    /// A read of main storage.
    SharedRead {
        /// Effective address.
        ea: u64,
        /// Bytes read.
        size: u64,
    },
    /// A lock-line reservation taken.
    ReservationAcquire {
        /// Address in the line.
        line: u64,
    },
    /// An inbound-mailbox message taken.
    MailboxPop {
        /// The message.
        message: u32,
    },
    /// An MFC command queued.
    Dma {
        /// Put or get.
        direction: SpuReferenceDirection,
        /// Source address: a local-store offset when the local store is
        /// the source.
        source: u64,
        /// Destination address.
        destination: u64,
        /// Bytes moved.
        size: u64,
        /// The source is the issuer's local store.
        local_store_source: bool,
        /// The tag group, below 32.
        tag: Option<u8>,
        /// Fence or barrier.
        ordering: SpuReferenceOrdering,
        /// A list element that stalls its list when it completes.
        stall_notify: bool,
        /// The command holds a command-queue slot.
        holds_slot: bool,
        /// Bytes the command fixes at issue.
        payload: Option<String>,
    },
    /// An MFC command the MFC refuses, queued.
    InvalidCommand {
        /// The command.
        command: SpuReferenceInvalidCommand,
    },
    /// An effect the SPU is not expected to emit.
    Other {
        /// Its kind.
        effect: String,
    },
}

/// The other SPU at the end of the replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferencePeerState {
    /// Local-store overrides of its initial local store.
    pub local_store: Vec<SpuReferenceBytes>,
    /// Its signal-notification registers.
    pub signals: [SpuReferenceSignal; 2],
    /// Messages in its inbound mailbox, oldest first.
    pub in_mbox: Vec<u32>,
}

/// An expected component: one value, a set of legal values, or the
/// reason it is not compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpuField<T> {
    /// The one expected value.
    Value {
        /// Expected value.
        value: T,
    },
    /// The architecture leaves the value open between these. The replay
    /// checks the result is one of them, and separately that it is the
    /// one CellGov documents choosing.
    OneOf {
        /// The legal values, at least two.
        values: Vec<T>,
        /// Index in `values` of the one CellGov documents choosing.
        chosen: usize,
        /// Why the architecture leaves the value open.
        reason: String,
    },
    /// The architecture leaves the value undefined.
    Undefined {
        /// Source-specific reason.
        reason: String,
    },
    /// The source cannot represent the value.
    Unsupported {
        /// Source-specific reason.
        reason: String,
    },
}

/// The expected end: every component of the SPU's context, what it did,
/// and what it left in the world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpuReferenceFinal {
    /// How the replay ended.
    pub end: SpuField<SpuReferenceEnd>,
    /// Register overrides of the initial bank; the whole bank compares.
    pub regs_hex: SpuField<BTreeMap<String, String>>,
    /// Local-store overrides of the loaded store; the whole store compares.
    pub local_store: SpuField<Vec<SpuReferenceBytes>>,
    /// Program counter.
    pub pc: SpuField<u32>,
    /// Local storage limit register.
    pub lslr: SpuField<u32>,
    /// FPSCR as 32 lowercase hexadecimal digits.
    pub fpscr: SpuField<String>,
    /// The stopped state.
    pub stop: SpuField<Option<SpuReferenceStop>>,
    /// Interrupt-enable state.
    pub interrupts_enabled: SpuField<bool>,
    /// State save and restore register 0.
    pub srr0: SpuField<u32>,
    /// Signal-notification registers 1 and 2.
    pub signals: SpuField<[SpuReferenceSignal; 2]>,
    /// Every channel's data and count.
    pub channels: SpuField<SpuReferenceChannelState>,
    /// Local reserved-line address.
    pub reservation: SpuField<Option<u64>>,
    /// Every effect, in emission order.
    pub effects: SpuField<Vec<SpuReferenceEffect>>,
    /// Main-storage overrides of the world's regions; every region
    /// compares.
    pub main_memory: SpuField<Vec<SpuReferenceBytes>>,
    /// The other SPU; a value needs a peer in the world.
    pub peer: SpuField<SpuReferencePeerState>,
    /// One result per problem-state operation, in order.
    pub ppu_results: SpuField<Vec<SpuReferencePpuResult>>,
    /// Commands the MFC refused when its queue reached them, in order.
    pub mfc_exceptions: SpuField<Vec<SpuReferenceInvalidCommand>>,
}

/// A reference file of either form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpuReferenceFile {
    /// A single-vector file.
    Single(Box<super::types::SpuReferenceArtifact>),
    /// A vector set.
    Set(SpuReferenceSet),
}
