//! SPU images: `sys_spu_image_import`, `sys_spu_image_open`,
//! `sys_spu_image_close`, the user-image parse at thread initialize, and
//! the image group start loads.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::format::elf::ELF32_E_ENTRY;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::spu;
use cellgov_time::GuestTicks;

use crate::dispatch::{Lv2Dispatch, SpuLoadImage};
use crate::host::guest_struct::read_be_u32;
use crate::host::{Lv2Host, Lv2Runtime};
use crate::image::LsSegment;

/// The `sys_spu_image` record the kernel hands back: `type` KERNEL,
/// with the kernel image id occupying the `entry_point` word.
fn kernel_image_struct(id: u32) -> [u8; 16] {
    let mut img_struct = [0u8; 16];
    img_struct[0..4].copy_from_slice(&spu::image::TYPE_KERNEL.to_be_bytes());
    img_struct[4..8].copy_from_slice(&id.to_be_bytes());
    img_struct
}

/// The first kernel image id minted for an image that already has a
/// live id. Content-store handles count up from 1 and stay below it.
const EXTRA_IMAGE_ID_BASE: u32 = 0x8000_0000;

/// The path `sys_spu_image_open` names at `path_ptr`, without its NUL,
/// or the code the kernel answers instead.
fn image_path(rt: &dyn Lv2Runtime, path_ptr: u32) -> Result<&[u8], errno::Lv2ErrCode> {
    let addr = u64::from(path_ptr);
    let Some(path) = rt.read_committed_until(addr, spu::IMAGE_PATH_MAX, 0) else {
        // No NUL sits within the window. If the whole window reads, the
        // kernel does not find the path; if it does not, the read faults.
        return Err(if rt.read_committed(addr, spu::IMAGE_PATH_MAX).is_some() {
            errno::CELL_ENOENT
        } else {
            errno::CELL_EFAULT
        });
    };
    if path
        .split(|&b| b == b'/')
        .any(|name| name.len() > spu::IMAGE_PATH_NAME_MAX)
    {
        return Err(errno::CELL_ENAMETOOLONG);
    }
    Ok(path)
}

/// Why a user-type `sys_spu_image` was refused at thread initialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UserImageRefusal {
    /// A record or segment table word could not be read.
    Unreadable,
    /// The record or a segment breaks a documented bound; the payload
    /// is the diagnostic site naming which one.
    Invalid(&'static str),
}

impl Lv2Host {
    /// Parse a user-type `sys_spu_image` into its entry and local-store segments.
    ///
    /// Segment bytes are snapshotted here rather than at group start,
    /// which has no guest-memory access.
    pub(super) fn parse_user_image(
        &self,
        img_ptr: u32,
        rt: &dyn Lv2Runtime,
    ) -> Result<(u32, Vec<LsSegment>), UserImageRefusal> {
        use spu::{image, segment, LS_SIZE};
        // Field addresses are formed in u64: a record or table row at
        // the top of the 32-bit space reads as unmapped (CELL_EFAULT)
        // instead of wrapping the guest pointer.
        let word = |addr: u64| read_be_u32(rt, addr).ok_or(UserImageRefusal::Unreadable);
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
            // `ls + size > LS_SIZE` is this model's own end check:
            // bounding `ls` and `size` separately still admits a
            // segment that runs off the end of local store. Group
            // start has no way to answer the guest.
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
            // A FILL range reads zero on the console whatever its
            // `value` word holds.
            let bytes = if seg_type == segment::TYPE_COPY {
                rt.read_committed(u64::from(addr), size as usize)
                    .ok_or(UserImageRefusal::Unreadable)?
                    .to_vec()
            } else {
                vec![0; size as usize]
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

    /// `sys_spu_image_import` (157): register `size` bytes at `img_ptr`
    /// in [`crate::image::ContentStore`] under a synthetic path, mint a
    /// kernel image id for it, and write the kernel record naming that
    /// id into the SPU image struct at `handle_out`.
    ///
    /// # Errors
    ///
    /// Each code is the one the `spu_image_import_raw` capture shows for
    /// that refusal alone; the order among them is CellGov's.
    ///
    /// - `CELL_EINVAL` when `size` is zero or does not fit in `usize`.
    /// - `CELL_EFAULT` when `size` bytes at `img_ptr` are unreadable.
    /// - `CELL_ENOEXEC` when the bytes do not start with the ELF magic.
    /// - `CELL_EFAULT` when `handle_out` is not writable for 16 bytes.
    #[allow(
        clippy::too_many_arguments,
        reason = "request payload plus the dispatch tick"
    )]
    pub(in crate::host) fn dispatch_image_import(
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
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        if size == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let img_bytes = match rt.read_committed(u64::from(img_ptr), size) {
            Some(b) => b,
            None => {
                return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
            }
        };
        if !img_bytes.starts_with(&cellgov_ps3_abi::format::elf::ELF_MAGIC) {
            return Lv2Dispatch::immediate(errno::CELL_ENOEXEC.into());
        }
        if !rt.writable(u64::from(handle_out), 16) {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        // Synthetic path keys every (type_id, img_ptr) pair to a distinct
        // entry; ELF parsing is deferred to sys_spu_thread_initialize. A
        // guest can import other bytes from an address it imported from
        // before, so a path that holds other bytes takes the next numeric
        // suffix: the store refuses to re-register a path with new bytes.
        let base = format!("/:import:{type_id:#x}:{img_ptr:#x}");
        let mut path = base.clone();
        let mut suffix = 0u32;
        while self
            .state
            .content
            .lookup_by_path(path.as_bytes())
            .is_some_and(|r| r.elf_bytes != img_bytes)
        {
            suffix += 1;
            path = format!("{base}:{suffix}");
        }
        let handle = self
            .content_store_mut()
            .register(path.as_bytes(), img_bytes.to_vec());
        let id = self.mint_image_id(handle);

        let img_struct = kernel_image_struct(id);
        let range = ByteRange::contiguous_u32(handle_out, 16);
        let effect = Effect::shared_write(
            range,
            WritePayload::from_slice(&img_struct),
            requester,
            tick,
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![effect],
        }
    }

    /// Mint a kernel image id for one open or import of `handle`.
    /// The first live id of an image is its content-store handle;
    /// an image that already has a live id gets the next id from
    /// [`EXTRA_IMAGE_ID_BASE`]. The `spu_image_close_edges` capture
    /// shows two imports of one ELF getting two ids, one closable
    /// without the other.
    fn mint_image_id(&mut self, handle: crate::image::SpuImageHandle) -> u32 {
        let first = handle.raw();
        let id = if self
            .state
            .spu_image_ids
            .get(first)
            .is_some_and(|&live| live != 0)
        {
            let extra = self
                .state
                .spu_image_ids
                .iter()
                .filter(|&(id, _)| id >= EXTRA_IMAGE_ID_BASE)
                .count();
            EXTRA_IMAGE_ID_BASE.saturating_add(u32::try_from(extra).unwrap_or(u32::MAX))
        } else {
            first
        };
        self.state.spu_image_ids.insert(id, first);
        id
    }

    /// `sys_spu_image_close` (158): close the kernel image id the
    /// record at `img_ptr` names.
    ///
    /// The id stays in the table, closed, and the image's bytes stay in
    /// the content store: a later open of the same path finds them
    /// again, and a kernel record naming the closed id initializes no
    /// thread.
    ///
    /// # Errors
    ///
    /// As the `spu_image_import_raw` and `spu_image_close_edges`
    /// captures show:
    ///
    /// - `CELL_EFAULT` when the record is unreadable.
    /// - `CELL_EINVAL` when the record is not a kernel record.
    /// - `CELL_ESRCH` when the record names a closed or unknown id; a
    ///   second close of one record is this case.
    pub(in crate::host) fn dispatch_spu_image_close(
        &mut self,
        img_ptr: u32,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let word = |offset: u32| read_be_u32(rt, u64::from(img_ptr) + u64::from(offset));
        let (Some(image_type), Some(handle)) = (
            word(spu::image::TYPE_OFFSET),
            word(spu::image::ENTRY_OFFSET),
        ) else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        if image_type != spu::image::TYPE_KERNEL {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        match self.state.spu_image_ids.get(handle).copied() {
            Some(live) if live != 0 => {
                self.state.spu_image_ids.insert(handle, 0);
                Lv2Dispatch::immediate(0)
            }
            _ => Lv2Dispatch::immediate(errno::CELL_ESRCH.into()),
        }
    }

    /// `sys_spu_image_open`: resolve the NUL-terminated path at
    /// `path_ptr` in the content store, mint a kernel image id for the
    /// image, and write the kernel-shaped `sys_spu_image` record naming
    /// that id to `img_ptr`.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` when the path bytes are unreadable.
    /// - `CELL_ENAMETOOLONG` when a name in the path is longer than
    ///   `IMAGE_PATH_NAME_MAX`.
    /// - `CELL_ENOENT` when no NUL sits within `IMAGE_PATH_MAX`.
    /// - `CELL_ENOENT` when the store holds no image at the path.
    pub(in crate::host) fn dispatch_image_open(
        &mut self,
        img_ptr: u32,
        path_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let path = match image_path(rt, path_ptr) {
            Ok(path) => path,
            Err(code) => return Lv2Dispatch::immediate(code.into()),
        };

        let record = match self.state.content.lookup_by_path(path) {
            Some(r) => r,
            None => {
                return Lv2Dispatch::immediate(errno::CELL_ENOENT.into());
            }
        };

        let handle = record.handle;
        let id = self.mint_image_id(handle);
        let img_struct = kernel_image_struct(id);

        let range = ByteRange::contiguous_u32(img_ptr, 16);
        let effect = Effect::shared_write(
            range,
            WritePayload::from_slice(&img_struct),
            requester,
            tick,
        );

        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![effect],
        }
    }

    /// Resolve an image handle to what group start loads and where it
    /// enters.
    ///
    /// A kernel image enters at its own `e_entry`; a user image has no
    /// header, so it enters where its record says. This function does
    /// not bound the entry. The SPU loader refuses an image shorter
    /// than an ELF32 header, and refuses an entry with no whole
    /// instruction word left in local store, so the 0 this reports for
    /// a truncated header never reaches a fetch.
    ///
    /// [CBE-Handbook p:421 s:14.6.3.3] control is transferred to the entry-point address the image's parameter area names.
    pub(super) fn load_image_for(
        &self,
        handle: crate::image::SpuImageHandle,
    ) -> Option<(SpuLoadImage, u32)> {
        if let Some(record) = self.state.content.lookup_by_handle(handle) {
            let entry = record
                .elf_bytes
                .get(ELF32_E_ENTRY..ELF32_E_ENTRY + 4)
                .and_then(|w| w.try_into().ok())
                .map_or(0, u32::from_be_bytes);
            return Some((SpuLoadImage::Elf(record.elf_bytes.clone()), entry));
        }
        let user = self.state.content.lookup_user_image(handle)?;
        Some((SpuLoadImage::Segments(user.segments.clone()), user.entry))
    }
}
