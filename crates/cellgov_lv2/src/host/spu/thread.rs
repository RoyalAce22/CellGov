//! SPU thread setup: `sys_spu_thread_initialize`.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::spu;
use cellgov_time::GuestTicks;

use super::image::UserImageRefusal;
use crate::dispatch::Lv2Dispatch;
use crate::host::guest_struct::{read_be_u32, GuestStruct};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::request::Lv2Request;
use crate::thread_group::MAX_SLOTS_PER_GROUP;

impl Lv2Host {
    /// `sys_spu_thread_initialize`: bind an image and its arguments to
    /// slot `thread_num` of a group, and write the thread id to
    /// `thread_ptr`.
    ///
    /// The record's `type` word selects the path. A kernel record
    /// (from `sys_spu_image_open` / `sys_spu_image_import`) carries
    /// the image id in `entry_point`. A user record's 24-byte segment
    /// table must meet the kernel's bounds:
    ///
    /// - the entry is inside local store;
    /// - 1 to 32 segments;
    /// - at least one COPY segment, its source 4-byte aligned;
    /// - at most one INFO segment, of at most 256 bytes;
    /// - loadable segments 16-byte aligned, inside local store and
    ///   non-overlapping.
    ///
    /// Group start has no guest-memory access, so this arm snapshots
    /// a user record's bytes into local-store segments and copies the
    /// argument block. The slot index is free within the 256-entry
    /// map, so the arm accepts a slot past the declared thread count.
    /// Group destroy withdraws the user images.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` when `thread_num` is at or past
    ///   [`MAX_SLOTS_PER_GROUP`]; this screen precedes every pointer
    ///   read.
    /// - `CELL_EINVAL` when the record type is unknown, a user record
    ///   breaks a bound above, or the thread id would overflow.
    /// - `CELL_EFAULT` when the record, its segment table, a COPY
    ///   source, or the argument block is unreadable.
    /// - `CELL_ESRCH` when a kernel record names no image the kernel
    ///   holds, or the group is unknown.
    /// - `CELL_EBUSY` when the slot is occupied, the group is already
    ///   started, or its declared thread count is populated.
    pub(in crate::host) fn dispatch_thread_initialize(
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
                return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
            }
        };

        // This screen runs ahead of every pointer read, so an
        // out-of-range slot with an unreadable image pointer answers
        // CELL_EINVAL rather than CELL_EFAULT, as the
        // spu_group_init_limits capture shows.
        if thread_num >= MAX_SLOTS_PER_GROUP {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }

        // A kernel record carries the image id in `entry_point`.
        let image_word = |offset: u32| read_be_u32(rt, u64::from(img_ptr) + u64::from(offset));
        let Some(image_type) = image_word(spu::image::TYPE_OFFSET) else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        let (kernel_handle, user_image) = match image_type {
            spu::image::TYPE_KERNEL => {
                let Some(handle) = image_word(spu::image::ENTRY_OFFSET) else {
                    return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
                };
                (handle, None)
            }
            spu::image::TYPE_USER => match self.parse_user_image(img_ptr, rt) {
                Ok(parsed) => (0, Some(parsed)),
                Err(UserImageRefusal::Unreadable) => {
                    return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
                }
                Err(UserImageRefusal::Invalid(site)) => {
                    self.log_invariant_break(
                        site,
                        format_args!(
                            "sys_spu_thread_initialize refused the user image record at \
                             0x{img_ptr:08x}"
                        ),
                    );
                    return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
                }
            },
            _ => return Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
        };

        // Args snapshot at initialize time, not at group_start: the PPU
        // may reuse the same stack variable across calls. `arg_ptr == 0`
        // opts out; a non-zero pointer that fails to read is EFAULT.
        let args = if arg_ptr == 0 {
            [0u64; 4]
        } else {
            match GuestStruct::read(rt, arg_ptr as u64, 32) {
                Some(block) => std::array::from_fn(|i| block.u64_at(i * 8)),
                None => {
                    return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
                }
            }
        };

        let thread_id = match group_id
            .checked_mul(MAX_SLOTS_PER_GROUP)
            .and_then(|base| base.checked_add(thread_num))
        {
            Some(id) => id,
            None => {
                return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
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
                // A minted id resolves to its image's handle, and a closed
                // one is CELL_ESRCH. An id no open minted is read as a
                // content-store handle.
                let kernel_handle = match self.state.spu_image_ids.get(kernel_handle).copied() {
                    Some(0) => return Lv2Dispatch::immediate(errno::CELL_ESRCH.into()),
                    Some(handle) => handle,
                    None => kernel_handle,
                };
                // ContentStore never allocates handle 0; guest-supplied 0 -> ESRCH.
                let Some(handle) = crate::image::SpuImageHandle::new(kernel_handle) else {
                    return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
                };
                // The id must name an image the kernel holds; a lookup
                // miss is CELL_ESRCH. User-image handles are not
                // kernel ids, so a forged kernel record cannot alias
                // one at group start.
                if self.state.content.lookup_by_handle(handle).is_none() {
                    return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
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
                Some(errno::CELL_ESRCH)
            }
            Err(crate::thread_group::InitializeThreadError::SlotAlreadyInitialized) => {
                Some(errno::CELL_EBUSY)
            }
            // Once the group leaves its not-initialized state, this
            // answers CELL_EBUSY, the same code an occupied slot
            // takes.
            Err(crate::thread_group::InitializeThreadError::GroupAlreadyStarted { .. }) => {
                Some(errno::CELL_EBUSY)
            }
            // A fully populated group has left its not-initialized
            // state too, so it takes the same arm.
            Err(crate::thread_group::InitializeThreadError::GroupFull { .. }) => {
                Some(errno::CELL_EBUSY)
            }
            // A slot index past the thread map is a bad argument, not
            // a missing object: CELL_EINVAL.
            Err(crate::thread_group::InitializeThreadError::SlotOutOfRange) => {
                Some(errno::CELL_EINVAL)
            }
        };
        if let Some(code) = refusal {
            if let Some(user) = registered_user {
                self.content_store_mut().withdraw_user_image(user);
            }
            return Lv2Dispatch::immediate(code.into());
        }

        let range = ByteRange::contiguous_u32(thread_ptr, 4);
        let effect = Effect::shared_write(
            range,
            WritePayload::from_slice(&thread_id.to_be_bytes()),
            requester,
            tick,
        );

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![effect],
        }
    }
}
