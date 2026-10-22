//! SPU thread groups: create, destroy, start and join.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::spu;
use cellgov_time::GuestTicks;

use crate::dispatch::{Lv2BlockReason, Lv2Dispatch, PendingResponse, SpuInitState};
use crate::host::guest_struct::read_be_u32;
use crate::host::{Lv2Host, Lv2Runtime};
use crate::thread_group::{
    CreateGroupError, DestroyGroupError, GroupKind, GroupRequest, GroupState,
};

/// The arguments of `sys_spu_thread_group_create`.
#[derive(Debug, Clone, Copy)]
pub(in crate::host) struct GroupCreateArgs {
    /// Out: group id.
    pub(in crate::host) id_ptr: u32,
    /// Thread count.
    pub(in crate::host) num_threads: u32,
    /// Group priority.
    pub(in crate::host) priority: i32,
    /// The `sys_spu_thread_group_attribute` record.
    pub(in crate::host) attr_ptr: u32,
}

impl Lv2Host {
    /// `sys_spu_thread_group_create`: mint a monotonic group id and
    /// write it to `id_ptr`, if the SPU pool has room for a group of
    /// `num_threads` threads of the attribute's type.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` when the attribute is unreadable, its container
    ///   word included for a type that takes memory from a container.
    /// - `CELL_EINVAL` when `num_threads` is zero, a group with a
    ///   scheduler context has a priority out of range, or the group can
    ///   never fit the pool.
    /// - `CELL_ESRCH` when the attribute names no memory container that
    ///   exists.
    /// - `CELL_ENOMEM` when the container cannot hold the group's memory.
    /// - `CELL_EBUSY` when the groups that exist leave no room for it.
    /// - `CELL_EAGAIN` when the group allocator is exhausted.
    ///
    /// The reference console answers each of these in this order
    /// (`tests/micro/spu_group_limits`, `tests/micro/spu_pool_edges`,
    /// `tests/micro/spu_group_priority`, `tests/micro/spu_group_container`),
    /// except that no capture places `CELL_ENOMEM` against `CELL_EBUSY`.
    /// A container-type group takes its memory from the container the
    /// attribute names (see [`GroupKind::container_memory`]), and its
    /// destroy returns it. A group of any other type takes nothing from
    /// the process's own budget, which CellGov does not track.
    pub(in crate::host) fn dispatch_group_create(
        &mut self,
        args: GroupCreateArgs,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let GroupCreateArgs {
            id_ptr,
            num_threads,
            priority,
            attr_ptr,
        } = args;
        let type_addr = u64::from(attr_ptr) + u64::from(spu::group_attr::TYPE_OFFSET);
        let Some(group_type) = read_be_u32(rt, type_addr) else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        let container = if group_type & spu::group_type::MEMORY_FROM_CONTAINER != 0 {
            let ct_addr = u64::from(attr_ptr) + u64::from(spu::group_attr::CONTAINER_OFFSET);
            let Some(ct) = read_be_u32(rt, ct_addr) else {
                return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
            };
            Some(ct)
        } else {
            None
        };
        // A zero-thread group is refused before anything is
        // allocated: a slotless group could never reach the
        // fully-initialized state `sys_spu_thread_group_start`
        // requires.
        if num_threads == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let root = self.privilege_of(requester).root();
        let pid = self.state.processes.process_of_unit(requester);
        let request = GroupRequest {
            num_threads,
            group_type,
            priority,
        };
        if self.state.groups.check_valid(request, root, pid).is_err() {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if container.is_some_and(|ct| !self.state.memory_containers.contains_key(ct)) {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        let charge = container.map(|ct| {
            let bytes = GroupKind::of_type(group_type).container_memory(num_threads);
            (ct, bytes)
        });
        if let Some((ct, bytes)) = charge {
            if !self
                .state
                .memory_containers
                .get(ct)
                .is_some_and(|c| c.has_room(bytes))
            {
                return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
            }
        }
        // The pool holds at most `pool::USABLE_SPUS` threads per group,
        // well under the `MAX_SLOTS_PER_GROUP` thread-id encoding.
        let group_id = match self.state.groups.create_typed(request, root, pid) {
            Ok(id) => id,
            Err(CreateGroupError::Invalid) => {
                return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
            }
            Err(CreateGroupError::Busy) => {
                return Lv2Dispatch::immediate(errno::CELL_EBUSY.into());
            }
            Err(CreateGroupError::Exhausted) => {
                return Lv2Dispatch::immediate(errno::CELL_EAGAIN.into());
            }
        };
        if let Some((ct, bytes)) = charge {
            if let Some(mut c) = self.state.memory_containers.get_mut(ct) {
                c.taken += bytes;
            }
            if let Some(mut group) = self.state.groups.get_mut(group_id) {
                group.container = Some(ct);
            }
        }

        let range = ByteRange::contiguous_u32(id_ptr, 4);
        let effect = Effect::shared_write(
            range,
            WritePayload::from_slice(&group_id.to_be_bytes()),
            requester,
            tick,
        );

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![effect],
        }
    }

    /// `sys_spu_thread_group_destroy`: withdraw a group whose state is
    /// not [`GroupState::Running`]. Unknown id -> CELL_ESRCH; running
    /// group -> CELL_EBUSY (the title must terminate or join first).
    /// Destroy clears the group's unit and thread maps and withdraws
    /// the user images its slots registered.
    pub(in crate::host) fn dispatch_group_destroy(&mut self, group_id: u32) -> Lv2Dispatch {
        // A user image lives as long as the group whose slot
        // registered it, so its segments go when the group does.
        // Kernel handles are not in the user map and pass through
        // untouched.
        let slot_handles: Vec<_> = self
            .state
            .groups
            .get(group_id)
            .map(|g| g.slots.values().map(|s| s.image_handle).collect())
            .unwrap_or_default();
        let charge = self.state.groups.get(group_id).and_then(|g| {
            g.container
                .map(|ct| (ct, g.kind().container_memory(g.num_threads)))
        });
        let code = match self.state.groups.destroy(group_id) {
            Ok(()) => {
                for handle in slot_handles {
                    self.content_store_mut().withdraw_user_image(handle);
                }
                // The memory goes back to the container the group took
                // it from.
                if let Some((ct, bytes)) = charge {
                    if let Some(mut c) = self.state.memory_containers.get_mut(ct) {
                        debug_assert!(
                            c.taken >= bytes,
                            "container {ct:#x} gets back more than it gave"
                        );
                        c.taken = c.taken.saturating_sub(bytes);
                    }
                }
                0
            }
            Err(DestroyGroupError::Unknown) => errno::CELL_ESRCH.into(),
            Err(DestroyGroupError::Busy) => errno::CELL_EBUSY.into(),
        };
        Lv2Dispatch::Immediate {
            code,
            effects: vec![],
        }
    }

    /// `sys_spu_thread_group_start`: register every initialized slot's
    /// SPU and move the group to [`GroupState::Running`]. Unknown id ->
    /// CELL_ESRCH; a running group -> CELL_ESTAT; a finished group
    /// starts again.
    /// The runtime creates the SPUs from the returned init states.
    /// Each slot enters at the entry [`Self::load_image_for`] reports.
    pub(in crate::host) fn dispatch_group_start(&mut self, group_id: u32) -> Lv2Dispatch {
        let group = match self.state.groups.get(group_id) {
            Some(g) => g,
            None => {
                return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
            }
        };

        // Group start leaves the initialized state and answers
        // CELL_ESTAT while the group runs. A finished group is
        // initialized again, whether or not a join has read it: its
        // previous run's units are dropped and it starts afresh
        // (tests/micro/spu_group_suspend).
        let restart = group.state == GroupState::Finished;
        if group.state != GroupState::Created && !restart {
            return Lv2Dispatch::immediate(errno::CELL_ESTAT.into());
        }

        // Two-pass: validate every handle, then build `inits`. The second
        // pass's `expect` requires the lookups to be pure reads. A
        // refusal comes before the restart reset, so it changes nothing.
        let slot_entries: Vec<_> = group.slots.iter().map(|(&k, v)| (k, v.clone())).collect();
        for (_slot_idx, slot) in &slot_entries {
            if self.load_image_for(slot.image_handle).is_none() {
                return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
            }
        }
        if restart {
            self.state.groups.reset_for_restart(group_id);
        }

        let mut inits = std::collections::BTreeMap::new();
        for (slot_idx, slot) in &slot_entries {
            let (image, entry_pc) = self
                .load_image_for(slot.image_handle)
                .expect("handle validated above");
            inits.insert(
                *slot_idx,
                SpuInitState {
                    image,
                    entry_pc,
                    stack_ptr: 0x3FFF0,
                    args: slot.args,
                    group_id,
                },
            );
        }

        let mut group = self
            .state
            .groups
            .get_mut(group_id)
            .expect("group existed above");
        group.state = GroupState::Running;
        // Each run starts from the configuration the kernel keeps
        // (tests/micro/spu_signal_mbox_edges).
        for slot in group.slots.values_mut() {
            slot.live_signal_config = None;
        }
        drop(group);

        Lv2Dispatch::RegisterSpu {
            inits,
            effects: vec![],
            code: 0,
        }
    }

    /// `sys_spu_thread_group_join`: park the caller on a running group
    /// until every SPU in it finishes; a finished group answers at
    /// once. The cause and status are the ones the group finished
    /// with: `ALL_THREADS_EXIT` and 0 when every thread exited, or
    /// `GROUP_EXIT` and the status a `sys_spu_thread_group_exit` gave.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when `group_id` is unknown.
    /// - `CELL_EINVAL` when the group is not yet started.
    /// - `CELL_EFAULT` when `cause_ptr` is null; the arm writes
    ///   nothing.
    /// - `CELL_EFAULT` when `status_ptr` alone is null; the arm still
    ///   writes the cause.
    ///
    /// Both null screens run after the wait. The `spu_group_join_null`
    /// capture shows the three null cases on a running group.
    pub(in crate::host) fn dispatch_group_join(
        &self,
        group_id: u32,
        cause_ptr: u32,
        status_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let group = match self.state.groups.get(group_id) {
            Some(g) => g,
            None => {
                return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
            }
        };

        match group.state {
            GroupState::Created => Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
            GroupState::Running => Lv2Dispatch::Block {
                reason: Lv2BlockReason::ThreadGroupJoin { group_id },
                pending: PendingResponse::ThreadGroupJoin {
                    group_id,
                    code: 0,
                    cause_ptr,
                    status_ptr,
                    // The wake writes the cause and status the group
                    // finishes with; these hold until then.
                    cause: group.join_cause,
                    status: group.exit_status,
                },
                effects: vec![],
            },
            GroupState::Finished => {
                // NULL out-pointer contract, screened after the wait
                // so it applies even when the group already finished:
                // a NULL cause writes nothing -- not even a
                // non-NULL status -- and returns CELL_EFAULT; a NULL
                // status alone still writes cause and returns
                // CELL_EFAULT. Mirrors resolve_join_wakes in
                // cellgov_core so the immediate and parked paths agree.
                if cause_ptr == 0 {
                    return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
                }
                let mut effects = vec![Effect::shared_write(
                    ByteRange::contiguous_u32(cause_ptr, 4),
                    WritePayload::from_slice(&group.join_cause.to_be_bytes()),
                    requester,
                    tick,
                )];
                if status_ptr == 0 {
                    return Lv2Dispatch::Immediate {
                        code: errno::CELL_EFAULT.into(),
                        effects,
                    };
                }
                effects.push(Effect::shared_write(
                    ByteRange::contiguous_u32(status_ptr, 4),
                    WritePayload::from_slice(&group.exit_status.to_be_bytes()),
                    requester,
                    tick,
                ));
                Lv2Dispatch::Immediate { code: 0, effects }
            }
        }
    }
}
