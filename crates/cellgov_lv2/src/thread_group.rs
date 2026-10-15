//! Managed SPU thread group table.
//!
//! Owns group lifecycle from create through finish. Group ids are
//! monotonic u32 tokens starting at 1; 0 is reserved.

use crate::dispatch::{SpuInitState, SpuLoadImage};
use crate::image::SpuImageHandle;
use cellgov_event::UnitId;
use cellgov_mem::lanes::{self, source, LaneEntryMut, LaneMap, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv2::spu::{event, group_join_cause, group_priority, group_type, pool};
use std::collections::BTreeMap;

/// Cap on slots per group; the `group_id * 256 + slot` thread-id
/// encoding aliases adjacent groups' ranges above this.
///
/// `sys_spu_thread_group_create` holds a group to the SPU pool, at most
/// [`pool::USABLE_SPUS`] threads, so no group a create admits reaches
/// this cap; it bounds the slot indices `sys_spu_thread_initialize`
/// names.
pub const MAX_SLOTS_PER_GROUP: u32 = 256;

const _: () = assert!(pool::USABLE_SPUS <= MAX_SLOTS_PER_GROUP);

/// A single slot within a thread group.
#[derive(Debug, Clone)]
pub struct ThreadSlot {
    /// Image handle from `sys_spu_image_open`.
    pub image_handle: SpuImageHandle,
    /// Snapshot of the argument block at initialize time; the PPU
    /// may reuse the same stack variable across initialize calls.
    pub args: [u64; 4],
    /// `None` until `sys_spu_thread_group_start` resolves the image.
    pub init: Option<SpuInitState>,
    /// The thread's `SPU_Cfg` signal configuration: bit 0 puts
    /// signal-notification register 1 in OR mode, bit 1 register 2. 0,
    /// both registers overwriting, until `sys_spu_thread_set_spu_cfg`.
    ///
    /// [CBEA p:239 s:16.4] each register either overwrites its contents or ORs the data written into them.
    pub signal_config: u64,
    /// The event queue each connected SPU event port delivers to, by
    /// port number below [`event::PORT_COUNT`].
    pub event_ports: BTreeMap<u32, u32>,
    /// The event queues the SPU names by number in its own receives,
    /// at most [`event::QUEUE_BINDING_COUNT`] of them.
    pub queue_bindings: BTreeMap<u32, u32>,
}

/// Lifecycle state of a thread group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupState {
    /// Created but not yet started.
    Created,
    /// Started; SPUs are running.
    Running,
    /// All SPUs finished.
    Finished,
}

/// A managed SPU thread group.
#[derive(Debug, Clone)]
pub struct ThreadGroup {
    /// Group id.
    pub id: u32,
    /// Expected SPU slot count, declared at create time.
    pub num_threads: u32,
    /// Populated slots, keyed by slot index.
    pub slots: BTreeMap<u32, ThreadSlot>,
    /// Current lifecycle state.
    pub state: GroupState,
    /// Drives the terminal transition to [`GroupState::Finished`]
    /// when it reaches 0.
    pub remaining_unfinished: u32,
    /// The `cause` a join of the finished group reads.
    pub join_cause: u32,
    /// The `status` a join of the finished group reads.
    pub exit_status: u32,
    /// The status each thread gave `sys_spu_thread_exit`, by raw unit
    /// id.
    pub thread_exit_status: BTreeMap<u64, u32>,
    /// The event queue each connected group event type delivers to.
    pub event_queues: BTreeMap<u32, u32>,
    /// The `type` word of the group's attribute at create.
    pub group_type: u32,
    /// The priority given at create or by
    /// `sys_spu_thread_group_set_priority`. Served back, never a
    /// scheduling input.
    pub priority: i32,
    /// Whether `sys_spu_thread_group_suspend` holds the running group.
    pub suspended: bool,
    /// The status each thread gave `sys_spu_thread_exit` in an earlier
    /// run of the group, by slot. A thread that has not exited in the
    /// current run reports it.
    pub previous_exit_status: BTreeMap<u32, u32>,
}

/// What `sys_spu_thread_group_create` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupRequest {
    /// Thread count.
    pub num_threads: u32,
    /// The `type` word of the group's attribute.
    pub group_type: u32,
    /// Group priority.
    pub priority: i32,
}

/// Whether `priority` is one a process, with root permission when
/// `root`, may give a group with a scheduler context.
fn priority_in_range(priority: i32, root: bool) -> bool {
    let min = if root {
        group_priority::MIN_ROOT
    } else {
        group_priority::MIN_USER
    };
    (min..=group_priority::MAX).contains(&priority)
}

/// How a thread group occupies the SPU pool, from its `type` word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// The scheduler context-switches the group, so all such groups
    /// together need as many SPUs as the largest one.
    Context,
    /// The group holds one SPU per thread while it lives.
    NonContext,
    /// The group shares one SPU with the system and holds one SPU per
    /// other thread.
    CooperateWithSystem,
}

impl GroupKind {
    /// The kind a group attribute's `type` word selects.
    pub fn of_type(group_type: u32) -> Self {
        if group_type & group_type::COOPERATE_WITH_SYSTEM != 0 {
            Self::CooperateWithSystem
        } else if group_type & group_type::NON_CONTEXT != 0 {
            Self::NonContext
        } else {
            Self::Context
        }
    }
}

/// What a set of groups needs from the SPU pool.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PoolDemand {
    /// SPUs held for the whole life of a group.
    physical: u32,
    /// SPUs the context-switched groups need at once.
    controllable: u32,
    /// Groups that share an SPU with the system.
    cooperating: u32,
}

impl PoolDemand {
    /// The demand of one group of `num` threads.
    fn of(kind: GroupKind, num: u32) -> Self {
        match kind {
            GroupKind::Context => Self {
                controllable: num,
                ..Self::default()
            },
            GroupKind::NonContext => Self {
                physical: num,
                ..Self::default()
            },
            GroupKind::CooperateWithSystem => Self {
                physical: num.saturating_sub(1),
                controllable: 1,
                cooperating: 1,
            },
        }
    }

    /// The demand of `self` and `other` together: physical SPUs add up,
    /// and context-switched groups need the larger of the two.
    fn with(self, other: Self) -> Self {
        Self {
            physical: self.physical.saturating_add(other.physical),
            controllable: self.controllable.max(other.controllable),
            cooperating: self.cooperating.saturating_add(other.cooperating),
        }
    }

    /// Whether the pool holds this demand with `spu_limit` SPUs for SPU
    /// threads. The physical bound is strict: a demand that takes every
    /// SPU is refused.
    fn fits(self, spu_limit: u32) -> bool {
        self.physical < spu_limit && self.controllable <= spu_limit && self.cooperating <= 1
    }
}

impl ThreadGroup {
    /// How the group occupies the SPU pool.
    pub fn kind(&self) -> GroupKind {
        GroupKind::of_type(self.group_type)
    }

    /// Whether the group takes `sys_spu_thread_group_suspend` and
    /// `sys_spu_thread_group_resume`: a scheduler-context group whose
    /// type carries no [`group_type::NO_RUN_CONTROL_MASK`] bit.
    fn takes_run_control(&self) -> bool {
        self.kind() == GroupKind::Context && self.group_type & group_type::NO_RUN_CONTROL_MASK == 0
    }

    /// Whether every declared slot is initialized.
    pub fn is_initialized(&self) -> bool {
        self.slots.len() as u32 >= self.num_threads
    }

    /// Whether no initialized thread has event port `port` connected.
    pub fn port_is_free(&self, port: u32) -> bool {
        self.slots
            .values()
            .all(|slot| !slot.event_ports.contains_key(&port))
    }
}

impl LaneValue for ThreadGroup {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.id));
        lanes.lane(2, 0, u64::from(self.num_threads));
        lanes.lane(3, 0, u64::from(self.remaining_unfinished));
        let state = match self.state {
            GroupState::Created => 1,
            GroupState::Running => 2,
            GroupState::Finished => 3,
        };
        lanes.lane(4, 0, state);
        lanes.lane(5, 0, self.slots.len() as u64);
        lanes.lane(23, 0, u64::from(self.join_cause));
        lanes.lane(24, 0, u64::from(self.exit_status));
        for (&unit, &status) in &self.thread_exit_status {
            lanes.lane(25, unit, u64::from(status));
        }
        for (&event_type, &queue) in &self.event_queues {
            lanes.lane(27, u64::from(event_type), u64::from(queue));
        }
        if self.group_type != group_type::NORMAL {
            lanes.lane(30, 0, u64::from(self.group_type));
        }
        if self.priority != 0 {
            lanes.lane(31, 0, u64::from(self.priority as u32));
        }
        if self.suspended {
            lanes.lane(32, 0, 1);
        }
        for (&slot, &status) in &self.previous_exit_status {
            lanes.lane(33, u64::from(slot), u64::from(status));
        }
        for (&index, slot) in &self.slots {
            let index = u64::from(index);
            lanes.lane(6, index, 1);
            lanes.lane(7, index, u64::from(slot.image_handle.raw()));
            for (i, arg) in slot.args.iter().enumerate() {
                lanes.lane(8 + i as u8, index, *arg);
            }
            if slot.signal_config != 0 {
                lanes.lane(26, index, slot.signal_config);
            }
            for (&port, &queue) in &slot.event_ports {
                lanes.lane(28, (index << 32) | u64::from(port), u64::from(queue));
            }
            for (&number, &queue) in &slot.queue_bindings {
                lanes.lane(29, (index << 32) | u64::from(number), u64::from(queue));
            }
            let Some(init) = &slot.init else {
                continue;
            };
            lanes.lane(12, index, 1);
            lanes.lane(13, index, u64::from(init.entry_pc));
            lanes.lane(14, index, u64::from(init.stack_ptr));
            for (i, arg) in init.args.iter().enumerate() {
                lanes.lane(15 + i as u8, index, *arg);
            }
            lanes.lane(19, index, u64::from(init.group_id));
            match &init.image {
                SpuLoadImage::Elf(bytes) => {
                    lanes.lane(20, index, 1);
                    lanes.bytes(21, &[index, 0], bytes);
                }
                SpuLoadImage::Segments(segments) => {
                    lanes.lane(20, index, 2);
                    lanes.lane(22, index, segments.len() as u64);
                    for (j, segment) in segments.iter().enumerate() {
                        lanes.bytes(
                            21,
                            &[index, 1 + j as u64, u64::from(segment.ls_start)],
                            &segment.bytes,
                        );
                    }
                }
            }
        }
    }
}

/// Failure modes of [`ThreadGroupTable::bind_thread_queue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BindQueueError {
    /// -> `CELL_ESRCH`: no initialized thread has the id.
    #[error("no initialized SPU thread with that id")]
    UnknownThread,
    /// -> `CELL_EBUSY`: the number or the queue is already bound.
    #[error("the queue number or the queue is already bound on the thread")]
    Busy,
    /// -> `CELL_EAGAIN`: the thread holds
    /// [`event::QUEUE_BINDING_COUNT`] bindings.
    #[error("the thread holds its full count of queue bindings")]
    Full,
}

/// Failure modes of [`ThreadGroupTable::unbind_thread_queue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UnbindQueueError {
    /// -> `CELL_ESRCH`: no initialized thread has the id.
    #[error("no initialized SPU thread with that id")]
    UnknownThread,
    /// -> `CELL_ESRCH`: the thread binds no queue under that number.
    #[error("no queue is bound under that number")]
    NotBound,
}

/// Failure modes of [`ThreadGroupTable::initialize_thread`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InitializeThreadError {
    /// No group with this id.
    #[error("unknown thread group")]
    UnknownGroup,
    /// Slots can only be populated while the group is
    /// [`GroupState::Created`].
    #[error("thread group already started (state {state:?})")]
    GroupAlreadyStarted {
        /// Current group state.
        state: GroupState,
    },
    /// Every one of the `num_threads` slots declared at create time
    /// is already populated; the slot index itself may be any value
    /// below [`MAX_SLOTS_PER_GROUP`].
    #[error("thread group full (declared {num_threads} threads)")]
    GroupFull {
        /// The group's declared slot count.
        num_threads: u32,
    },
    /// Slot index is `>= MAX_SLOTS_PER_GROUP`.
    #[error("slot index out of supported range")]
    SlotOutOfRange,
    /// The slot already has an entry.
    #[error("slot already initialized")]
    SlotAlreadyInitialized,
}

/// Failure modes of [`ThreadGroupTable::record_spu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecordSpuError {
    /// No group with this id.
    #[error("unknown thread group")]
    UnknownGroup,
    /// Registering against a finished group would inflate the
    /// counter without triggering the terminal transition.
    #[error("thread group already finished")]
    GroupAlreadyFinished,
    /// `unit_id` is already registered in some group, or was
    /// registered in one that has since finished.
    #[error("SPU unit already registered")]
    DuplicateUnit,
    /// `(group_id, slot)` already maps to a different unit.
    #[error("(group, slot) already maps to a different unit")]
    ThreadIdCollision,
    /// Slot index is `>= MAX_SLOTS_PER_GROUP`.
    #[error("slot index out of supported range")]
    SlotOutOfRange,
    /// `group_id * 256 + slot` overflows `u32`.
    #[error("group_id * 256 + slot overflowed u32")]
    ThreadIdOverflow,
}

/// Failure modes of [`ThreadGroupTable::notify_spu_finished`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NotifySpuFinishedError {
    /// `unit_id` was never registered as an SPU. Benign for
    /// non-SPU units; callers iterating every unit can ignore.
    #[error("unknown SPU unit")]
    UnknownUnit,
    /// `unit_id` was registered and its notify already fired.
    /// Distinct from [`Self::UnknownUnit`] so a double-notify
    /// against a live unit does not masquerade as a
    /// non-SPU-unit call.
    #[error("SPU notify already fired (group {group_id})")]
    AlreadyFinished {
        /// Group this unit belonged to.
        group_id: u32,
    },
    /// Owning group is not in [`GroupState::Running`]
    /// (finish-before-start).
    #[error("thread group not running (state {state:?})")]
    GroupNotRunning {
        /// Current group state.
        state: GroupState,
    },
    /// `remaining_unfinished` was already 0.
    #[error("remaining_unfinished was already 0")]
    CounterUnderflow,
}

/// Failure modes of [`ThreadGroupTable::destroy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DestroyGroupError {
    /// Group id does not exist (`sys_spu_thread_group_destroy` -> CELL_ESRCH).
    #[error("unknown thread group")]
    Unknown,
    /// Group is in [`GroupState::Running`]; the title must terminate
    /// or join it first (`sys_spu_thread_group_destroy` -> CELL_EBUSY).
    #[error("thread group is running")]
    Busy,
}

/// Failure modes of [`ThreadGroupTable::create_typed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CreateGroupError {
    /// -> `CELL_EINVAL`: a type outside [`group_type::USER_TYPES`], and
    /// for a process with root permission outside
    /// [`group_type::ROOT_TYPES`] too; or a group that can never fit the
    /// pool: more threads than [`pool::USABLE_SPUS`] (one for a root type
    /// with a [`group_type::SINGLE_THREAD_MASK`] bit), a demand above the
    /// SPU-thread limit, or a group sharing with the system of fewer than
    /// two threads.
    #[error("the group type is refused or the group can never fit the SPU pool")]
    Invalid,
    /// -> `CELL_EBUSY`: the groups that exist leave no room for it.
    #[error("the SPU pool has no room for the group")]
    Busy,
    /// -> `CELL_EAGAIN`: the u32 id space is exhausted.
    #[error("the thread-group id space is exhausted")]
    Exhausted,
}

/// Failure modes of [`ThreadGroupTable::suspend`],
/// [`ThreadGroupTable::resume`] and [`ThreadGroupTable::check_yield`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RunControlError {
    /// -> `CELL_ESRCH`: no group has the id.
    #[error("unknown thread group")]
    Unknown,
    /// -> `CELL_EINVAL`: the group has no scheduler context, shares an
    /// SPU with the system, or its type takes no run control.
    #[error("the thread group takes no suspend or resume")]
    NoRunControl,
    /// -> `CELL_ESTAT`: the group is not in a state the call acts on.
    #[error("the thread group is not in a state the call acts on")]
    NotRunning,
}

/// Failure modes of [`ThreadGroupTable::set_priority`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SetPriorityError {
    /// -> `CELL_ESRCH`: no group has the id.
    #[error("unknown thread group")]
    Unknown,
    /// -> `CELL_EINVAL`: the group has no scheduler context, shares an
    /// SPU with the system, or the priority is out of range.
    #[error("the group takes no priority, or the priority is out of range")]
    Invalid,
}

/// Failure modes of [`ThreadGroupTable::set_raw_limit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SetLimitsError {
    /// -> `CELL_EINVAL`: more than [`pool::RAW_SPU_MAX`] Raw SPUs.
    #[error("max_raw_spu {max_raw} exceeds the Raw SPU maximum")]
    Invalid {
        /// The requested Raw SPU count.
        max_raw: u32,
    },
    /// -> `CELL_EBUSY`: the groups or the Raw SPUs that exist do not fit
    /// the new split.
    #[error("the existing thread groups or Raw SPUs do not fit max_raw_spu {max_raw}")]
    Busy {
        /// The requested Raw SPU count.
        max_raw: u32,
    },
}

/// Failure modes of [`ThreadGroupTable::terminate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TerminateGroupError {
    /// -> `CELL_ESRCH`: no group has the id.
    #[error("unknown thread group")]
    Unknown,
    /// -> `CELL_ESTAT`: the group is not running.
    #[error("thread group not running (state {state:?})")]
    NotRunning {
        /// Current group state.
        state: GroupState,
    },
}

/// Failure modes of [`ThreadGroupTable::thread_exit_status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExitStatusError {
    /// -> `CELL_ESRCH`: no initialized thread has the id.
    #[error("no initialized SPU thread with that id")]
    UnknownThread,
    /// -> `CELL_ESTAT`: the thread did not call `sys_spu_thread_exit`.
    #[error("the SPU thread has no exit status")]
    NoStatus,
}

/// Table of managed SPU thread groups.
///
/// `unit_to_group` and `finished_units` partition the SPU
/// registration space: a unit lives in exactly one of them, and
/// `notify_spu_finished` moves entries from the former to the
/// latter. Keeping them distinct is what lets a double-notify
/// surface as [`NotifySpuFinishedError::AlreadyFinished`] rather
/// than re-decrementing the counter.
#[derive(Debug, Clone)]
pub struct ThreadGroupTable {
    groups: LaneMap<u32, ThreadGroup>,
    unit_to_group: LaneMap<UnitId, u32>,
    finished_units: LaneMap<UnitId, u32>,
    /// Synthetic `group_id * 256 + slot` -> raw runtime `UnitId`.
    thread_id_to_unit: LaneMap<u32, u64>,
    next_id: u32,
    /// The physical SPUs `sys_spu_initialize` set aside for Raw SPUs;
    /// the rest of [`pool::USABLE_SPUS`] runs SPU threads.
    max_raw: u32,
}

impl Default for ThreadGroupTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadGroupTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self {
            groups: LaneMap::new(source::THREAD_GROUP, u64::from),
            unit_to_group: LaneMap::new(source::GROUP_UNIT, UnitId::raw),
            finished_units: LaneMap::new(source::GROUP_FINISHED_UNIT, UnitId::raw),
            thread_id_to_unit: LaneMap::new(source::GROUP_THREAD_ID, u64::from),
            next_id: 1,
            max_raw: 0,
        }
    }

    /// Create a normal group with `num_threads` slots, outside the SPU
    /// pool accounting.
    ///
    /// Returns `None` if the u32 id space is exhausted.
    pub fn create(&mut self, num_threads: u32) -> Option<u32> {
        self.insert_group(GroupRequest {
            num_threads,
            group_type: group_type::NORMAL,
            priority: 0,
        })
    }

    /// Create the group `request` describes, for a process with root
    /// permission when `root`, if the SPU pool has room for it.
    ///
    /// # Errors
    /// See [`CreateGroupError`]; an error creates nothing.
    pub fn create_typed(
        &mut self,
        request: GroupRequest,
        root: bool,
    ) -> Result<u32, CreateGroupError> {
        self.check_valid(request, root)?;
        let demand = PoolDemand::of(GroupKind::of_type(request.group_type), request.num_threads);
        if !self.occupied().with(demand).fits(self.spu_limit()) {
            return Err(CreateGroupError::Busy);
        }
        self.insert_group(request)
            .ok_or(CreateGroupError::Exhausted)
    }

    /// Whether a process, with root permission when `root`, may create
    /// the group `request` describes, and whether the group can ever fit
    /// the pool, whatever groups exist.
    ///
    /// # Errors
    /// [`CreateGroupError::Invalid`] when it cannot.
    pub fn check_valid(&self, request: GroupRequest, root: bool) -> Result<(), CreateGroupError> {
        let GroupRequest {
            num_threads,
            group_type,
            priority,
        } = request;
        let root_type = group_type::ROOT_TYPES.contains(&group_type);
        if !(group_type::USER_TYPES.contains(&group_type) || (root && root_type)) {
            return Err(CreateGroupError::Invalid);
        }
        let max_threads = if root_type && group_type & group_type::SINGLE_THREAD_MASK != 0 {
            1
        } else {
            pool::USABLE_SPUS
        };
        let kind = GroupKind::of_type(group_type);
        let demand = PoolDemand::of(kind, num_threads);
        let spu_limit = self.spu_limit();
        if num_threads > max_threads
            || (kind == GroupKind::Context && !priority_in_range(priority, root))
            || demand.physical > spu_limit
            || demand.controllable > spu_limit
            || (kind == GroupKind::CooperateWithSystem && num_threads < 2)
        {
            return Err(CreateGroupError::Invalid);
        }
        Ok(())
    }

    /// Give the group `group_id` priority `priority`, as
    /// `sys_spu_thread_group_set_priority` does for a process with root
    /// permission when `root`. The priority is served back and is not a
    /// scheduling input: CellGov runs every SPU thread as its own unit, so
    /// the contest for SPUs it decides on the console never arises.
    ///
    /// # Errors
    /// See [`SetPriorityError`]; an error keeps the old priority.
    pub fn set_priority(
        &mut self,
        group_id: u32,
        priority: i32,
        root: bool,
    ) -> Result<(), SetPriorityError> {
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or(SetPriorityError::Unknown)?;
        if group.kind() != GroupKind::Context || !priority_in_range(priority, root) {
            return Err(SetPriorityError::Invalid);
        }
        group.priority = priority;
        Ok(())
    }

    /// The priority `sys_spu_thread_group_get_priority` reads for the
    /// group `group_id`: 0 for a group without a scheduler context,
    /// [`group_priority::COOPERATING`] for one that shares an SPU with the
    /// system, and the stored priority otherwise. `None` when no group
    /// has the id.
    pub fn priority(&self, group_id: u32) -> Option<i32> {
        let group = self.groups.get(group_id)?;
        Some(match group.kind() {
            GroupKind::NonContext => 0,
            GroupKind::CooperateWithSystem => group_priority::COOPERATING,
            GroupKind::Context => group.priority,
        })
    }

    /// The SPUs left for SPU threads once Raw SPUs take their share.
    pub fn spu_limit(&self) -> u32 {
        pool::USABLE_SPUS - self.max_raw
    }

    /// The physical SPUs set aside for Raw SPUs.
    pub fn raw_limit(&self) -> u32 {
        self.max_raw
    }

    /// Set aside `max_raw` physical SPUs for Raw SPUs, as
    /// `sys_spu_initialize` does, while `live_raw` Raw SPUs exist.
    ///
    /// # Errors
    /// See [`SetLimitsError`]; an error keeps the old split.
    // The console capture of tests/micro/spu_raw_lifecycle answers
    // CELL_EBUSY to a limit of 1 while two Raw SPUs exist.
    pub fn set_raw_limit(&mut self, max_raw: u32, live_raw: u32) -> Result<(), SetLimitsError> {
        if max_raw > pool::RAW_SPU_MAX {
            return Err(SetLimitsError::Invalid { max_raw });
        }
        if live_raw > max_raw || !self.occupied().fits(pool::USABLE_SPUS - max_raw) {
            return Err(SetLimitsError::Busy { max_raw });
        }
        self.max_raw = max_raw;
        Ok(())
    }

    /// What every group that exists needs from the SPU pool. A group
    /// keeps its share until it is destroyed.
    fn occupied(&self) -> PoolDemand {
        self.groups
            .values()
            .fold(PoolDemand::default(), |acc, group| {
                acc.with(PoolDemand::of(group.kind(), group.num_threads))
            })
    }

    fn insert_group(&mut self, request: GroupRequest) -> Option<u32> {
        let GroupRequest {
            num_threads,
            group_type,
            priority,
        } = request;
        let id = self.next_id;
        if id == 0 {
            return None;
        }
        self.next_id = id.wrapping_add(1);
        let group = ThreadGroup {
            id,
            num_threads,
            slots: BTreeMap::new(),
            state: GroupState::Created,
            remaining_unfinished: 0,
            join_cause: group_join_cause::ALL_THREADS_EXIT,
            exit_status: 0,
            thread_exit_status: BTreeMap::new(),
            event_queues: BTreeMap::new(),
            group_type,
            priority,
            suspended: false,
            previous_exit_status: BTreeMap::new(),
        };
        self.groups.insert(id, group);
        Some(id)
    }

    /// Populate a slot in a group (from `sys_spu_thread_initialize`).
    ///
    /// # Errors
    /// See [`InitializeThreadError`].
    pub fn initialize_thread(
        &mut self,
        group_id: u32,
        slot: u32,
        image_handle: SpuImageHandle,
        args: [u64; 4],
    ) -> Result<(), InitializeThreadError> {
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or(InitializeThreadError::UnknownGroup)?;
        if group.state != GroupState::Created {
            return Err(InitializeThreadError::GroupAlreadyStarted { state: group.state });
        }
        if slot >= MAX_SLOTS_PER_GROUP {
            return Err(InitializeThreadError::SlotOutOfRange);
        }
        if group.slots.contains_key(&slot) {
            return Err(InitializeThreadError::SlotAlreadyInitialized);
        }
        // The slot index is the SPU number, unique within the group;
        // its ceiling is MAX_SLOTS_PER_GROUP, which is CellGov's own.
        // Only the populated count is capped against the group's
        // declared thread count, so any index below that ceiling is
        // accepted.
        if group.slots.len() as u32 >= group.num_threads {
            return Err(InitializeThreadError::GroupFull {
                num_threads: group.num_threads,
            });
        }
        group.slots.insert(
            slot,
            ThreadSlot {
                image_handle,
                args,
                init: None,
                signal_config: 0,
                event_ports: BTreeMap::new(),
                queue_bindings: BTreeMap::new(),
            },
        );
        Ok(())
    }

    /// Look up a group by id.
    pub fn get(&self, group_id: u32) -> Option<&ThreadGroup> {
        self.groups.get(group_id)
    }

    /// Mutably look up a group by id.
    pub fn get_mut(&mut self, group_id: u32) -> Option<LaneEntryMut<'_, u32, ThreadGroup>> {
        self.groups.get_mut(group_id)
    }

    /// Withdraw a group whose state allows destruction.
    ///
    /// Mirrors `sys_spu_thread_group_destroy`: a group in
    /// [`GroupState::Running`] is in-flight and reports
    /// [`DestroyGroupError::Busy`]; an unknown id reports
    /// [`DestroyGroupError::Unknown`]. The unit / thread-id maps for
    /// the group's slots are scrubbed so a future `create` reusing
    /// the same id starts clean.
    pub fn destroy(&mut self, group_id: u32) -> Result<(), DestroyGroupError> {
        match self.groups.get(group_id) {
            None => return Err(DestroyGroupError::Unknown),
            Some(g) if g.state == GroupState::Running => return Err(DestroyGroupError::Busy),
            Some(_) => {}
        }
        self.groups.remove(group_id);
        self.unit_to_group.retain(|_, gid| *gid != group_id);
        self.finished_units.retain(|_, gid| *gid != group_id);
        self.thread_id_to_unit
            .retain(|tid, _| tid / MAX_SLOTS_PER_GROUP != group_id);
        Ok(())
    }

    /// Number of groups.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Register `unit_id` as an SPU in `group_id` at `slot`.
    ///
    /// All validation runs before any insert, so `Err` leaves the
    /// table untouched.
    ///
    /// # Errors
    /// See [`RecordSpuError`].
    pub fn record_spu(
        &mut self,
        unit_id: UnitId,
        group_id: u32,
        slot: u32,
    ) -> Result<(), RecordSpuError> {
        {
            let group = self
                .groups
                .get(group_id)
                .ok_or(RecordSpuError::UnknownGroup)?;
            if group.state == GroupState::Finished {
                return Err(RecordSpuError::GroupAlreadyFinished);
            }
        }
        if slot >= MAX_SLOTS_PER_GROUP {
            return Err(RecordSpuError::SlotOutOfRange);
        }
        let thread_id = group_id
            .checked_mul(MAX_SLOTS_PER_GROUP)
            .and_then(|x| x.checked_add(slot))
            .ok_or(RecordSpuError::ThreadIdOverflow)?;
        if self.unit_to_group.contains_key(unit_id) || self.finished_units.contains_key(unit_id) {
            return Err(RecordSpuError::DuplicateUnit);
        }
        if self.thread_id_to_unit.contains_key(thread_id) {
            return Err(RecordSpuError::ThreadIdCollision);
        }
        self.unit_to_group.insert(unit_id, group_id);
        self.thread_id_to_unit.insert(thread_id, unit_id.raw());
        let mut group = self
            .groups
            .get_mut(group_id)
            .expect("group existence checked above and table is &mut");
        group.remaining_unfinished = group
            .remaining_unfinished
            .checked_add(1)
            .expect("remaining_unfinished overflows u32: more than 2^32 SPUs in one group");
        Ok(())
    }

    /// Restore a group whose runtime-side SPU construction failed.
    ///
    /// Returns `false` unless `group_id` names a running group with no
    /// registered units. That restriction keeps this recovery path from
    /// restoring a group after any SPU becomes guest-visible.
    pub(crate) fn cancel_unregistered_start(&mut self, group_id: u32) -> bool {
        let has_units = self.unit_to_group.values().any(|&gid| gid == group_id)
            || self.finished_units.values().any(|&gid| gid == group_id);
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        if group.state != GroupState::Running || group.remaining_unfinished != 0 || has_units {
            return false;
        }
        group.state = GroupState::Created;
        true
    }

    /// Look up the runtime UnitId for a synthetic thread_id.
    pub fn unit_for_thread(&self, thread_id: u32) -> Option<UnitId> {
        self.thread_id_to_unit
            .get(thread_id)
            .map(|&raw| UnitId::new(raw))
    }

    /// The slot an initialized thread occupies, as `(group_id, slot)`.
    fn slot_of_thread(&self, thread_id: u32) -> Option<(u32, u32)> {
        let (group_id, slot) = (
            thread_id / MAX_SLOTS_PER_GROUP,
            thread_id % MAX_SLOTS_PER_GROUP,
        );
        self.groups
            .get(group_id)?
            .slots
            .contains_key(&slot)
            .then_some((group_id, slot))
    }

    /// The signal configuration of an initialized thread.
    pub fn signal_config(&self, thread_id: u32) -> Option<u64> {
        let (group_id, slot) = self.slot_of_thread(thread_id)?;
        self.groups
            .get(group_id)?
            .slots
            .get(&slot)
            .map(|s| s.signal_config)
    }

    /// Set the signal configuration of an initialized thread. `false`
    /// when no initialized thread has the id.
    pub fn set_signal_config(&mut self, thread_id: u32, value: u64) -> bool {
        let Some((group_id, slot)) = self.slot_of_thread(thread_id) else {
            return false;
        };
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        match group.slots.get_mut(&slot) {
            Some(s) => {
                s.signal_config = value;
                true
            }
            None => false,
        }
    }

    /// The `(group_id, slot)` of an initialized thread, or `None` when
    /// no initialized thread has the id.
    pub fn thread_slot(&self, thread_id: u32) -> Option<(u32, u32)> {
        self.slot_of_thread(thread_id)
    }

    /// The queue connected to group event type `event_type`.
    pub fn group_event_queue(&self, group_id: u32, event_type: u32) -> Option<u32> {
        self.groups
            .get(group_id)?
            .event_queues
            .get(&event_type)
            .copied()
    }

    /// Connect group event type `event_type` to `queue`, or disconnect it
    /// with `None`. Returns `false` when no group has the id.
    pub fn set_group_event_queue(
        &mut self,
        group_id: u32,
        event_type: u32,
        queue: Option<u32>,
    ) -> bool {
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        match queue {
            Some(queue) => group.event_queues.insert(event_type, queue),
            None => group.event_queues.remove(&event_type),
        };
        true
    }

    /// The queue connected to event port `port` of an initialized thread.
    pub fn thread_port_queue(&self, thread_id: u32, port: u32) -> Option<u32> {
        let (group_id, slot) = self.slot_of_thread(thread_id)?;
        self.groups
            .get(group_id)?
            .slots
            .get(&slot)?
            .event_ports
            .get(&port)
            .copied()
    }

    /// Connect event port `port` of an initialized thread to `queue`, or
    /// disconnect it with `None`. Returns `false` when no initialized
    /// thread has the id.
    pub fn set_thread_port_queue(&mut self, thread_id: u32, port: u32, queue: Option<u32>) -> bool {
        let Some((group_id, slot)) = self.slot_of_thread(thread_id) else {
            return false;
        };
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        let Some(slot) = group.slots.get_mut(&slot) else {
            return false;
        };
        match queue {
            Some(queue) => slot.event_ports.insert(port, queue),
            None => slot.event_ports.remove(&port),
        };
        true
    }

    /// Connect event port `port` of every initialized thread of the group
    /// to `queue`, or disconnect it with `None`. Returns `false` when no
    /// group has the id.
    pub fn set_group_port_queue(&mut self, group_id: u32, port: u32, queue: Option<u32>) -> bool {
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        for slot in group.slots.values_mut() {
            match queue {
                Some(queue) => slot.event_ports.insert(port, queue),
                None => slot.event_ports.remove(&port),
            };
        }
        true
    }

    /// The queues an initialized thread names by number, by that number.
    pub fn thread_queue_bindings(&self, thread_id: u32) -> Option<&BTreeMap<u32, u32>> {
        let (group_id, slot) = self.slot_of_thread(thread_id)?;
        Some(&self.groups.get(group_id)?.slots.get(&slot)?.queue_bindings)
    }

    /// Bind `queue` to `number` on an initialized thread.
    ///
    /// # Errors
    ///
    /// See [`BindQueueError`]; an error binds nothing.
    pub fn bind_thread_queue(
        &mut self,
        thread_id: u32,
        number: u32,
        queue: u32,
    ) -> Result<(), BindQueueError> {
        let (group_id, slot) = self
            .slot_of_thread(thread_id)
            .ok_or(BindQueueError::UnknownThread)?;
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or(BindQueueError::UnknownThread)?;
        let slot = group
            .slots
            .get_mut(&slot)
            .ok_or(BindQueueError::UnknownThread)?;
        if slot.queue_bindings.contains_key(&number)
            || slot.queue_bindings.values().any(|&bound| bound == queue)
        {
            return Err(BindQueueError::Busy);
        }
        if slot.queue_bindings.len() >= event::QUEUE_BINDING_COUNT {
            return Err(BindQueueError::Full);
        }
        slot.queue_bindings.insert(number, queue);
        Ok(())
    }

    /// Drop the binding an initialized thread holds under `number`.
    ///
    /// # Errors
    ///
    /// See [`UnbindQueueError`].
    pub fn unbind_thread_queue(
        &mut self,
        thread_id: u32,
        number: u32,
    ) -> Result<(), UnbindQueueError> {
        let (group_id, slot) = self
            .slot_of_thread(thread_id)
            .ok_or(UnbindQueueError::UnknownThread)?;
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or(UnbindQueueError::UnknownThread)?;
        let slot = group
            .slots
            .get_mut(&slot)
            .ok_or(UnbindQueueError::UnknownThread)?;
        slot.queue_bindings
            .remove(&number)
            .map(|_| ())
            .ok_or(UnbindQueueError::NotBound)
    }

    /// Drop every group event connection, thread port connection and
    /// queue binding that targets `queue_id`.
    ///
    /// # Cross-module contract
    ///
    /// The queue-destroy arm must call this, or a surviving connection
    /// names a queue the queue table no longer knows.
    ///
    /// O(n) over every slot of every group.
    pub fn unbind_event_queue(&mut self, queue_id: u32) {
        self.groups.for_each_mut(|_, group| {
            group.event_queues.retain(|_, &mut queue| queue != queue_id);
            for slot in group.slots.values_mut() {
                slot.event_ports.retain(|_, &mut queue| queue != queue_id);
                slot.queue_bindings
                    .retain(|_, &mut queue| queue != queue_id);
            }
        });
    }

    /// The signal configuration of the thread a registered SPU runs,
    /// finished or not.
    pub fn signal_config_of_unit(&self, unit_id: UnitId) -> Option<u64> {
        let thread_id = self
            .thread_id_to_unit
            .iter()
            .find(|(_, &raw)| raw == unit_id.raw())
            .map(|(tid, _)| tid)?;
        self.signal_config(thread_id)
    }

    /// The group of a registered SPU that has not finished.
    pub fn live_group_of(&self, unit_id: UnitId) -> Option<u32> {
        self.unit_to_group.get(unit_id).copied()
    }

    /// The group of a registered SPU, finished or not.
    pub fn group_of(&self, unit_id: UnitId) -> Option<u32> {
        self.unit_to_group
            .get(unit_id)
            .or_else(|| self.finished_units.get(unit_id))
            .copied()
    }

    /// Thread-id lookup filtered to [`GroupState::Running`].
    ///
    /// Mailbox writes and other state-sensitive paths must use this
    /// so writes to not-yet-started or already-finished threads
    /// surface as `ESRCH` rather than queuing in a dead mailbox.
    pub fn running_unit_for_thread(&self, thread_id: u32) -> Option<UnitId> {
        let unit_id = self.unit_for_thread(thread_id)?;
        let group_id = self.unit_to_group.get(unit_id)?;
        let group = self.groups.get(*group_id)?;
        if group.state == GroupState::Running {
            Some(unit_id)
        } else {
            None
        }
    }

    /// Notify that the SPU `unit_id` has finished.
    ///
    /// `Ok(Some(group_id))` means this notify drove the group to
    /// `Finished`; `Ok(None)` means the group still has live SPUs.
    ///
    /// # Errors
    /// See [`NotifySpuFinishedError`]. Only `UnknownUnit` is
    /// benign; the others indicate dispatch-layer state
    /// corruption.
    pub fn notify_spu_finished(
        &mut self,
        unit_id: UnitId,
    ) -> Result<Option<u32>, NotifySpuFinishedError> {
        if let Some(&group_id) = self.finished_units.get(unit_id) {
            return Err(NotifySpuFinishedError::AlreadyFinished { group_id });
        }
        let group_id = *self
            .unit_to_group
            .get(unit_id)
            .ok_or(NotifySpuFinishedError::UnknownUnit)?;
        // Existence is guaranteed: `record_spu` checks the group
        // before inserting into `unit_to_group`, and the table
        // exposes no group-destroy path.
        let mut group = self
            .groups
            .get_mut(group_id)
            .expect("unit_to_group references a nonexistent group");
        if group.state != GroupState::Running {
            return Err(NotifySpuFinishedError::GroupNotRunning { state: group.state });
        }
        if group.remaining_unfinished == 0 {
            return Err(NotifySpuFinishedError::CounterUnderflow);
        }
        group.remaining_unfinished -= 1;
        let terminal = group.remaining_unfinished == 0;
        if terminal {
            group.state = GroupState::Finished;
        }
        // Move across the unit_to_group/finished_units boundary so
        // a second notify hits AlreadyFinished, not re-decrement.
        self.unit_to_group.remove(unit_id);
        self.finished_units.insert(unit_id, group_id);
        if terminal {
            Ok(Some(group_id))
        } else {
            Ok(None)
        }
    }

    /// Record that the SPU `unit_id` called `sys_spu_thread_exit` with
    /// `status`, then notify its finish as
    /// [`Self::notify_spu_finished`] does.
    ///
    /// # Errors
    /// See [`NotifySpuFinishedError`]; an error records nothing.
    pub fn thread_exit(
        &mut self,
        unit_id: UnitId,
        status: u32,
    ) -> Result<Option<u32>, NotifySpuFinishedError> {
        let group_id = self.unit_to_group.get(unit_id).copied();
        let finished = self.notify_spu_finished(unit_id)?;
        if let Some(mut group) = group_id.and_then(|gid| self.groups.get_mut(gid)) {
            group.thread_exit_status.insert(unit_id.raw(), status);
        }
        Ok(finished)
    }

    /// End the group of the SPU `unit_id`, which called
    /// `sys_spu_thread_group_exit` with `status`. Every thread of the
    /// group finishes, and a join reads [`group_join_cause::GROUP_EXIT`]
    /// with `status`. Returns the group id and the group's other units
    /// that had not finished.
    ///
    /// # Errors
    /// See [`NotifySpuFinishedError`]; an error changes nothing.
    pub fn group_exit(
        &mut self,
        unit_id: UnitId,
        status: u32,
    ) -> Result<(u32, Vec<UnitId>), NotifySpuFinishedError> {
        if let Some(&group_id) = self.finished_units.get(unit_id) {
            return Err(NotifySpuFinishedError::AlreadyFinished { group_id });
        }
        let group_id = *self
            .unit_to_group
            .get(unit_id)
            .ok_or(NotifySpuFinishedError::UnknownUnit)?;
        // `destroy` scrubs a group's units with it, so a miss here
        // answers as the unit being unknown.
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return Err(NotifySpuFinishedError::UnknownUnit);
        };
        if group.state != GroupState::Running {
            return Err(NotifySpuFinishedError::GroupNotRunning { state: group.state });
        }
        group.state = GroupState::Finished;
        group.remaining_unfinished = 0;
        group.join_cause = group_join_cause::GROUP_EXIT;
        group.exit_status = status;
        let members: Vec<UnitId> = self
            .unit_to_group
            .iter()
            .filter(|(_, &gid)| gid == group_id)
            .map(|(unit, _)| unit)
            .collect();
        for &unit in &members {
            self.unit_to_group.remove(unit);
            self.finished_units.insert(unit, group_id);
        }
        let others = members
            .into_iter()
            .filter(|&unit| unit != unit_id)
            .collect();
        Ok((group_id, others))
    }

    /// End the running group `group_id`, as `sys_spu_thread_group_terminate` does.
    ///
    /// Every thread of the group finishes, and a join reads
    /// [`group_join_cause::TERMINATED`] with `status`. A terminated
    /// thread records no exit status. Returns the units of the group
    /// that were still running.
    ///
    /// # Errors
    /// See [`TerminateGroupError`]; an error changes nothing.
    pub fn terminate(
        &mut self,
        group_id: u32,
        status: u32,
    ) -> Result<Vec<UnitId>, TerminateGroupError> {
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or(TerminateGroupError::Unknown)?;
        if group.state != GroupState::Running {
            return Err(TerminateGroupError::NotRunning { state: group.state });
        }
        group.state = GroupState::Finished;
        group.suspended = false;
        group.remaining_unfinished = 0;
        group.join_cause = group_join_cause::TERMINATED;
        group.exit_status = status;
        let members: Vec<UnitId> = self
            .unit_to_group
            .iter()
            .filter(|(_, &gid)| gid == group_id)
            .map(|(unit, _)| unit)
            .collect();
        for &unit in &members {
            self.unit_to_group.remove(unit);
            self.finished_units.insert(unit, group_id);
        }
        Ok(members)
    }

    /// The units of group `group_id` that have not finished.
    fn live_units(&self, group_id: u32) -> Vec<UnitId> {
        self.unit_to_group
            .iter()
            .filter(|(_, &gid)| gid == group_id)
            .map(|(unit, _)| unit)
            .collect()
    }

    /// Suspend the running group `group_id`, as
    /// `sys_spu_thread_group_suspend` does. A suspended group stays
    /// suspended. Returns the group's units that have not finished, for
    /// the runtime to hold.
    ///
    /// # Errors
    /// See [`RunControlError`]; an error changes nothing.
    pub fn suspend(&mut self, group_id: u32) -> Result<Vec<UnitId>, RunControlError> {
        {
            let mut group = self
                .groups
                .get_mut(group_id)
                .ok_or(RunControlError::Unknown)?;
            if !group.takes_run_control() {
                return Err(RunControlError::NoRunControl);
            }
            if group.state != GroupState::Running {
                return Err(RunControlError::NotRunning);
            }
            group.suspended = true;
        }
        Ok(self.live_units(group_id))
    }

    /// Resume the suspended group `group_id`, as
    /// `sys_spu_thread_group_resume` does. Returns the group's units that
    /// have not finished, for the runtime to release.
    ///
    /// # Errors
    /// See [`RunControlError`]; [`RunControlError::NotRunning`] also
    /// covers a running group that is not suspended. An error changes
    /// nothing.
    pub fn resume(&mut self, group_id: u32) -> Result<Vec<UnitId>, RunControlError> {
        {
            let mut group = self
                .groups
                .get_mut(group_id)
                .ok_or(RunControlError::Unknown)?;
            if !group.takes_run_control() {
                return Err(RunControlError::NoRunControl);
            }
            if group.state != GroupState::Running || !group.suspended {
                return Err(RunControlError::NotRunning);
            }
            group.suspended = false;
        }
        Ok(self.live_units(group_id))
    }

    /// Check a `sys_spu_thread_group_yield` of group `group_id`. A yield
    /// changes nothing: every SPU thread runs as its own unit, so no other
    /// group waits for the SPUs it would give up.
    ///
    /// # Errors
    /// [`RunControlError::Unknown`], and [`RunControlError::NotRunning`]
    /// when the group is not running or is suspended. A group that takes
    /// no run control answers `Ok`.
    pub fn check_yield(&self, group_id: u32) -> Result<(), RunControlError> {
        let group = self.groups.get(group_id).ok_or(RunControlError::Unknown)?;
        if !group.takes_run_control() {
            return Ok(());
        }
        if group.state != GroupState::Running || group.suspended {
            return Err(RunControlError::NotRunning);
        }
        Ok(())
    }

    /// Return the finished group `group_id` to [`GroupState::Created`]
    /// for another start, whether or not a join has read it: its slots
    /// and their arguments stay, each thread's exit status moves to
    /// [`ThreadGroup::previous_exit_status`], and the previous run's
    /// units and join record go. Returns `false`, changing nothing,
    /// unless the group finished.
    pub(crate) fn reset_for_restart(&mut self, group_id: u32) -> bool {
        let units: Vec<(u32, u64)> = self
            .thread_id_to_unit
            .iter()
            .filter(|(tid, _)| tid / MAX_SLOTS_PER_GROUP == group_id)
            .map(|(tid, &raw)| (tid % MAX_SLOTS_PER_GROUP, raw))
            .collect();
        {
            let Some(mut group) = self.groups.get_mut(group_id) else {
                return false;
            };
            if group.state != GroupState::Finished {
                return false;
            }
            for (slot, raw) in units {
                if let Some(&status) = group.thread_exit_status.get(&raw) {
                    group.previous_exit_status.insert(slot, status);
                }
            }
            group.state = GroupState::Created;
            group.remaining_unfinished = 0;
            group.join_cause = group_join_cause::ALL_THREADS_EXIT;
            group.exit_status = 0;
            group.thread_exit_status.clear();
        }
        self.finished_units.retain(|_, gid| *gid != group_id);
        self.thread_id_to_unit
            .retain(|tid, _| tid / MAX_SLOTS_PER_GROUP != group_id);
        true
    }

    /// The status an initialized thread gave `sys_spu_thread_exit`.
    ///
    /// # Errors
    /// See [`ExitStatusError`].
    pub fn thread_exit_status(&self, thread_id: u32) -> Result<u32, ExitStatusError> {
        let (group_id, slot) = self
            .slot_of_thread(thread_id)
            .ok_or(ExitStatusError::UnknownThread)?;
        let group = self
            .groups
            .get(group_id)
            .ok_or(ExitStatusError::UnknownThread)?;
        self.unit_for_thread(thread_id)
            .and_then(|unit| group.thread_exit_status.get(&unit.raw()))
            .or_else(|| group.previous_exit_status.get(&slot))
            .copied()
            .ok_or(ExitStatusError::NoStatus)
    }

    /// Replace the argument block of an initialized thread. The group's
    /// next start passes it; a running SPU keeps the block it started
    /// with. Returns `false` when no initialized thread has the id.
    pub fn set_thread_args(&mut self, thread_id: u32, args: [u64; 4]) -> bool {
        let Some((group_id, slot)) = self.slot_of_thread(thread_id) else {
            return false;
        };
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        match group.slots.get_mut(&slot) {
            Some(s) => {
                s.args = args;
                true
            }
            None => false,
        }
    }

    /// The `cause` and `status` a join of the finished group
    /// `group_id` reads, or `None` for a group that is not finished.
    pub fn join_result(&self, group_id: u32) -> Option<(u32, u32)> {
        let group = self.groups.get(group_id)?;
        (group.state == GroupState::Finished).then_some((group.join_cause, group.exit_status))
    }

    /// The table's partial of the sync-state sum: the groups, the three
    /// unit maps and the id allocator's cursor.
    pub fn sync_partial(&self) -> u128 {
        self.groups
            .partial()
            .wrapping_add(self.unit_to_group.partial())
            .wrapping_add(self.finished_units.partial())
            .wrapping_add(self.thread_id_to_unit.partial())
            .wrapping_add(self.ids_term())
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.groups
            .partial_from_scratch()
            .wrapping_add(self.unit_to_group.partial_from_scratch())
            .wrapping_add(self.finished_units.partial_from_scratch())
            .wrapping_add(self.thread_id_to_unit.partial_from_scratch())
            .wrapping_add(self.ids_term())
    }

    /// The id allocator's term, and the SPU pool split's once Raw SPUs
    /// take a share.
    fn ids_term(&self) -> u128 {
        let ids = lanes::value_term(source::GROUP_IDS, 0, &self.next_id);
        if self.max_raw == 0 {
            ids
        } else {
            ids.wrapping_add(lanes::value_term(source::SPU_LIMITS, 0, &self.max_raw))
        }
    }
}

#[cfg(test)]
#[path = "tests/thread_group_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/thread_group_event_tests.rs"]
mod event_tests;

#[cfg(test)]
#[path = "tests/thread_group_restart_tests.rs"]
mod restart_tests;
