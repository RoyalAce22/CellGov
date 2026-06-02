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
    /// # Error precedence
    ///
    /// 1. `img_ptr` / `size` out of guest bounds -> CELL_EINVAL, no effects.
    /// 2. `handle_out` not writable for 16 bytes -> CELL_EFAULT, no effects.
    /// 3. Otherwise CELL_OK with one effect (the image struct write).
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
            bytes: WritePayload::new(img_struct.to_vec()),
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
            bytes: WritePayload::new(img_struct.to_vec()),
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
            bytes: WritePayload::new(group_id.to_be_bytes().to_vec()),
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
            bytes: WritePayload::new(thread_id.to_be_bytes().to_vec()),
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
                        bytes: WritePayload::new(
                            sys_spu::group_join_cause::GROUP_EXIT.to_be_bytes().to_vec(),
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
                        bytes: WritePayload::new(0u32.to_be_bytes().to_vec()),
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
mod tests {
    use super::*;
    use crate::host::test_support::FakeRuntime;
    use cellgov_mem::{GuestAddr, GuestMemory};
    use cellgov_time::GuestTicks;

    #[test]
    fn image_import_registers_distinct_entries_per_type_id_img_ptr() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let req1 = Lv2Request::SpuImageImport {
            handle_out: 0x100,
            img_ptr: 0x200,
            size: 32,
            type_id: 0xAA,
        };
        let req2 = Lv2Request::SpuImageImport {
            handle_out: 0x200,
            img_ptr: 0x400,
            size: 32,
            type_id: 0xAA,
        };
        let r1 = host.dispatch(req1, UnitId::new(0), &rt);
        let r2 = host.dispatch(req2, UnitId::new(0), &rt);
        let (h1, h2) = match (&r1, &r2) {
            (
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: e1,
                },
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: e2,
                },
            ) => {
                let Effect::SharedWriteIntent { bytes: b1, .. } = &e1[0] else {
                    panic!("e1");
                };
                let Effect::SharedWriteIntent { bytes: b2, .. } = &e2[0] else {
                    panic!("e2");
                };
                (
                    u32::from_be_bytes(b1.bytes()[..4].try_into().unwrap()),
                    u32::from_be_bytes(b2.bytes()[..4].try_into().unwrap()),
                )
            }
            other => panic!("expected two Immediate code=0, got {other:?}"),
        };
        assert_ne!(h1, h2, "same type_id+img_ptr-distinct entries");
    }

    #[test]
    fn image_import_out_of_range_img_ptr_returns_einval() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1000);
        let req = Lv2Request::SpuImageImport {
            handle_out: 0x100,
            img_ptr: 0x800,
            size: 0x1000, // 0x800 + 0x1000 = 0x1800 > 0x1000
            type_id: 1,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, cell_errors::CELL_EINVAL.into());
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn image_import_unwritable_handle_out_returns_efault() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1000);
        let req = Lv2Request::SpuImageImport {
            handle_out: 0xFF8, // 0xFF8 + 16 = 0x1008 > 0x1000
            img_ptr: 0x100,
            size: 32,
            type_id: 1,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, cell_errors::CELL_EFAULT.into());
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn image_open_out_of_range_path_returns_error() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let req = Lv2Request::SpuImageOpen {
            img_ptr: 0x1000,
            path_ptr: 0x2000,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_ne!(code, 0);
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn group_create_allocates_id_and_writes_to_guest() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x4000);
        let req = Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x3000,
            num_threads: 2,
            priority: 100,
            attr_ptr: 0x3800,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 1);
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                    assert_eq!(range.start().raw(), 0x3000);
                    assert_eq!(range.length(), 4);
                    assert_eq!(bytes.bytes(), &1u32.to_be_bytes());
                } else {
                    panic!("expected SharedWriteIntent");
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
        assert_eq!(host.thread_groups().len(), 1);
        let group = host.thread_groups().get(1).unwrap();
        assert_eq!(group.num_threads, 2);
    }

    #[test]
    fn group_create_allocates_monotonic_ids() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x4000);
        let r1 = host.dispatch(
            Lv2Request::SpuThreadGroupCreate {
                id_ptr: 0x100,
                num_threads: 1,
                priority: 0,
                attr_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        );
        let r2 = host.dispatch(
            Lv2Request::SpuThreadGroupCreate {
                id_ptr: 0x200,
                num_threads: 1,
                priority: 0,
                attr_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        );
        if let Lv2Dispatch::Immediate { effects, .. } = r1 {
            assert_eq!(
                effects[0].clone(),
                Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(0x100), 4).unwrap(),
                    bytes: WritePayload::new(1u32.to_be_bytes().to_vec()),
                    ordering: PriorityClass::Normal,
                    source: UnitId::new(0),
                    source_time: GuestTicks::ZERO,
                }
            );
        }
        if let Lv2Dispatch::Immediate { effects, .. } = r2 {
            assert_eq!(
                effects[0].clone(),
                Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(0x200), 4).unwrap(),
                    bytes: WritePayload::new(2u32.to_be_bytes().to_vec()),
                    ordering: PriorityClass::Normal,
                    source: UnitId::new(0),
                    source_time: GuestTicks::ZERO,
                }
            );
        }
    }

    #[test]
    fn group_create_rejects_oversized_num_threads() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x4000);
        let req = Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads: 300,
            priority: 0,
            attr_ptr: 0,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, cell_errors::CELL_EINVAL.into());
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
        assert_eq!(host.thread_groups().len(), 0);
    }

    #[test]
    fn thread_initialize_records_slot() {
        let mut host = Lv2Host::new();
        host.content_store_mut().register(b"/spu.elf", vec![0xAA]);

        // img_ptr at 0x200: handle=1 pre-populated (as image_open would write).
        let mut mem = GuestMemory::new(0x4000);
        let img_range = ByteRange::new(GuestAddr::new(0x200), 4).unwrap();
        mem.apply_commit(img_range, &1u32.to_be_bytes()).unwrap();
        let rt = FakeRuntime::with_memory(mem);

        host.dispatch(
            Lv2Request::SpuThreadGroupCreate {
                id_ptr: 0x100,
                num_threads: 2,
                priority: 0,
                attr_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        );
        let result = host.dispatch(
            Lv2Request::SpuThreadInitialize {
                thread_ptr: 0x300,
                group_id: 1,
                thread_num: 0,
                img_ptr: 0x200,
                attr_ptr: 0,
                arg_ptr: 0x1000,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 1);
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
        let group = host.thread_groups().get(1).unwrap();
        assert_eq!(group.slots.len(), 1);
        assert_eq!(group.slots[&0].image_handle.raw(), 1);
    }

    #[test]
    fn thread_initialize_unknown_group_returns_error() {
        let mut host = Lv2Host::new();
        let mut mem = GuestMemory::new(0x1000);
        let img_range = ByteRange::new(GuestAddr::new(0x200), 4).unwrap();
        mem.apply_commit(img_range, &1u32.to_be_bytes()).unwrap();
        let rt = FakeRuntime::with_memory(mem);
        let result = host.dispatch(
            Lv2Request::SpuThreadInitialize {
                thread_ptr: 0x300,
                group_id: 99,
                thread_num: 0,
                img_ptr: 0x200,
                attr_ptr: 0,
                arg_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_ne!(code, 0);
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn content_store_accessible_through_host() {
        let mut host = Lv2Host::new();
        assert!(host.content_store().is_empty());
        let h = host
            .content_store_mut()
            .register(b"/app_home/spu.elf", vec![1, 2, 3]);
        assert_eq!(h.raw(), 1);
        assert_eq!(host.content_store().len(), 1);
    }

    #[test]
    fn state_hash_changes_when_image_registered() {
        let empty = Lv2Host::new();
        let mut populated = Lv2Host::new();
        populated.content_store_mut().register(b"/spu.elf", vec![]);
        assert_ne!(empty.state_hash(), populated.state_hash());
    }

    #[test]
    fn state_hash_deterministic_across_instances() {
        let mut a = Lv2Host::new();
        let mut b = Lv2Host::new();
        a.content_store_mut().register(b"/spu.elf", vec![1, 2]);
        b.content_store_mut().register(b"/spu.elf", vec![1, 2]);
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn image_open_writes_struct_and_returns_cell_ok() {
        let mut host = Lv2Host::new();
        host.content_store_mut()
            .register(b"/app_home/spu.elf", vec![0xAA]);

        let mut mem = GuestMemory::new(0x300);
        let path = b"/app_home/spu.elf\0";
        let path_range = ByteRange::new(GuestAddr::new(0x100), path.len() as u64).unwrap();
        mem.apply_commit(path_range, path).unwrap();

        let rt = FakeRuntime::with_memory(mem);
        let req = Lv2Request::SpuImageOpen {
            img_ptr: 0x200,
            path_ptr: 0x100,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 1);
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                    assert_eq!(range.start().raw(), 0x200);
                    assert_eq!(range.length(), 16);
                    assert_eq!(&bytes.bytes()[0..4], &1u32.to_be_bytes());
                } else {
                    panic!("expected SharedWriteIntent");
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn image_open_unknown_path_returns_error() {
        let mut host = Lv2Host::new();
        let mut mem = GuestMemory::new(0x300);
        let path = b"/nonexistent.elf\0";
        let path_range = ByteRange::new(GuestAddr::new(0x100), path.len() as u64).unwrap();
        mem.apply_commit(path_range, path).unwrap();

        let rt = FakeRuntime::with_memory(mem);
        let req = Lv2Request::SpuImageOpen {
            img_ptr: 0x200,
            path_ptr: 0x100,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_ne!(code, 0);
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn image_open_bad_path_ptr_returns_error() {
        let host_with_image = {
            let mut h = Lv2Host::new();
            h.content_store_mut().register(b"/spu.elf", vec![]);
            h
        };
        let rt = FakeRuntime::new(64);
        let req = Lv2Request::SpuImageOpen {
            img_ptr: 0,
            path_ptr: 0xFFFF,
        };
        let result = host_with_image.clone().dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_ne!(code, 0);
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn image_open_handle_is_deterministic() {
        let make_host = || {
            let mut h = Lv2Host::new();
            h.content_store_mut().register(b"/spu.elf", vec![1, 2, 3]);
            h
        };

        let mut mem = GuestMemory::new(0x300);
        let path = b"/spu.elf\0";
        let path_range = ByteRange::new(GuestAddr::new(0x100), path.len() as u64).unwrap();
        mem.apply_commit(path_range, path).unwrap();
        let rt = FakeRuntime::with_memory(mem);

        let r1 = make_host().dispatch(
            Lv2Request::SpuImageOpen {
                img_ptr: 0x200,
                path_ptr: 0x100,
            },
            UnitId::new(0),
            &rt,
        );
        let r2 = make_host().dispatch(
            Lv2Request::SpuImageOpen {
                img_ptr: 0x200,
                path_ptr: 0x100,
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(r1, r2);
    }

    #[test]
    fn group_start_returns_register_spu_with_inits() {
        let mut host = Lv2Host::new();
        host.content_store_mut()
            .register(b"/spu.elf", vec![0xAA, 0xBB]);

        let mut mem = GuestMemory::new(0x4000);
        let path = b"/spu.elf\0";
        let path_range = ByteRange::new(GuestAddr::new(0x100), path.len() as u64).unwrap();
        mem.apply_commit(path_range, path).unwrap();
        let img_range = ByteRange::new(GuestAddr::new(0x300), 4).unwrap();
        mem.apply_commit(img_range, &1u32.to_be_bytes()).unwrap();

        // sys_spu_thread_argument: 4 x u64 big-endian; arg0 = 0x1000.
        let mut arg_bytes = [0u8; 32];
        arg_bytes[0..8].copy_from_slice(&0x1000u64.to_be_bytes());
        let arg_range = ByteRange::new(GuestAddr::new(0x200), 32).unwrap();
        mem.apply_commit(arg_range, &arg_bytes).unwrap();

        let rt = FakeRuntime::with_memory(mem);

        host.dispatch(
            Lv2Request::SpuImageOpen {
                img_ptr: 0x300,
                path_ptr: 0x100,
            },
            UnitId::new(0),
            &rt,
        );

        host.dispatch(
            Lv2Request::SpuThreadGroupCreate {
                id_ptr: 0x400,
                num_threads: 1,
                priority: 0,
                attr_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        );

        host.dispatch(
            Lv2Request::SpuThreadInitialize {
                thread_ptr: 0x500,
                group_id: 1,
                thread_num: 0,
                img_ptr: 0x300,
                attr_ptr: 0,
                arg_ptr: 0x200,
            },
            UnitId::new(0),
            &rt,
        );

        let result = host.dispatch(
            Lv2Request::SpuThreadGroupStart { group_id: 1 },
            UnitId::new(0),
            &rt,
        );

        match result {
            Lv2Dispatch::RegisterSpu { inits, code, .. } => {
                assert_eq!(code, 0);
                assert_eq!(inits.len(), 1);
                let init = inits.get(&0).expect("slot 0 init");
                assert_eq!(init.ls_bytes, vec![0xAA, 0xBB]);
                assert_eq!(init.entry_pc, 0x80);
                assert_eq!(init.stack_ptr, 0x3FFF0);
                assert_eq!(init.args[0], 0x1000);
                assert_eq!(init.group_id, 1);
                assert!(inits.contains_key(&0));
            }
            other => panic!("expected RegisterSpu, got {other:?}"),
        }
    }

    #[test]
    fn group_start_unknown_group_returns_error() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::SpuThreadGroupStart { group_id: 99 },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, .. } => assert_ne!(code, 0),
            other => panic!("expected Immediate error, got {other:?}"),
        }
    }
}
