//! SPU-lifecycle LV2 dispatch: image open, thread-group
//! create/start/initialize/join, and mailbox write.

use cellgov_effects::{Effect, MailboxMessage, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_sync::MailboxId;

use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_spu;

use crate::dispatch::{Lv2BlockReason, Lv2Dispatch, PendingResponse, SpuInitState};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::request::Lv2Request;
use crate::thread_group::{DestroyGroupError, GroupState, MAX_SLOTS_PER_GROUP};

impl Lv2Host {
    /// `sys_spu_image_import`: register `size` bytes at `img_ptr` in
    /// [`crate::image::ContentStore`] under a synthetic path and write the
    /// handle into the SPU image struct at `handle_out`.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` when `img_ptr` / `size` are out of guest bounds.
    /// - `CELL_EFAULT` when `handle_out` is not writable for 16 bytes.
    pub(super) fn dispatch_image_import(
        &mut self,
        handle_out: u32,
        img_ptr: u32,
        size: u64,
        type_id: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
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

        let mut img_struct = [0u8; 16];
        img_struct[0..4].copy_from_slice(&handle.raw().to_be_bytes());
        let range = ByteRange::contiguous_u32(handle_out, 16);
        let effect = Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::from_slice(&img_struct),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
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

        let record = match self.content.lookup_by_path(path) {
            Some(r) => r,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_ENOENT.into());
            }
        };

        // sys_spu_image_t (16 bytes, big-endian):
        //   [0..4]   type/handle (u32)
        //   [4..8]   entry point (u32) -- resolved at thread init
        //   [8..12]  segments addr (u32) -- opaque, unused
        //   [12..16] nsegs (i32)
        let handle = record.handle;
        let mut img_struct = [0u8; 16];
        img_struct[0..4].copy_from_slice(&handle.raw().to_be_bytes());

        let range = ByteRange::contiguous_u32(img_ptr, 16);
        let effect = Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::from_slice(&img_struct),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
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
    ) -> Lv2Dispatch {
        if num_threads > MAX_SLOTS_PER_GROUP {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let group_id = match self.groups.create(num_threads) {
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
            source_time: self.current_tick,
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
        let code = match self.groups.destroy(group_id) {
            Ok(()) => 0,
            Err(DestroyGroupError::Unknown) => cell_errors::CELL_ESRCH.into(),
            Err(DestroyGroupError::Busy) => cell_errors::CELL_EBUSY.into(),
        };
        Lv2Dispatch::Immediate {
            code,
            effects: vec![],
        }
    }

    pub(super) fn dispatch_group_start(&mut self, group_id: u32) -> Lv2Dispatch {
        let group = match self.groups.get_mut(group_id) {
            Some(g) => g,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
            }
        };

        // Two-pass: validate every handle, then build `inits`. The second
        // pass's `expect` requires `lookup_by_handle` to be a pure read.
        let slot_entries: Vec<_> = group.slots.iter().map(|(&k, v)| (k, v.clone())).collect();
        for (_slot_idx, slot) in &slot_entries {
            if self.content.lookup_by_handle(slot.image_handle).is_none() {
                return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
            }
        }

        let mut inits = std::collections::BTreeMap::new();
        for (slot_idx, slot) in &slot_entries {
            let record = self
                .content
                .lookup_by_handle(slot.image_handle)
                .expect("handle validated above");
            inits.insert(
                *slot_idx,
                SpuInitState {
                    ls_bytes: record.elf_bytes.clone(),
                    entry_pc: 0x80,
                    stack_ptr: 0x3FFF0,
                    args: slot.args,
                    group_id,
                },
            );
        }

        self.groups
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
    ) -> Lv2Dispatch {
        self.spu_thread_initialize_dispatches =
            self.spu_thread_initialize_dispatches.wrapping_add(1);
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

        let image_handle = match rt.read_committed(img_ptr as u64, 4) {
            Some(bytes) if bytes.len() >= 4 => {
                let fixed: [u8; 4] = bytes[0..4].try_into().expect("slice length checked above");
                u32::from_be_bytes(fixed)
            }
            _ => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
            }
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
                    for (i, chunk) in bytes.chunks_exact(8).enumerate().take(4) {
                        a[i] = u64::from_be_bytes(
                            chunk.try_into().expect("chunks_exact(8) yields [u8; 8]"),
                        );
                    }
                    a
                }
                _ => {
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                }
            }
        };

        if thread_num >= MAX_SLOTS_PER_GROUP {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        // Wrap near u32::MAX surfaces as EINVAL, not a silent collision.
        let thread_id = match group_id
            .checked_mul(MAX_SLOTS_PER_GROUP)
            .and_then(|base| base.checked_add(thread_num))
        {
            Some(id) => id,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
            }
        };

        // ContentStore never allocates handle 0; guest-supplied 0 -> ESRCH.
        let Some(handle) = crate::image::SpuImageHandle::new(image_handle) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        match self
            .groups
            .initialize_thread(group_id, thread_num, handle, args)
        {
            Ok(()) => {}
            Err(crate::thread_group::InitializeThreadError::UnknownGroup) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
            }
            Err(crate::thread_group::InitializeThreadError::SlotAlreadyInitialized) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into());
            }
            Err(_) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
            }
        }

        let range = ByteRange::contiguous_u32(thread_ptr, 4);
        let effect = Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::from_slice(&thread_id.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
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
    ) -> Lv2Dispatch {
        let group = match self.groups.get(group_id) {
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
                let mut effects = vec![];
                if cause_ptr != 0 {
                    let range = ByteRange::contiguous_u32(cause_ptr, 4);
                    effects.push(Effect::SharedWriteIntent {
                        range,
                        bytes: WritePayload::from_slice(
                            &sys_spu::group_join_cause::GROUP_EXIT.to_be_bytes(),
                        ),
                        ordering: PriorityClass::Normal,
                        source: requester,
                        source_time: self.current_tick,
                    });
                }
                if status_ptr != 0 {
                    let range = ByteRange::contiguous_u32(status_ptr, 4);
                    effects.push(Effect::SharedWriteIntent {
                        range,
                        bytes: WritePayload::from_slice(&0u32.to_be_bytes()),
                        ordering: PriorityClass::Normal,
                        source: requester,
                        source_time: self.current_tick,
                    });
                }
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
        // Non-running target: ESRCH. Silent drop would lose mailbox data.
        let target_uid = match self.groups.running_unit_for_thread(thread_id) {
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
