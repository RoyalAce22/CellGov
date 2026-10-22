//! The group and slot types, the SPU pool demand of a group, and the
//! group's hash lanes.

use crate::dispatch::{SpuInitState, SpuLoadImage};
use crate::image::SpuImageHandle;
use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;
#[cfg(doc)]
use cellgov_ps3_abi::lv2::spu::event;
use cellgov_ps3_abi::lv2::spu::{group_priority, group_type, pool, GROUP_MEMORY_PER_THREAD};
use std::collections::BTreeMap;

/// Cap on slots per group; the `group_id * 256 + slot` thread-id
/// encoding aliases adjacent groups' ranges above this.
///
/// `sys_spu_thread_group_create` holds a group to the SPU pool, at most
/// [`pool::USABLE_SPUS`] threads, so no group a create admits reaches
/// this cap; it bounds the slot indices `sys_spu_thread_initialize`
/// names. The `spu_group_init_limits` capture shows the console's slot
/// ceiling at the same 256.
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
    ///
    /// This is the configuration the kernel keeps for the thread. A
    /// `sys_spu_thread_set_spu_cfg` while the group runs changes
    /// [`Self::live_signal_config`] instead: the
    /// `spu_signal_mbox_edges` capture reads a configuration set during
    /// a run back during that run, and the earlier one after it.
    pub signal_config: u64,
    /// The configuration the running SPU uses when a
    /// `sys_spu_thread_set_spu_cfg` during the run replaced
    /// [`Self::signal_config`]; `None` until then. Each start clears it.
    pub live_signal_config: Option<u64>,
    /// The value the kernel keeps for each signal-notification register,
    /// `None` until a `sys_spu_thread_write_snr` reaches the thread while
    /// its group does not run. Each start loads the values present into
    /// the new SPU. The SPU reading a register and a write during a run
    /// leave them as they are, as the `spu_snr_outside_run` and
    /// `spu_snr_run_save` captures show.
    pub saved_signals: [Option<u32>; 2],
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
    /// The memory container the group's memory came from, for a type
    /// that takes it from one.
    pub container: Option<u32>,
    /// The process that created the group. The SPU pool split and the
    /// pool accounting are that process's own.
    pub pid: u32,
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
pub(super) fn priority_in_range(priority: i32, root: bool) -> bool {
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
    /// The memory a group of this kind with `num_threads` threads takes
    /// from a memory container: a local-store save area per thread, and
    /// one in all for a group that shares an SPU with the system. No
    /// container type lacks a scheduler context, so a group without one
    /// takes none.
    // The console captures of tests/micro/spu_group_container and
    // tests/micro/spu_group_container_coop fix the first two amounts.
    pub fn container_memory(self, num_threads: u32) -> u64 {
        match self {
            Self::Context => u64::from(num_threads) * GROUP_MEMORY_PER_THREAD,
            Self::CooperateWithSystem => GROUP_MEMORY_PER_THREAD,
            Self::NonContext => 0,
        }
    }

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
pub(super) struct PoolDemand {
    /// SPUs held for the whole life of a group.
    pub(super) physical: u32,
    /// SPUs the context-switched groups need at once.
    pub(super) controllable: u32,
    /// Groups that share an SPU with the system.
    cooperating: u32,
}

impl PoolDemand {
    /// The demand of one group of `num` threads.
    pub(super) fn of(kind: GroupKind, num: u32) -> Self {
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
    pub(super) fn with(self, other: Self) -> Self {
        Self {
            physical: self.physical.saturating_add(other.physical),
            controllable: self.controllable.max(other.controllable),
            cooperating: self.cooperating.saturating_add(other.cooperating),
        }
    }

    /// Whether the pool holds this demand with `spu_limit` SPUs for SPU
    /// threads. The physical bound is strict: a demand that takes every
    /// SPU is refused.
    pub(super) fn fits(self, spu_limit: u32) -> bool {
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
    pub(super) fn takes_run_control(&self) -> bool {
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
        if let Some(container) = self.container {
            lanes.lane(34, 0, u64::from(container));
        }
        // A group without the lane belongs to the boot process.
        if self.pid != BOOT_PROCESS_PID {
            lanes.lane(35, 0, u64::from(self.pid));
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
            if let Some(live) = slot.live_signal_config {
                lanes.lane(36, index, live);
            }
            for (register, value) in slot.saved_signals.iter().enumerate() {
                if let Some(value) = value {
                    lanes.lane(37, (index << 1) | register as u64, u64::from(*value));
                }
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
