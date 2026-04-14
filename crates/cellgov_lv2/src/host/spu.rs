//! SPU-related LV2 dispatch methods.
//!
//! Split out of `host.rs` to keep the main dispatcher manageable.
//! Each function here is a method on `Lv2Host` that handles one
//! `Lv2Request` variant for the SPU lifecycle: image open, thread
//! group create/start/initialize/join, and mailbox write.

use cellgov_effects::{Effect, MailboxMessage, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_sync::MailboxId;
use cellgov_time::GuestTicks;

use crate::dispatch::{Lv2BlockReason, Lv2Dispatch, PendingResponse, SpuInitState};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::request::Lv2Request;

impl Lv2Host {
    pub(super) fn dispatch_image_open(
        &mut self,
        img_ptr: u32,
        path_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        // Read the NUL-terminated path string from guest memory.
        // Limit to 256 bytes to avoid unbounded reads.
        let path_bytes = match rt.read_committed(path_ptr as u64, 256) {
            Some(bytes) => bytes,
            None => {
                return Lv2Dispatch::Immediate {
                    code: 0x8001_0002, // CELL_EFAULT
                    effects: vec![],
                };
            }
        };
        let path_len = path_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(path_bytes.len());
        let path = &path_bytes[..path_len];

        let record = match self.content.lookup_by_path(path) {
            Some(r) => r,
            None => {
                return Lv2Dispatch::Immediate {
                    code: 0x8001_0002, // CELL_ENOENT
                    effects: vec![],
                };
            }
        };

        // Build the sys_spu_image_t struct (16 bytes, big-endian):
        //   offset 0: type/handle (u32)
        //   offset 4: entry point (u32) -- 0 for now, resolved at thread init
        //   offset 8: segments addr (u32) -- opaque, not used by CellGov
        //   offset 12: nsegs (i32) -- 0 for CellGov's purposes
        let handle = record.handle;
        let mut img_struct = [0u8; 16];
        img_struct[0..4].copy_from_slice(&handle.raw().to_be_bytes());

        let range =
            ByteRange::new(GuestAddr::new(img_ptr as u64), 16).expect("sys_spu_image_t range");
        let effect = Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::new(img_struct.to_vec()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: GuestTicks::ZERO,
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
        let group_id = self.groups.create(num_threads);

        // Write the allocated group id (big-endian u32) to the
        // caller's output pointer.
        let range = ByteRange::new(GuestAddr::new(id_ptr as u64), 4)
            .expect("sys_spu_thread_group_create id_ptr range");
        let effect = Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::new(group_id.to_be_bytes().to_vec()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: GuestTicks::ZERO,
        };

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![effect],
        }
    }

    pub(super) fn dispatch_group_start(&mut self, group_id: u32) -> Lv2Dispatch {
        let group = match self.groups.get_mut(group_id) {
            Some(g) => g,
            None => {
                return Lv2Dispatch::Immediate {
                    code: 0x8001_000D, // CELL_ESRCH
                    effects: vec![],
                };
            }
        };

        let mut inits = Vec::new();
        let slot_entries: Vec<_> = group.slots.iter().map(|(&k, v)| (k, v.clone())).collect();
        for (slot_idx, slot) in &slot_entries {
            let record = match self.content.lookup_by_handle(slot.image_handle) {
                Some(r) => r,
                None => {
                    return Lv2Dispatch::Immediate {
                        code: 0x8001_000D, // bad handle
                        effects: vec![],
                    };
                }
            };

            inits.push(SpuInitState {
                ls_bytes: record.elf_bytes.clone(),
                entry_pc: 0x80,
                stack_ptr: 0x3FFF0,
                args: slot.args,
                group_id,
                slot: *slot_idx,
            });
        }

        group.state = crate::thread_group::GroupState::Running;

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
            _ => unreachable!(),
        };

        // Read the image handle from the sys_spu_image_t struct at
        // img_ptr (first 4 bytes, big-endian u32).
        let image_handle = match rt.read_committed(img_ptr as u64, 4) {
            Some(bytes) => u32::from_be_bytes(bytes[0..4].try_into().unwrap()),
            None => {
                return Lv2Dispatch::Immediate {
                    code: 0x8001_0002, // CELL_EFAULT
                    effects: vec![],
                };
            }
        };

        // Read sys_spu_thread_argument (4x u64 BE) from guest memory
        // NOW, not at group_start time. The PPU may reuse the same
        // stack variable for multiple initialize calls.
        let args = if arg_ptr != 0 {
            match rt.read_committed(arg_ptr as u64, 32) {
                Some(bytes) => {
                    let mut a = [0u64; 4];
                    for (i, chunk) in bytes.chunks_exact(8).enumerate().take(4) {
                        a[i] = u64::from_be_bytes(chunk.try_into().unwrap());
                    }
                    a
                }
                None => [0u64; 4],
            }
        } else {
            [0u64; 4]
        };

        let handle = crate::dispatch::SpuImageHandle::new(image_handle);
        if !self
            .groups
            .initialize_thread(group_id, thread_num, handle, args)
        {
            return Lv2Dispatch::Immediate {
                code: 0x8001_000D, // CELL_ESRCH
                effects: vec![],
            };
        }

        // Write a thread id to *thread_ptr. Use a synthetic id
        // derived from group_id and thread_num.
        let thread_id = group_id * 256 + thread_num;
        let range = ByteRange::new(GuestAddr::new(thread_ptr as u64), 4).expect("thread_ptr range");
        let effect = Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::new(thread_id.to_be_bytes().to_vec()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: GuestTicks::ZERO,
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
    ) -> Lv2Dispatch {
        if self.groups.get(group_id).is_none() {
            return Lv2Dispatch::Immediate {
                code: 0x8001_000D, // CELL_ESRCH
                effects: vec![],
            };
        }
        Lv2Dispatch::Block {
            reason: Lv2BlockReason::ThreadGroupJoin { group_id },
            pending: PendingResponse::ThreadGroupJoin {
                group_id,
                code: 0,
                cause_ptr,
                status_ptr,
                cause: 0x0001, // SYS_SPU_THREAD_GROUP_JOIN_GROUP_EXIT
                status: 0,
            },
            effects: vec![],
        }
    }

    pub(super) fn dispatch_write_mb(
        &self,
        thread_id: u32,
        value: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let target_uid = match self.groups.unit_for_thread(thread_id) {
            Some(uid) => uid,
            None => {
                return Lv2Dispatch::Immediate {
                    code: 0x8001_000D, // CELL_ESRCH
                    effects: vec![],
                };
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
