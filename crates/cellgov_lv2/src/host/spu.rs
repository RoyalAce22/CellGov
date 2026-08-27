//! SPU-lifecycle LV2 dispatch: image open, thread-group
//! create/start/initialize/join, and mailbox write.

use cellgov_effects::{Effect, MailboxMessage, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_sync::MailboxId;

use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_spu;

use crate::dispatch::{Lv2BlockReason, Lv2Dispatch, PendingResponse, SpuInitState, SpuLoadImage};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::image::LsSegment;
use crate::request::Lv2Request;
use crate::thread_group::{DestroyGroupError, GroupState, MAX_SLOTS_PER_GROUP};
use cellgov_time::GuestTicks;

/// The `sys_spu_image` record the kernel hands back: `type` KERNEL with
/// the image id in `entry_point` (RPCS3 `sys_spu.cpp`
/// `sys_spu_thread_initialize`, `SYS_SPU_IMAGE_TYPE_KERNEL` arm).
fn kernel_image_struct(handle: crate::image::SpuImageHandle) -> [u8; 16] {
    let mut img_struct = [0u8; 16];
    img_struct[0..4].copy_from_slice(&sys_spu::image::TYPE_KERNEL.to_be_bytes());
    img_struct[4..8].copy_from_slice(&handle.raw().to_be_bytes());
    img_struct
}

/// Why a user-type `sys_spu_image` was refused at thread initialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserImageRefusal {
    /// A record or segment table word could not be read.
    Unreadable,
    /// The record or a segment breaks a documented bound; the payload
    /// is the diagnostic site naming which one.
    Invalid(&'static str),
}

impl Lv2Host {
    /// Parse a user-type `sys_spu_image` into its entry and local-store segments.
    ///
    /// The gates mirror RPCS3's `sys_spu.cpp` `sys_spu_thread_initialize`
    /// (`SYS_SPU_IMAGE_TYPE_USER` arm). Segment bytes are snapshotted
    /// here rather than at group start, which has no guest-memory access.
    fn parse_user_image(
        &self,
        img_ptr: u32,
        rt: &dyn Lv2Runtime,
    ) -> Result<(u32, Vec<LsSegment>), UserImageRefusal> {
        use sys_spu::{image, segment, LS_SIZE};
        // Field addresses are formed in u64: a record or table row at
        // the top of the 32-bit space reads as unmapped (CELL_EFAULT)
        // instead of wrapping the guest pointer.
        let word = |addr: u64| -> Result<u32, UserImageRefusal> {
            let bytes = rt
                .read_committed(addr, 4)
                .ok_or(UserImageRefusal::Unreadable)?;
            Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        };
        let record = u64::from(img_ptr);
        let entry = word(record + u64::from(image::ENTRY_OFFSET))?;
        let segs_ptr = word(record + u64::from(image::SEGS_OFFSET))?;
        let nsegs = word(record + u64::from(image::NSEGS_OFFSET))? as i32;
        if entry > image::ENTRY_MAX || nsegs <= 0 || nsegs > image::NSEGS_MAX {
            return Err(UserImageRefusal::Invalid(
                "dispatch.spu_user_image_record_bounds",
            ));
        }
        let mut segments: Vec<LsSegment> = Vec::new();
        let mut placed: Vec<(u32, u32)> = Vec::new();
        let mut found_info = false;
        let mut found_copy = false;
        for i in 0..nsegs as u32 {
            let base = segs_ptr
                .checked_add(i * segment::LEN)
                .ok_or(UserImageRefusal::Invalid(
                    "dispatch.spu_user_image_table_wraps",
                ))?;
            let row = u64::from(base);
            let seg_type = word(row + u64::from(segment::TYPE_OFFSET))?;
            let ls = word(row + u64::from(segment::LS_OFFSET))?;
            let size = word(row + u64::from(segment::SIZE_OFFSET))?;
            let addr = word(row + u64::from(segment::ADDR_OFFSET))?;
            match seg_type {
                segment::TYPE_INFO => {
                    if size > segment::INFO_SIZE_MAX || found_info {
                        return Err(UserImageRefusal::Invalid(
                            "dispatch.spu_user_image_info_segment",
                        ));
                    }
                    found_info = true;
                    continue;
                }
                segment::TYPE_COPY => {
                    if !addr.is_multiple_of(4) {
                        return Err(UserImageRefusal::Invalid(
                            "dispatch.spu_user_image_copy_source_unaligned",
                        ));
                    }
                    found_copy = true;
                }
                segment::TYPE_FILL => {}
                _ => {
                    return Err(UserImageRefusal::Invalid(
                        "dispatch.spu_user_image_segment_type",
                    ))
                }
            }
            // The end check is this model's own: the oracle stops at
            // `ls < LS_SIZE && size <= LS_SIZE` and would copy past
            // local store, while the loader here refuses the segment
            // at group start, where nothing can answer the guest.
            if size == 0
                || !(ls | size).is_multiple_of(segment::LOAD_ALIGN)
                || ls >= LS_SIZE
                || size > LS_SIZE
                || ls + size > LS_SIZE
            {
                return Err(UserImageRefusal::Invalid(
                    "dispatch.spu_user_image_segment_bounds",
                ));
            }
            if placed
                .iter()
                .any(|&(p_ls, p_size)| ls + size > p_ls && p_ls + p_size > ls)
            {
                return Err(UserImageRefusal::Invalid(
                    "dispatch.spu_user_image_segment_overlap",
                ));
            }
            placed.push((ls, size));
            let bytes = if seg_type == segment::TYPE_COPY {
                rt.read_committed(u64::from(addr), size as usize)
                    .ok_or(UserImageRefusal::Unreadable)?
                    .to_vec()
            } else {
                addr.to_be_bytes()
                    .iter()
                    .copied()
                    .cycle()
                    .take(size as usize)
                    .collect()
            };
            segments.push(LsSegment {
                ls_start: ls,
                bytes,
            });
        }
        if !found_copy {
            return Err(UserImageRefusal::Invalid(
                "dispatch.spu_user_image_no_copy_segment",
            ));
        }
        Ok((entry, segments))
    }

    /// `sys_spu_image_import`: register `size` bytes at `img_ptr` in
    /// [`crate::image::ContentStore`] under a synthetic path and write the
    /// handle into the SPU image struct at `handle_out`.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` when `img_ptr` / `size` are out of guest bounds.
    /// - `CELL_EFAULT` when `handle_out` is not writable for 16 bytes.
    #[allow(
        clippy::too_many_arguments,
        reason = "request payload plus the dispatch tick"
    )]
    pub(super) fn dispatch_image_import(
        &mut self,
        handle_out: u32,
        img_ptr: u32,
        size: u64,
        type_id: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        // > usize image cannot satisfy a read; reject as CELL_EINVAL
        // alongside the out-of-bounds branch.
        let Ok(size) = usize::try_from(size) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        };
        let img_bytes = match rt.read_committed(u64::from(img_ptr), size) {
            Some(b) => b,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
            }
        };
        if !rt.writable(u64::from(handle_out), 16) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        // Synthetic path keys every (type_id, img_ptr) pair to a distinct
        // entry; ELF parsing is deferred to sys_spu_thread_initialize.
        let path = format!("/:import:{type_id:#x}:{img_ptr:#x}");
        let handle = self
            .content_store_mut()
            .register(path.as_bytes(), img_bytes.to_vec());

        let img_struct = kernel_image_struct(handle);
        let range = ByteRange::contiguous_u32(handle_out, 16);
        let effect = Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::from_slice(&img_struct),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![effect],
        }
    }

    pub(super) fn dispatch_image_open(
        &mut self,
        img_ptr: u32,
        path_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let path_bytes = match rt.read_committed(path_ptr as u64, sys_spu::IMAGE_PATH_MAX) {
            Some(bytes) => bytes,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
            }
        };
        // Missing NUL is malformed (EINVAL), distinct from not-found (ENOENT).
        let path_len = match path_bytes.iter().position(|&b| b == 0) {
            Some(n) => n,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
            }
        };
        let path = &path_bytes[..path_len];

        let record = match self.state.content.lookup_by_path(path) {
            Some(r) => r,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_ENOENT.into());
            }
        };

        let img_struct = kernel_image_struct(record.handle);

        let range = ByteRange::contiguous_u32(img_ptr, 16);
        let effect = Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::from_slice(&img_struct),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![effect],
        }
    }

    pub(super) fn dispatch_group_create(
        &mut self,
        id_ptr: u32,
        num_threads: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        // A zero-thread group is refused before anything is allocated:
        // RPCS3 `sys_spu.cpp` `sys_spu_thread_group_create` rejects
        // `!num` with CELL_EINVAL, and a slotless group could never
        // reach the fully-initialized state that
        // `sys_spu_thread_group_start` requires.
        if num_threads == 0 || num_threads > MAX_SLOTS_PER_GROUP {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let group_id = match self.state.groups.create(num_threads) {
            Some(id) => id,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EAGAIN.into());
            }
        };

        let range = ByteRange::contiguous_u32(id_ptr, 4);
        let effect = Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::from_slice(&group_id.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![effect],
        }
    }

    /// `sys_spu_thread_group_destroy`: withdraw a group whose state is
    /// not [`GroupState::Running`]. Unknown id -> CELL_ESRCH; running
    /// group -> CELL_EBUSY (the title must terminate or join first).
    pub(super) fn dispatch_group_destroy(&mut self, group_id: u32) -> Lv2Dispatch {
        // A user image lives as long as the slot that registered it:
        // RPCS3 `sys_spu.h` keeps `lv2_spu_group::imgs` inside the
        // group, so its segments go when the group does. Kernel
        // handles are not in the user map and pass through untouched.
        let slot_handles: Vec<_> = self
            .state
            .groups
            .get(group_id)
            .map(|g| g.slots.values().map(|s| s.image_handle).collect())
            .unwrap_or_default();
        let code = match self.state.groups.destroy(group_id) {
            Ok(()) => {
                for handle in slot_handles {
                    self.content_store_mut().withdraw_user_image(handle);
                }
                0
            }
            Err(DestroyGroupError::Unknown) => cell_errors::CELL_ESRCH.into(),
            Err(DestroyGroupError::Busy) => cell_errors::CELL_EBUSY.into(),
        };
        Lv2Dispatch::Immediate {
            code,
            effects: vec![],
        }
    }

    /// Resolve an image handle to what group start loads and where it
    /// enters. A kernel image reports 0x80 and the SPU factories pin
    /// `pc` to it after the ELF loads, so the ELF's own `e_entry` is
    /// not consulted (RPCS3 `sys_spu_thread_group_start` enters at
    /// `e_entry`); a user image enters where its record says.
    fn load_image_for(&self, handle: crate::image::SpuImageHandle) -> Option<(SpuLoadImage, u32)> {
        if let Some(record) = self.state.content.lookup_by_handle(handle) {
            return Some((SpuLoadImage::Elf(record.elf_bytes.clone()), 0x80));
        }
        let user = self.state.content.lookup_user_image(handle)?;
        Some((SpuLoadImage::Segments(user.segments.clone()), user.entry))
    }

    /// `sys_spu_thread_group_start`: register every initialized slot's
    /// SPU and move the group to [`GroupState::Running`]. Unknown id ->
    /// CELL_ESRCH; a group that has already been started -> CELL_ESTAT.
    pub(super) fn dispatch_group_start(&mut self, group_id: u32) -> Lv2Dispatch {
        let group = match self.state.groups.get_mut(group_id) {
            Some(g) => g,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
            }
        };

        // RPCS3 `sys_spu.cpp` `sys_spu_thread_group_start` compare-and-
        // swaps the group out of its initialized state and answers
        // CELL_ESTAT for every other state, so a restart of a running or
        // finished group is refused instead of silently re-registering
        // its SPUs against a second RegisterSpu dispatch.
        if group.state != GroupState::Created {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESTAT.into());
        }

        // Two-pass: validate every handle, then build `inits`. The second
        // pass's `expect` requires the lookups to be pure reads.
        let slot_entries: Vec<_> = group.slots.iter().map(|(&k, v)| (k, v.clone())).collect();
        for (_slot_idx, slot) in &slot_entries {
            if self.load_image_for(slot.image_handle).is_none() {
                return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
            }
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

        self.state
            .groups
            .get_mut(group_id)
            .expect("group existed above")
            .state = GroupState::Running;

        Lv2Dispatch::RegisterSpu {
            inits,
            effects: vec![],
            code: 0,
        }
    }

    pub(super) fn dispatch_thread_initialize(
        &mut self,
        req: Lv2Request,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        self.obs.spu_thread_initialize_dispatches =
            self.obs.spu_thread_initialize_dispatches.wrapping_add(1);
        let (thread_ptr, group_id, thread_num, img_ptr, arg_ptr) = match req {
            Lv2Request::SpuThreadInitialize {
                thread_ptr,
                group_id,
                thread_num,
                img_ptr,
                arg_ptr,
                ..
            } => (thread_ptr, group_id, thread_num, img_ptr, arg_ptr),
            other => {
                debug_assert!(
                    false,
                    "dispatch_thread_initialize got wrong request variant: {other:?}"
                );
                return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
            }
        };

        // Slot index is screened ahead of every pointer read: RPCS3
        // `sys_spu.cpp` `sys_spu_thread_initialize` rejects an out-of-
        // range `spu_num` as its first act, so an out-of-range slot with
        // an unreadable image pointer answers CELL_EINVAL, not
        // CELL_EFAULT.
        if thread_num >= MAX_SLOTS_PER_GROUP {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }

        // A kernel record carries the image id in `entry_point`.
        let image_word = |offset: u32| -> Option<u32> {
            let bytes = rt.read_committed(u64::from(img_ptr) + u64::from(offset), 4)?;
            Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        };
        let Some(image_type) = image_word(sys_spu::image::TYPE_OFFSET) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        let (kernel_handle, user_image) = match image_type {
            sys_spu::image::TYPE_KERNEL => {
                let Some(handle) = image_word(sys_spu::image::ENTRY_OFFSET) else {
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                };
                (handle, None)
            }
            sys_spu::image::TYPE_USER => match self.parse_user_image(img_ptr, rt) {
                Ok(parsed) => (0, Some(parsed)),
                Err(UserImageRefusal::Unreadable) => {
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                }
                Err(UserImageRefusal::Invalid(site)) => {
                    self.log_invariant_break(
                        site,
                        format_args!(
                            "sys_spu_thread_initialize refused the user image record at \
                             0x{img_ptr:08x}"
                        ),
                    );
                    return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
                }
            },
            _ => return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
        };

        // Args snapshot at initialize time, not at group_start: the PPU
        // may reuse the same stack variable across calls. `arg_ptr == 0`
        // opts out; a non-zero pointer that fails to read is EFAULT.
        let args = if arg_ptr == 0 {
            [0u64; 4]
        } else {
            match rt.read_committed(arg_ptr as u64, 32) {
                Some(bytes) if bytes.len() >= 32 => {
                    let mut a = [0u64; 4];
                    for (i, chunk) in bytes.as_chunks::<8>().0.iter().enumerate().take(4) {
                        a[i] = u64::from_be_bytes(*chunk);
                    }
                    a
                }
                _ => {
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                }
            }
        };

        let thread_id = match group_id
            .checked_mul(MAX_SLOTS_PER_GROUP)
            .and_then(|base| base.checked_add(thread_num))
        {
            Some(id) => id,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
            }
        };

        let (handle, registered_user) = match user_image {
            Some((entry, segments)) => {
                let handle = self
                    .content_store_mut()
                    .register_user_image(entry, segments);
                (handle, Some(handle))
            }
            None => {
                // ContentStore never allocates handle 0; guest-supplied 0 -> ESRCH.
                let Some(handle) = crate::image::SpuImageHandle::new(kernel_handle) else {
                    return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
                };
                // The id must name an image the kernel holds: RPCS3
                // `sys_spu.cpp` `sys_spu_thread_initialize` answers
                // CELL_ESRCH when the KERNEL arm's id lookup fails.
                // User-image handles are not kernel ids, so a forged
                // kernel record cannot alias one at group start.
                if self.state.content.lookup_by_handle(handle).is_none() {
                    return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
                }
                (handle, None)
            }
        };
        let refusal = match self
            .state
            .groups
            .initialize_thread(group_id, thread_num, handle, args)
        {
            Ok(()) => None,
            Err(crate::thread_group::InitializeThreadError::UnknownGroup) => {
                Some(cell_errors::CELL_ESRCH)
            }
            Err(crate::thread_group::InitializeThreadError::SlotAlreadyInitialized) => {
                Some(cell_errors::CELL_EBUSY)
            }
            // RPCS3 `sys_spu.cpp` `sys_spu_thread_initialize` answers
            // CELL_EBUSY once the group has left its not-initialized
            // state, the same code it uses for an occupied slot.
            Err(crate::thread_group::InitializeThreadError::GroupAlreadyStarted { .. }) => {
                Some(cell_errors::CELL_EBUSY)
            }
            // A fully populated group has left its not-initialized
            // state too, so it takes the same arm.
            Err(crate::thread_group::InitializeThreadError::GroupFull { .. }) => {
                Some(cell_errors::CELL_EBUSY)
            }
            // A slot index past the thread map is the bad-argument arm
            // RPCS3 `sys_spu.cpp` `sys_spu_thread_initialize` answers
            // CELL_EINVAL for.
            Err(crate::thread_group::InitializeThreadError::SlotOutOfRange) => {
                Some(cell_errors::CELL_EINVAL)
            }
        };
        if let Some(code) = refusal {
            if let Some(user) = registered_user {
                self.content_store_mut().withdraw_user_image(user);
            }
            return Lv2Dispatch::immediate(code.into());
        }

        let range = ByteRange::contiguous_u32(thread_ptr, 4);
        let effect = Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::from_slice(&thread_id.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![effect],
        }
    }

    pub(super) fn dispatch_group_join(
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
                return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
            }
        };

        // TODO(spu): source cause/status from the group's recorded
        // termination reason once abnormal causes are tracked, instead
        // of hard-coding GROUP_EXIT / status 0 for both branches below.
        match group.state {
            GroupState::Created => Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
            GroupState::Running => Lv2Dispatch::Block {
                reason: Lv2BlockReason::ThreadGroupJoin { group_id },
                pending: PendingResponse::ThreadGroupJoin {
                    group_id,
                    code: 0,
                    cause_ptr,
                    status_ptr,
                    cause: sys_spu::group_join_cause::GROUP_EXIT,
                    status: 0,
                },
                effects: vec![],
            },
            GroupState::Finished => {
                // NULL out-pointer contract (RPCS3 sys_spu.cpp
                // sys_spu_thread_group_join checks the pointers after
                // the wait, so it applies even when the group already
                // finished): a NULL cause writes nothing -- not even a
                // non-NULL status -- and returns CELL_EFAULT; a NULL
                // status alone still writes cause and returns
                // CELL_EFAULT. Mirrors resolve_join_wakes in
                // cellgov_core so the immediate and parked paths agree.
                if cause_ptr == 0 {
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                }
                let mut effects = vec![Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(cause_ptr, 4),
                    bytes: WritePayload::from_slice(
                        &sys_spu::group_join_cause::GROUP_EXIT.to_be_bytes(),
                    ),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: tick,
                }];
                if status_ptr == 0 {
                    return Lv2Dispatch::Immediate {
                        code: cell_errors::CELL_EFAULT.into(),
                        effects,
                    };
                }
                effects.push(Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(status_ptr, 4),
                    bytes: WritePayload::from_slice(&0u32.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: tick,
                });
                Lv2Dispatch::Immediate { code: 0, effects }
            }
        }
    }

    pub(super) fn dispatch_write_mb(
        &self,
        thread_id: u32,
        value: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let target_uid = match self.state.groups.running_unit_for_thread(thread_id) {
            Some(uid) => uid,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
            }
        };
        let effect = Effect::MailboxSend {
            mailbox: MailboxId::new(target_uid.raw()),
            message: MailboxMessage::new(value),
            source: requester,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![effect],
        }
    }
}

#[cfg(test)]
#[path = "tests/spu_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/spu_user_image_tests.rs"]
mod user_image_tests;
