//! `sys_fs_open` host dispatch.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::fs::{FileStat, FsError, SeekWhence};
use crate::host::{Lv2Host, Lv2Runtime};

/// `CELL_FS_MAX_PATH_LENGTH`. Counts the terminator: max content
/// is `MAX_PATH_LEN - 1` and the NUL must appear at index `<= 1023`.
const MAX_PATH_LEN: usize = 1024;

/// `O_CREAT` bit in PS3 LV2 oflag.
const O_CREAT: u32 = 0x4;

/// Wire size of `CellFsStat`. The PSL1GHT struct is
/// `{ s32 mode; s32 uid; s32 gid; <pad4>; s64 atime; s64 mtime;
/// s64 ctime; u64 size; u64 blksize }`; 8-byte alignment of the
/// 64-bit fields forces 4 bytes of padding between gid and atime.
const CELL_FS_STAT_SIZE: u64 = 56;

/// `CellFsStat::mode` for a regular read-only file: `S_IFREG`
/// (0x8000) plus `r--r--r--` (`IRUSR | IRGRP | IROTH` = 0x124).
/// Mirrors RPCS3's `CELL_FS_S_*` constants.
const CELL_FS_S_IFREG_R_ONLY_MODE: u32 = 0x8000 | 0x100 | 0x020 | 0x004;

/// `CellFsStat::blksize` reported for every file. 4096 is the
/// PS3-equivalent IO block size; titles that look at this field
/// are typically computing read buffer sizes.
const CELL_FS_BLOCK_SIZE: u64 = 4096;

/// Whether a 56-byte `CellFsStat` write at `stat_out_ptr` would
/// land in writable guest memory and the pointer satisfies 8-byte
/// alignment (needed for the embedded u64 fields).
fn is_stat_ptr_writable(rt: &dyn Lv2Runtime, stat_out_ptr: u32) -> bool {
    stat_out_ptr & 0x7 == 0 && rt.writable(stat_out_ptr as u64, CELL_FS_STAT_SIZE as usize)
}

/// Build the 56-byte big-endian `CellFsStat` payload and wrap it
/// in a `SharedWriteIntent` at `stat_out_ptr`. atime / mtime /
/// ctime are deterministic zeros (the oracle has no concept of
/// host time); blob content is immutable so a real timestamp
/// would be misleading.
fn cell_fs_stat_write(
    stat: FileStat,
    stat_out_ptr: u32,
    source: UnitId,
    source_time: cellgov_time::GuestTicks,
) -> Effect {
    let mut blob = [0u8; CELL_FS_STAT_SIZE as usize];
    blob[0..4].copy_from_slice(&CELL_FS_S_IFREG_R_ONLY_MODE.to_be_bytes());
    // uid (offset 4), gid (offset 8), pad (offset 12) all stay zero.
    // atime / mtime / ctime at offsets 16 / 24 / 32 stay zero.
    blob[40..48].copy_from_slice(&stat.size.to_be_bytes());
    blob[48..56].copy_from_slice(&CELL_FS_BLOCK_SIZE.to_be_bytes());
    Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(stat_out_ptr as u64), CELL_FS_STAT_SIZE)
            .expect("stat_out_ptr range pre-validated by is_stat_ptr_writable"),
        bytes: WritePayload::from_slice(&blob),
        ordering: PriorityClass::Normal,
        source,
        source_time,
    }
}

/// Paths whose `sys_fs_open` succeeds with a synthetic descriptor.
/// Real CellGov has no virtual filesystem; the whitelist exists so
/// PSL1GHT-test ELFs that probe the host's fs surface (presence of
/// `PARAM.SFO`, fopen / fclose plumbing, write-to-output.txt
/// fixtures) progress past their checks. The fd value is non-zero
/// but otherwise unspecified; `sys_fs_read` is not modeled and
/// `sys_fs_write` redirects bytes to the host's `tty_log` so the
/// harness can compare the captured stream against the test's
/// `.expected` file regardless of whether the test wrote via TTY or
/// via fopen.
const FS_OPEN_WHITELIST: &[&[u8]] = &[b"/app_home/PARAM.SFO", b"/app_home/output.txt"];

impl Lv2Host {
    pub(super) fn dispatch_fs_open(
        &mut self,
        path_ptr: u32,
        flags: u32,
        fd_out_ptr: u32,
        mode: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if fd_out_ptr & 0x3 != 0 || !rt.writable(fd_out_ptr as u64, 4) {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }

        let path_bytes_owned: Vec<u8> =
            match rt.read_committed_until(path_ptr as u64, MAX_PATH_LEN, 0) {
                Some(prefix) => prefix.to_vec(),
                None => {
                    // Disambiguate unmapped path_ptr (EFAULT) from
                    // no-NUL-within-limit (EINVAL) via a 1-byte probe.
                    let code = if rt.read_committed(path_ptr as u64, 1).is_none() {
                        errno::CELL_EFAULT
                    } else {
                        errno::CELL_EINVAL
                    };
                    return Lv2Dispatch::Immediate {
                        code: code.into(),
                        effects: vec![],
                    };
                }
            };

        // Manifest-registered blobs route through the in-memory FS
        // layer first; the whitelist + ENOENT path only fires for
        // paths the manifest does not name. Non-UTF-8 guest paths
        // (e.g. Shift-JIS save-data names from Japanese titles)
        // bypass the FS layer by construction -- manifest keys are
        // UTF-8 strings, so a non-UTF-8 path can never name a
        // registered blob. A future manifest schema that needs to
        // reach Shift-JIS paths replaces the from_utf8 gate with the
        // chosen decode policy.
        if let Ok(p) = std::str::from_utf8(&path_bytes_owned) {
            match self.fs_store_mut().open_fd(p) {
                Ok(fd) => {
                    // Mirror the whitelist path's fs_fd_count bump
                    // so sys_process_get_number_of_object stays
                    // consistent across both fd-allocation routes.
                    self.fs_fd_count_inc();
                    return self.immediate_write_u32(fd, fd_out_ptr, requester);
                }
                Err(FsError::FdExhausted) => {
                    return Lv2Dispatch::Immediate {
                        code: errno::CELL_EMFILE.into(),
                        effects: vec![],
                    };
                }
                Err(FsError::UnknownPath) => {
                    // Not in the manifest; fall through to whitelist
                    // / ENOENT.
                }
                Err(other) => {
                    // open_fd's contract: only UnknownPath or
                    // FdExhausted. Anything else means the FsError
                    // surface grew without the dispatcher being
                    // updated. Surface as CELL_EFAULT (reads as
                    // "host bug") rather than silently degrading to
                    // ENOENT, which would let titles fail-soft on a
                    // missing file and push the divergence
                    // downstream to cross-runner compare time.
                    let path_str = String::from_utf8_lossy(&path_bytes_owned);
                    self.record_invariant_break(
                        "dispatch.fs_open.unexpected_fs_error",
                        format_args!(
                            "FsStore::open_fd returned {other:?} for {path_str:?}; \
                             contract violated"
                        ),
                    );
                    return Lv2Dispatch::Immediate {
                        code: errno::CELL_EFAULT.into(),
                        effects: vec![],
                    };
                }
            }
        }

        if FS_OPEN_WHITELIST.contains(&path_bytes_owned.as_slice()) {
            // Whitelist hit: allocate a synthetic fd, increment the
            // open-fd counter, write the fd back to the guest.
            let fd = self.alloc_id();
            self.fs_fd_count_inc();
            let write = Effect::SharedWriteIntent {
                range: ByteRange::new(GuestAddr::new(fd_out_ptr as u64), 4)
                    .expect("fd_out_ptr range u32 fits in u64"),
                bytes: WritePayload::from_slice(&fd.to_be_bytes()),
                ordering: PriorityClass::Normal,
                source: requester,
                source_time: rt.current_tick(),
            };
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![write],
            };
        }

        // PS3 paths are byte-strings; localized titles routinely
        // embed UTF-8 / Shift-JIS, so lossy decode is the only
        // correct shape for the log. Allocated only on the ENOENT
        // path so the manifest-hit happy path stays alloc-free.
        let path_str = String::from_utf8_lossy(&path_bytes_owned);
        if flags & O_CREAT != 0 {
            eprintln!(
                "sys_fs_open: INVARIANT-BREAK O_CREAT requested for {path_str:?} \
                 (flags={flags:#x} mode={mode:#o}); whitelist is empty, returning CELL_ENOENT"
            );
        } else {
            eprintln!(
                "sys_fs_open: returning CELL_ENOENT for path {path_str:?} \
                 (flags={flags:#x} mode={mode:#o})"
            );
        }

        let write = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(fd_out_ptr as u64), 4)
                .expect("fd_out_ptr range u32 fits in u64"),
            // PS3 is big-endian; guest reads via `lwz`.
            bytes: WritePayload::from_slice(&0u32.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: rt.current_tick(),
        };
        Lv2Dispatch::Immediate {
            code: errno::CELL_ENOENT.into(),
            effects: vec![write],
        }
    }

    /// `sys_fs_read` -- read up to `nbytes` from `fd`'s current
    /// offset into `buf_ptr`, advance the offset by the actual count
    /// returned, and write that count to `nread_out_ptr`.
    ///
    /// # Error precedence
    ///
    /// In order:
    /// 1. `nread_out_ptr` misaligned / unwritable -> CELL_EFAULT, no
    ///    effects.
    /// 2. Unknown `fd` -> CELL_EBADF, no effects (no out-pointer
    ///    write so the guest cannot mistake stale memory for nread).
    /// 3. `nbytes > 0` and `buf_ptr` unwritable for that span ->
    ///    CELL_EFAULT, no effects. Crucially, this happens BEFORE
    ///    the FS layer advances the offset; per POSIX, a failed
    ///    read must not change the file position.
    /// 4. Otherwise CELL_OK with up to two effects: the buffer
    ///    write (only if bytes were returned) and the nread write.
    pub(super) fn dispatch_fs_read(
        &mut self,
        fd: u32,
        buf_ptr: u32,
        nbytes: u64,
        nread_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        // nread is a u64 (PSL1GHT signature: `u64 *nread`); enforce
        // 8-byte alignment and writability.
        if nread_out_ptr & 0x7 != 0 || !rt.writable(nread_out_ptr as u64, 8) {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }

        // Peek fd validity without advancing the offset. fstat is
        // read-only and returns UnknownFd for an unknown fd; that
        // is the EBADF surface. read_at(fd, 0) would also work
        // (0-byte reads do not advance the cursor) but fstat reads
        // less out of the table and conveys intent.
        if self.fs_store().fstat(fd).is_err() {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EBADF.into(),
                effects: vec![],
            };
        }

        // Pin nbytes into a usize for the FS layer. On 64-bit hosts
        // (the only target) this is identity for any plausible
        // value. The clamp is defensive against a hypothetical
        // 32-bit build where huge guest-supplied nbytes could
        // truncate.
        let nbytes_usize = usize::try_from(nbytes).unwrap_or(usize::MAX);

        // Validate the destination buffer BEFORE the FS layer
        // advances the offset. POSIX requires a failed read leave
        // the file position unchanged; doing the writable check
        // after read_at would advance the offset and then return
        // EFAULT, which is a semantic break.
        if nbytes > 0 && !rt.writable(buf_ptr as u64, nbytes_usize) {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }

        // fstat said the fd is valid, so read_at must not surface
        // UnknownFd here. UnknownPath would mean the blob was
        // removed from under an open fd -- single-write
        // registration forbids that. Anything else is contract
        // drift in FsStore.
        let bytes_read = match self.fs_store_mut().read_at(fd, nbytes_usize) {
            Ok(b) => b,
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_read.unexpected_fs_error",
                    format_args!(
                        "FsStore::read_at returned {other:?} for fd={fd:#x} \
                         (fstat said valid); contract violated"
                    ),
                );
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EFAULT.into(),
                    effects: vec![],
                };
            }
        };

        let nread = bytes_read.len() as u64;
        let tick = rt.current_tick();
        let mut effects = Vec::with_capacity(2);
        if !bytes_read.is_empty() {
            effects.push(Effect::SharedWriteIntent {
                range: ByteRange::new(GuestAddr::new(buf_ptr as u64), bytes_read.len() as u64)
                    .expect("buf_ptr range pre-validated by writable() above"),
                bytes: WritePayload::from_slice(&bytes_read),
                ordering: PriorityClass::Normal,
                source: requester,
                source_time: tick,
            });
        }
        effects.push(Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(nread_out_ptr as u64), 8)
                .expect("nread_out_ptr range pre-validated by writable() above"),
            // PS3 is big-endian; guest reads via `ld`.
            bytes: WritePayload::from_slice(&nread.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        });
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    /// `sys_fs_fstat` -- populate a `CellFsStat` (56 bytes) for an
    /// open fd's backing blob.
    ///
    /// # Error precedence
    ///
    /// 1. `stat_out_ptr` misaligned / unwritable for 56 bytes ->
    ///    CELL_EFAULT, no effects.
    /// 2. Unknown `fd` -> CELL_EBADF, no effects.
    /// 3. Otherwise CELL_OK with a single 56-byte struct write.
    pub(super) fn dispatch_fs_fstat(
        &mut self,
        fd: u32,
        stat_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !is_stat_ptr_writable(rt, stat_out_ptr) {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }
        let stat = match self.fs_store().fstat(fd) {
            Ok(s) => s,
            Err(FsError::UnknownFd) => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EBADF.into(),
                    effects: vec![],
                };
            }
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_fstat.unexpected_fs_error",
                    format_args!(
                        "FsStore::fstat returned {other:?} for fd={fd:#x}; \
                         contract violated"
                    ),
                );
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EFAULT.into(),
                    effects: vec![],
                };
            }
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![cell_fs_stat_write(
                stat,
                stat_out_ptr,
                requester,
                rt.current_tick(),
            )],
        }
    }

    /// `sys_fs_stat` -- path-keyed variant of `sys_fs_fstat`.
    ///
    /// # Error precedence
    ///
    /// 1. `stat_out_ptr` misaligned / unwritable for 56 bytes ->
    ///    CELL_EFAULT, no effects.
    /// 2. `path_ptr` unmapped or no NUL within `MAX_PATH_LEN` ->
    ///    CELL_EFAULT or CELL_EINVAL, no effects (mirrors
    ///    `dispatch_fs_open`).
    /// 3. Path not registered in the FS layer -> CELL_ENOENT, no
    ///    effects.
    /// 4. Otherwise CELL_OK with a single 56-byte struct write.
    pub(super) fn dispatch_fs_stat(
        &mut self,
        path_ptr: u32,
        stat_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !is_stat_ptr_writable(rt, stat_out_ptr) {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }
        let path_bytes_owned: Vec<u8> =
            match rt.read_committed_until(path_ptr as u64, MAX_PATH_LEN, 0) {
                Some(prefix) => prefix.to_vec(),
                None => {
                    let code = if rt.read_committed(path_ptr as u64, 1).is_none() {
                        errno::CELL_EFAULT
                    } else {
                        errno::CELL_EINVAL
                    };
                    return Lv2Dispatch::Immediate {
                        code: code.into(),
                        effects: vec![],
                    };
                }
            };
        // Non-UTF-8 paths can never match a manifest blob (manifest
        // keys are UTF-8); short-circuit to ENOENT before touching
        // FsStore.
        let path_str = match std::str::from_utf8(&path_bytes_owned) {
            Ok(s) => s,
            Err(_) => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_ENOENT.into(),
                    effects: vec![],
                };
            }
        };
        let stat = match self.fs_store().stat_path(path_str) {
            Ok(s) => s,
            Err(FsError::UnknownPath) => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_ENOENT.into(),
                    effects: vec![],
                };
            }
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_stat.unexpected_fs_error",
                    format_args!(
                        "FsStore::stat_path returned {other:?} for {path_str:?}; \
                         contract violated"
                    ),
                );
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EFAULT.into(),
                    effects: vec![],
                };
            }
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![cell_fs_stat_write(
                stat,
                stat_out_ptr,
                requester,
                rt.current_tick(),
            )],
        }
    }

    /// `sys_fs_lseek` -- move `fd`'s offset to a new absolute
    /// position under SEEK_SET / SEEK_CUR / SEEK_END semantics and
    /// write that position to `pos_out_ptr`.
    ///
    /// # Error precedence
    ///
    /// In order:
    /// 1. `pos_out_ptr` misaligned / unwritable -> CELL_EFAULT, no
    ///    effects. We bail before touching the fd table because no
    ///    other error can be reported (the new position would have
    ///    nowhere to land).
    /// 2. `whence` not in `{0, 1, 2}` -> CELL_EINVAL, no effects.
    ///    Cheap argument check before fd lookup.
    /// 3. Unknown `fd` -> CELL_EBADF, no effects.
    /// 4. Seek lands outside `[0, u64::MAX]` (negative-past-zero or
    ///    positive overflow) -> CELL_EINVAL, no effects. The fd's
    ///    offset is unchanged on this path; FsStore::seek validates
    ///    before mutating.
    /// 5. Otherwise CELL_OK with one effect: the new position
    ///    written as a big-endian u64 at `pos_out_ptr`.
    pub(super) fn dispatch_fs_lseek(
        &mut self,
        fd: u32,
        offset: i64,
        whence: u32,
        pos_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        // pos is a u64 (PSL1GHT signature: `u64 *pos`); enforce
        // 8-byte alignment and writability before any fd touch.
        if pos_out_ptr & 0x7 != 0 || !rt.writable(pos_out_ptr as u64, 8) {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }

        // Decode whence; out-of-range is CELL_EINVAL with no
        // out-pointer write. Done before fd lookup so a probe with
        // garbage whence does not need a valid fd to surface
        // EINVAL.
        let whence = match SeekWhence::from_guest(whence) {
            Some(w) => w,
            None => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EINVAL.into(),
                    effects: vec![],
                };
            }
        };

        let new_pos = match self.fs_store_mut().seek(fd, offset, whence) {
            Ok(p) => p,
            Err(FsError::UnknownFd) => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EBADF.into(),
                    effects: vec![],
                };
            }
            Err(FsError::SeekOutOfRange) => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EINVAL.into(),
                    effects: vec![],
                };
            }
            Err(other) => {
                // seek's contract: only UnknownFd / SeekOutOfRange
                // / UnknownPath. UnknownPath would mean the blob
                // disappeared from under an open fd -- single-write
                // registration forbids that. Anything else is
                // FsError surface drift.
                self.record_invariant_break(
                    "dispatch.fs_lseek.unexpected_fs_error",
                    format_args!(
                        "FsStore::seek returned {other:?} for fd={fd:#x} \
                         offset={offset} whence={whence:?}; contract violated"
                    ),
                );
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EFAULT.into(),
                    effects: vec![],
                };
            }
        };

        let write = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(pos_out_ptr as u64), 8)
                .expect("pos_out_ptr range pre-validated by writable() above"),
            // PS3 is big-endian; guest reads via `ld`.
            bytes: WritePayload::from_slice(&new_pos.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: rt.current_tick(),
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_fs_close` -- release an fd allocated via the FS layer.
    ///
    /// FsStore-tracked fds are removed from the open-fd table so
    /// subsequent reads / fstats / closes via the FS layer see them
    /// as unknown (CELL_EBADF, the spec-correct outcome). Unknown
    /// fds are deliberately silent: the legacy whitelist route in
    /// [`Self::dispatch_fs_open`] hands out fds via `alloc_id` and
    /// does not register them in FsStore, so `sys_fs_close` on a
    /// whitelist fd must look like an unknown-fd close from the
    /// FS layer's point of view. Returning EBADF would break
    /// PSL1GHT's `fclose` on whitelist paths (PARAM.SFO,
    /// output.txt). The "read after close returns EBADF" invariant
    /// still holds for FsStore fds because the table entry is
    /// gone.
    ///
    /// `fs_fd_count` is not decremented either way: real PS3 keeps
    /// the kernel-side fs-object count untouched across
    /// `sys_fs_close`, and the `sys_process_get_number_of_object`
    /// matrix in ps3autotests pins this.
    pub(super) fn dispatch_fs_close(&mut self, fd: u32) -> Lv2Dispatch {
        match self.fs_store_mut().close_fd(fd) {
            Ok(()) | Err(FsError::UnknownFd) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Err(other) => {
                // close_fd's contract: only Ok or UnknownFd.
                // Anything else means FsError grew without dispatch
                // being updated. Surface as host-bug EFAULT rather
                // than silently degrading to CELL_OK.
                self.record_invariant_break(
                    "dispatch.fs_close.unexpected_fs_error",
                    format_args!(
                        "FsStore::close_fd returned {other:?} for fd={fd:#x}; \
                         contract violated"
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: errno::CELL_EFAULT.into(),
                    effects: vec![],
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::FD_BASE;
    use crate::request::Lv2Request;
    use cellgov_time::GuestTicks;

    struct PathRuntime {
        bytes: Vec<u8>,
        reserved: Vec<(u32, u32)>,
        tick: GuestTicks,
    }

    impl PathRuntime {
        fn empty(size: usize) -> Self {
            Self {
                bytes: vec![0u8; size],
                reserved: Vec::new(),
                tick: GuestTicks::ZERO,
            }
        }

        fn write(mut self, addr: u32, payload: &[u8]) -> Self {
            let start = addr as usize;
            self.bytes[start..start + payload.len()].copy_from_slice(payload);
            self
        }

        fn reserve(mut self, start: u32, end: u32) -> Self {
            self.reserved.push((start, end));
            self
        }

        fn with_tick(mut self, tick: GuestTicks) -> Self {
            self.tick = tick;
            self
        }
    }

    impl Lv2Runtime for PathRuntime {
        fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]> {
            let start = addr as usize;
            let end = start.checked_add(len)?;
            if end > self.bytes.len() {
                return None;
            }
            for &(rs, re) in &self.reserved {
                let rs = rs as usize;
                let re = re as usize;
                if start < re && rs < end {
                    return None;
                }
            }
            Some(&self.bytes[start..end])
        }

        fn current_tick(&self) -> GuestTicks {
            self.tick
        }

        fn read_committed_until(&self, addr: u64, max_len: usize, terminator: u8) -> Option<&[u8]> {
            let start = addr as usize;
            self.read_committed(addr, 1)?;
            let mut len = 0usize;
            while len < max_len {
                let probe = self.read_committed(addr + len as u64, 1)?;
                if probe[0] == terminator {
                    return Some(&self.bytes[start..start + len]);
                }
                len += 1;
            }
            None
        }

        fn writable(&self, addr: u64, len: usize) -> bool {
            let Some(end) = (addr).checked_add(len as u64) else {
                return false;
            };
            if end > self.bytes.len() as u64 {
                return false;
            }
            let start = addr as usize;
            let end = end as usize;
            for &(rs, re) in &self.reserved {
                let rs = rs as usize;
                let re = re as usize;
                if start < re && rs < end {
                    return false;
                }
            }
            true
        }
    }

    fn run(host: &mut Lv2Host, rt: &dyn Lv2Runtime, request: Lv2Request) -> Lv2Dispatch {
        host.dispatch(request, UnitId::new(0), rt)
    }

    fn fs_open(path_ptr: u32, fd_out_ptr: u32, flags: u32, mode: u32) -> Lv2Request {
        Lv2Request::FsOpen {
            path_ptr,
            flags,
            fd_out_ptr,
            mode,
        }
    }

    fn assert_immediate(
        d: Lv2Dispatch,
        expected_code: u32,
        expected_effects: usize,
    ) -> Vec<Effect> {
        match d {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code as u32, expected_code, "errno mismatch");
                assert_eq!(effects.len(), expected_effects, "effect count mismatch");
                effects
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn unknown_path_returns_enoent_and_writes_zero_fd() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/dev_hdd0/game/foo/USRDIR/bar.dat\0");

        let effects = assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0x241, 0o666)),
            errno::CELL_ENOENT.code,
            1,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x20000);
                assert_eq!(bytes.bytes(), &0u32.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn path_without_null_terminator_returns_einval() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, &vec![b'A'; MAX_PATH_LEN]);

        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_EINVAL.code,
            0,
        );
    }

    #[test]
    fn out_of_range_path_ptr_returns_efault() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000);
        assert_immediate(
            run(&mut host, &rt, fs_open(0xFFFF_FF00, 0x20000, 0, 0)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn fd_out_ptr_unmapped_returns_efault_no_effects() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0xFFFF_FF00, 0, 0)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn fd_out_ptr_misaligned_returns_efault() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20001, 0, 0)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn fd_out_ptr_in_reserved_region_returns_efault() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000)
            .write(0x10000, b"/foo\0")
            .reserve(0x30000, 0x31000);
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x30100, 0, 0)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn path_at_region_end_succeeds() {
        // Place path in the last 5 bytes; pre-region-aware reads
        // EFAULTed here because a fixed 1024-byte window spilled
        // past the buffer.
        let mut host = Lv2Host::new();
        let path_ptr: u32 = 0x40000 - 5;
        let rt = PathRuntime::empty(0x40000).write(path_ptr, b"/foo\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(path_ptr, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
    }

    #[test]
    fn high_bit_bytes_in_path_succeed() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\xe6\x97\xa5\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
    }

    #[test]
    fn empty_path_returns_enoent() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
    }

    #[test]
    fn max_length_path_succeeds() {
        // 1023 content bytes + NUL: NUL at index MAX_PATH_LEN - 1
        // is the inclusive boundary the scan window must reach.
        let mut host = Lv2Host::new();
        let mut payload = vec![b'A'; MAX_PATH_LEN - 1];
        payload.push(0);
        let rt = PathRuntime::empty(0x40000).write(0x10000, &payload);
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
    }

    #[test]
    fn first_null_terminator_wins() {
        // Indirect: ENOENT (not EINVAL) confirms the scan stopped
        // at the first NUL. Tighten by reading back the logged
        // path once a structured invariant-break sink lands.
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0/bar\0");
        let effects = assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, .. } => {
                assert_eq!(range.start().raw(), 0x20000);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn o_creat_returns_enoent() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/save\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, O_CREAT, 0o666)),
            errno::CELL_ENOENT.code,
            1,
        );
    }

    #[test]
    fn source_time_matches_runtime_tick() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000)
            .write(0x10000, b"/foo\0")
            .with_tick(GuestTicks::new(42));
        let effects = assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { source_time, .. } => {
                assert_eq!(source_time.raw(), 42);
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    /// Pull the u32 fd payload out of a single SharedWriteIntent at
    /// `expected_addr`. Panics if the dispatch shape diverges.
    fn extract_fd(d: Lv2Dispatch, expected_addr: u64) -> u32 {
        let effects = match d {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0, "expected CELL_OK");
                effects
            }
            other => panic!("expected Immediate, got {other:?}"),
        };
        assert_eq!(effects.len(), 1, "expected exactly one effect");
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), expected_addr);
                assert_eq!(range.length(), 4);
                u32::from_be_bytes(bytes.bytes().try_into().unwrap())
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn registered_blob_path_routes_through_fs_layer() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/registered/foo".into(), b"bytes".to_vec())
            .unwrap();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/registered/foo\0");
        let fd = extract_fd(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            0x20000,
        );
        assert!(
            fd >= FD_BASE,
            "fs-layer fd must be in the FsStore range (>= FD_BASE), got {fd:#x}",
        );
        // FsStore tracked the open; kernel-side fs_fd_count tracked
        // it too -- both routes through sys_process_get_number_of_object
        // must agree on a single live fd.
        assert_eq!(host.fs_store().open_fd_count(), 1);
    }

    #[test]
    fn unknown_path_still_enoents_when_other_paths_are_registered() {
        // Mixed-state regression: a manifest with `/registered`
        // present must not change ENOENT for a different,
        // unregistered path. The FS-layer probe must miss cleanly
        // and fall through to the existing whitelist + ENOENT path.
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/registered".into(), b"x".to_vec())
            .unwrap();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/missing\0");
        let effects = assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_ENOENT.code,
            1,
        );
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), 0x20000);
                assert_eq!(bytes.bytes(), &0u32.to_be_bytes());
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
        // Failed FS lookup must not burn a host-side fd id.
        assert_eq!(host.fs_store().open_fd_count(), 0);
    }

    #[test]
    fn two_opens_of_same_registered_path_return_distinct_fds() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/dup".into(), b"x".to_vec())
            .unwrap();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/dup\0");
        let fd_a = extract_fd(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            0x20000,
        );
        let fd_b = extract_fd(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            0x20000,
        );
        assert_ne!(fd_a, fd_b, "second open must allocate a fresh fd");
        assert_eq!(host.fs_store().open_fd_count(), 2);
    }

    #[test]
    fn manifest_blob_shadows_whitelist_path() {
        // PARAM.SFO is in the static whitelist. Once the manifest
        // names it, the FS layer takes priority. The two routes
        // are not distinguishable by fd value alone (both
        // allocators happen to start at 0x4000_0001 today), so
        // the test pins routing via the FsStore side-effect:
        // open_fd_count = 1 means the FS layer ran; 0 means the
        // alloc_id whitelist branch ran.
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/app_home/PARAM.SFO".into(), b"sfo-bytes".to_vec())
            .unwrap();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/PARAM.SFO\0");
        let fd = extract_fd(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            0x20000,
        );
        assert!(fd >= FD_BASE);
        assert_eq!(
            host.fs_store().open_fd_count(),
            1,
            "manifest path must route through FsStore, not the whitelist; \
             if a future refactor unifies the two routes (e.g. whitelist \
             becomes synthetic register_blob at boot) this assertion will \
             still hold but `whitelist_path_still_works_when_manifest_is_silent` \
             will need to be rewritten -- the routing oracle is the pair",
        );
    }

    #[test]
    fn whitelist_path_still_works_when_manifest_is_silent() {
        // Whitelist coverage for PSL1GHT-test ELFs: a whitelist
        // path with no manifest entry still gets a synthetic fd via
        // the alloc_id route. The pairing with
        // `manifest_blob_shadows_whitelist_path` is the routing
        // oracle -- this test pins "FS layer was NOT consulted"
        // by checking FsStore stays empty.
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/PARAM.SFO\0");
        let fd = extract_fd(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            0x20000,
        );
        assert_ne!(fd, 0);
        assert_eq!(
            host.fs_store().open_fd_count(),
            0,
            "whitelist route must not register an fd in FsStore; if this \
             changed, the FS-layer routing was unified and this test \
             should assert fd-range membership against a distinct \
             allocator base instead",
        );
    }

    fn fs_read(fd: u32, buf_ptr: u32, nbytes: u64, nread_out_ptr: u32) -> Lv2Request {
        Lv2Request::FsRead {
            fd,
            buf_ptr,
            nbytes,
            nread_out_ptr,
        }
    }

    /// Open `path` against `host` (which must already have the blob
    /// registered) and return the allocated fd. Panics on any
    /// dispatch shape divergence.
    fn open_registered(host: &mut Lv2Host, path: &[u8]) -> (u32, PathRuntime) {
        // Lay out the path at 0x10000 and reserve the rest of the
        // 1 MiB sandbox for buffer / out-pointer use.
        let mut bytes = path.to_vec();
        bytes.push(0);
        let rt = PathRuntime::empty(0x100000).write(0x10000, &bytes);
        let fd = extract_fd(run(host, &rt, fs_open(0x10000, 0x20000, 0, 0)), 0x20000);
        (fd, rt)
    }

    /// Pull (nread, optional buffer-write bytes) from a successful
    /// FsRead dispatch. `expected_nread_addr` and
    /// `expected_buf_addr` pin the effect addresses.
    fn extract_read(
        d: Lv2Dispatch,
        expected_buf_addr: u64,
        expected_nread_addr: u64,
    ) -> (u64, Option<Vec<u8>>) {
        let effects = match d {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0, "expected CELL_OK");
                effects
            }
            other => panic!("expected Immediate, got {other:?}"),
        };
        // 1 effect = nread-only (zero-byte read); 2 effects = buffer + nread.
        match effects.as_slice() {
            [nread_effect] => match nread_effect {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    assert_eq!(range.start().raw(), expected_nread_addr);
                    assert_eq!(range.length(), 8);
                    let nread = u64::from_be_bytes(bytes.bytes().try_into().unwrap());
                    (nread, None)
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            },
            [buf_effect, nread_effect] => {
                let buf_bytes = match buf_effect {
                    Effect::SharedWriteIntent { range, bytes, .. } => {
                        assert_eq!(range.start().raw(), expected_buf_addr);
                        bytes.bytes().to_vec()
                    }
                    other => panic!("expected SharedWriteIntent, got {other:?}"),
                };
                let nread = match nread_effect {
                    Effect::SharedWriteIntent { range, bytes, .. } => {
                        assert_eq!(range.start().raw(), expected_nread_addr);
                        assert_eq!(range.length(), 8);
                        u64::from_be_bytes(bytes.bytes().try_into().unwrap())
                    }
                    other => panic!("expected SharedWriteIntent, got {other:?}"),
                };
                (nread, Some(buf_bytes))
            }
            other => panic!("expected 1 or 2 effects, got {other:?}"),
        }
    }

    #[test]
    fn read_full_file_returns_all_bytes_and_full_count() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"hello".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        let (nread, buf) = extract_read(
            run(&mut host, &rt, fs_read(fd, 0x30000, 5, 0x30100)),
            0x30000,
            0x30100,
        );
        assert_eq!(nread, 5);
        assert_eq!(buf.unwrap(), b"hello");
    }

    #[test]
    fn partial_read_advances_offset_and_second_read_returns_remainder() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"abcdef".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        let (n1, b1) = extract_read(
            run(&mut host, &rt, fs_read(fd, 0x30000, 3, 0x30100)),
            0x30000,
            0x30100,
        );
        assert_eq!(n1, 3);
        assert_eq!(b1.unwrap(), b"abc");
        let (n2, b2) = extract_read(
            run(&mut host, &rt, fs_read(fd, 0x30000, 3, 0x30100)),
            0x30000,
            0x30100,
        );
        assert_eq!(n2, 3);
        assert_eq!(b2.unwrap(), b"def");
    }

    #[test]
    fn read_past_eof_returns_zero_bytes_and_no_buffer_write() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"abc".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        // Drain the file.
        let (n, _) = extract_read(
            run(&mut host, &rt, fs_read(fd, 0x30000, 100, 0x30100)),
            0x30000,
            0x30100,
        );
        assert_eq!(n, 3);
        // Second read at EOF: nread=0, no buffer effect.
        let (n_eof, b_eof) = extract_read(
            run(&mut host, &rt, fs_read(fd, 0x30000, 100, 0x30100)),
            0x30000,
            0x30100,
        );
        assert_eq!(n_eof, 0);
        assert!(
            b_eof.is_none(),
            "EOF read must not emit a buffer write effect",
        );
    }

    #[test]
    fn read_with_zero_nbytes_returns_ok_with_only_nread_write() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"abc".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        let (n, b) = extract_read(
            run(&mut host, &rt, fs_read(fd, 0x30000, 0, 0x30100)),
            0x30000,
            0x30100,
        );
        assert_eq!(n, 0);
        assert!(b.is_none());
        // Offset must not advance: a follow-up real read still
        // returns the file from byte 0.
        let (n2, b2) = extract_read(
            run(&mut host, &rt, fs_read(fd, 0x30000, 3, 0x30100)),
            0x30000,
            0x30100,
        );
        assert_eq!(n2, 3);
        assert_eq!(b2.unwrap(), b"abc");
    }

    #[test]
    fn read_unknown_fd_returns_ebadf_with_no_effects() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x100000);
        // No fd ever opened; FsStore is empty.
        assert_immediate(
            run(&mut host, &rt, fs_read(0xCAFE_BABE, 0x30000, 8, 0x30100)),
            errno::CELL_EBADF.code,
            0,
        );
    }

    #[test]
    fn read_bad_buffer_pointer_returns_efault_and_does_not_advance_offset() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"abcdef".to_vec())
            .unwrap();
        let (fd, rt_with_path) = open_registered(&mut host, b"/foo");
        // Build a runtime with a reserved range covering the buffer
        // we are about to pass; writes there must EFAULT.
        let rt = PathRuntime::empty(0x100000)
            .write(0x10000, b"/foo\0")
            .reserve(0x30000, 0x31000);
        let _ = rt_with_path; // sandbox shape no longer needed.
        assert_immediate(
            run(&mut host, &rt, fs_read(fd, 0x30100, 3, 0x40000)),
            errno::CELL_EFAULT.code,
            0,
        );
        // Offset was not advanced: a subsequent valid read still
        // returns the file from byte 0.
        let (n, b) = extract_read(
            run(&mut host, &rt, fs_read(fd, 0x40010, 6, 0x40000)),
            0x40010,
            0x40000,
        );
        assert_eq!(n, 6);
        assert_eq!(b.unwrap(), b"abcdef");
    }

    #[test]
    fn read_misaligned_nread_pointer_returns_efault() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        // 8-byte alignment required; 0x30001 is misaligned.
        assert_immediate(
            run(&mut host, &rt, fs_read(fd, 0x30000, 1, 0x30001)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn read_unmapped_nread_pointer_returns_efault() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        assert_immediate(
            run(&mut host, &rt, fs_read(fd, 0x30000, 1, 0xFFFF_FF00)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn read_unknown_fd_takes_precedence_over_bad_buffer() {
        // Pin error precedence: even if the buffer is bad, an
        // unknown fd surfaces as EBADF first. The dispatcher must
        // not leak buffer-write attempts on an invalid fd.
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x100000).reserve(0x30000, 0x31000);
        assert_immediate(
            run(&mut host, &rt, fs_read(0xDEAD_BEEF, 0x30100, 4, 0x40000)),
            errno::CELL_EBADF.code,
            0,
        );
    }

    fn fs_close(fd: u32) -> Lv2Request {
        Lv2Request::FsClose { fd }
    }

    fn fs_fstat(fd: u32, stat_out_ptr: u32) -> Lv2Request {
        Lv2Request::FsFstat { fd, stat_out_ptr }
    }

    fn fs_stat(path_ptr: u32, stat_out_ptr: u32) -> Lv2Request {
        Lv2Request::FsStat {
            path_ptr,
            stat_out_ptr,
        }
    }

    /// Pull the 56-byte CellFsStat blob written by an FsFstat /
    /// FsStat success dispatch.
    fn extract_stat(d: Lv2Dispatch, expected_addr: u64) -> Vec<u8> {
        let effects = match d {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0, "expected CELL_OK");
                effects
            }
            other => panic!("expected Immediate, got {other:?}"),
        };
        assert_eq!(effects.len(), 1, "expected exactly one effect");
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), expected_addr);
                assert_eq!(range.length(), CELL_FS_STAT_SIZE);
                bytes.bytes().to_vec()
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    /// Read mode / size / blksize out of a CellFsStat blob.
    fn parse_stat(blob: &[u8]) -> (u32, u64, u64) {
        let mode = u32::from_be_bytes(blob[0..4].try_into().unwrap());
        let size = u64::from_be_bytes(blob[40..48].try_into().unwrap());
        let blksize = u64::from_be_bytes(blob[48..56].try_into().unwrap());
        (mode, size, blksize)
    }

    #[test]
    fn fstat_returns_size_mode_and_blksize() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"hello world".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        let blob = extract_stat(run(&mut host, &rt, fs_fstat(fd, 0x40000)), 0x40000);
        let (mode, size, blksize) = parse_stat(&blob);
        assert_eq!(mode, CELL_FS_S_IFREG_R_ONLY_MODE);
        assert_eq!(size, 11);
        assert_eq!(blksize, CELL_FS_BLOCK_SIZE);
    }

    #[test]
    fn fstat_pads_and_zeros_deterministic_fields() {
        // Pin the determinism contract: every CellFsStat byte that
        // is not size / mode / blksize is zero. The atime / mtime /
        // ctime fields and the gid->atime padding all read as
        // zeros so two stats of the same blob hash bit-identical.
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        let blob = extract_stat(run(&mut host, &rt, fs_fstat(fd, 0x40000)), 0x40000);
        // uid offset 4..8, gid offset 8..12, pad 12..16, atime 16..24,
        // mtime 24..32, ctime 32..40 -- all zero.
        for byte in &blob[4..40] {
            assert_eq!(*byte, 0, "deterministic-field byte must be zero");
        }
    }

    #[test]
    fn fstat_unknown_fd_returns_ebadf_with_no_effects() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x100000);
        assert_immediate(
            run(&mut host, &rt, fs_fstat(0xCAFE_BABE, 0x40000)),
            errno::CELL_EBADF.code,
            0,
        );
    }

    #[test]
    fn fstat_after_close_returns_ebadf() {
        // The "closed fd is unusable for fstat" invariant the
        // design doc calls out.
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
        assert_immediate(
            run(&mut host, &rt, fs_fstat(fd, 0x40000)),
            errno::CELL_EBADF.code,
            0,
        );
    }

    #[test]
    fn fstat_misaligned_stat_out_ptr_returns_efault() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        // 8-byte alignment required for the embedded u64 fields;
        // 0x40001 is misaligned.
        assert_immediate(
            run(&mut host, &rt, fs_fstat(fd, 0x40001)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn fstat_unmapped_stat_out_ptr_returns_efault() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        assert_immediate(
            run(&mut host, &rt, fs_fstat(fd, 0xFFFF_FF00)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn stat_known_path_returns_size() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/data.xml".into(), b"abcdef".to_vec())
            .unwrap();
        let rt = PathRuntime::empty(0x100000).write(0x10000, b"/data.xml\0");
        let blob = extract_stat(run(&mut host, &rt, fs_stat(0x10000, 0x40000)), 0x40000);
        let (mode, size, blksize) = parse_stat(&blob);
        assert_eq!(mode, CELL_FS_S_IFREG_R_ONLY_MODE);
        assert_eq!(size, 6);
        assert_eq!(blksize, CELL_FS_BLOCK_SIZE);
    }

    #[test]
    fn stat_unknown_path_returns_enoent_with_no_effects() {
        // No blob registered at /missing.
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x100000).write(0x10000, b"/missing\0");
        assert_immediate(
            run(&mut host, &rt, fs_stat(0x10000, 0x40000)),
            errno::CELL_ENOENT.code,
            0,
        );
    }

    #[test]
    fn stat_bad_path_pointer_returns_efault() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x100000);
        assert_immediate(
            run(&mut host, &rt, fs_stat(0xFFFF_FF00, 0x40000)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn stat_path_without_null_terminator_returns_einval() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x100000).write(0x10000, &vec![b'A'; MAX_PATH_LEN]);
        assert_immediate(
            run(&mut host, &rt, fs_stat(0x10000, 0x40000)),
            errno::CELL_EINVAL.code,
            0,
        );
    }

    #[test]
    fn stat_misaligned_stat_out_ptr_returns_efault_before_path_check() {
        // Pin precedence: bad stat_out_ptr is checked before path
        // validation. A probe with bad out-pointer and bad path
        // sees EFAULT, not EINVAL/ENOENT.
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x100000);
        assert_immediate(
            run(&mut host, &rt, fs_stat(0xFFFF_FF00, 0x40001)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    fn fs_lseek(fd: u32, offset: i64, whence: u32, pos_out_ptr: u32) -> Lv2Request {
        Lv2Request::FsLseek {
            fd,
            offset,
            whence,
            pos_out_ptr,
        }
    }

    /// Pull the new u64 position from a successful FsLseek dispatch.
    fn extract_pos(d: Lv2Dispatch, expected_addr: u64) -> u64 {
        let effects = match d {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0, "expected CELL_OK");
                effects
            }
            other => panic!("expected Immediate, got {other:?}"),
        };
        assert_eq!(effects.len(), 1, "expected exactly one effect");
        match &effects[0] {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), expected_addr);
                assert_eq!(range.length(), 8);
                u64::from_be_bytes(bytes.bytes().try_into().unwrap())
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn lseek_set_to_midfile_then_read_returns_expected_bytes() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"abcdef".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        let pos = extract_pos(
            run(&mut host, &rt, fs_lseek(fd, 4, 0 /* SET */, 0x30200)),
            0x30200,
        );
        assert_eq!(pos, 4);
        let (n, b) = extract_read(
            run(&mut host, &rt, fs_read(fd, 0x30000, 10, 0x30100)),
            0x30000,
            0x30100,
        );
        assert_eq!(n, 2);
        assert_eq!(b.unwrap(), b"ef");
    }

    #[test]
    fn lseek_end_returns_file_size() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"hello world".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        let pos = extract_pos(
            run(&mut host, &rt, fs_lseek(fd, 0, 2 /* END */, 0x30200)),
            0x30200,
        );
        assert_eq!(pos, 11);
    }

    #[test]
    fn lseek_cur_advances_relative_to_current_offset() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"abcdefghij".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        // Read 3 bytes -> offset = 3.
        let _ = run(&mut host, &rt, fs_read(fd, 0x30000, 3, 0x30100));
        // SEEK_CUR + 4 -> offset = 7.
        let pos = extract_pos(
            run(&mut host, &rt, fs_lseek(fd, 4, 1 /* CUR */, 0x30200)),
            0x30200,
        );
        assert_eq!(pos, 7);
        let (n, b) = extract_read(
            run(&mut host, &rt, fs_read(fd, 0x30000, 10, 0x30100)),
            0x30000,
            0x30100,
        );
        assert_eq!(n, 3);
        assert_eq!(b.unwrap(), b"hij");
    }

    #[test]
    fn lseek_unknown_fd_returns_ebadf() {
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x100000);
        assert_immediate(
            run(&mut host, &rt, fs_lseek(0xCAFE_BABE, 0, 0, 0x30200)),
            errno::CELL_EBADF.code,
            0,
        );
    }

    #[test]
    fn lseek_bad_whence_returns_einval() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        // whence=3 is not one of {0, 1, 2}.
        assert_immediate(
            run(&mut host, &rt, fs_lseek(fd, 0, 3, 0x30200)),
            errno::CELL_EINVAL.code,
            0,
        );
    }

    #[test]
    fn lseek_negative_past_zero_returns_einval() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"abc".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        // SET to -1 lands outside [0, u64::MAX] -> EINVAL.
        assert_immediate(
            run(&mut host, &rt, fs_lseek(fd, -1, 0 /* SET */, 0x30200)),
            errno::CELL_EINVAL.code,
            0,
        );
    }

    #[test]
    fn lseek_failed_seek_does_not_advance_offset() {
        // Pin: a CELL_EINVAL seek must leave the fd's offset alone
        // so a subsequent read still returns from where it was.
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"abc".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        // Move to offset 1 first.
        let _ = run(&mut host, &rt, fs_lseek(fd, 1, 0, 0x30200));
        // Failed seek (negative past zero).
        assert_immediate(
            run(&mut host, &rt, fs_lseek(fd, -10, 1 /* CUR */, 0x30200)),
            errno::CELL_EINVAL.code,
            0,
        );
        // Read from where we were (offset 1) -> 'bc'.
        let (n, b) = extract_read(
            run(&mut host, &rt, fs_read(fd, 0x30000, 5, 0x30100)),
            0x30000,
            0x30100,
        );
        assert_eq!(n, 2);
        assert_eq!(b.unwrap(), b"bc");
    }

    #[test]
    fn lseek_misaligned_pos_out_ptr_returns_efault() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        // 8-byte aligned required; 0x30201 is misaligned.
        assert_immediate(
            run(&mut host, &rt, fs_lseek(fd, 0, 0, 0x30201)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn lseek_unmapped_pos_out_ptr_returns_efault() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        assert_immediate(
            run(&mut host, &rt, fs_lseek(fd, 0, 0, 0xFFFF_FF00)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn lseek_bad_pos_out_ptr_takes_precedence_over_bad_whence_and_fd() {
        // Pin precedence: EFAULT on pos_out_ptr is checked before
        // whence-decode and fd lookup. A probe that gets bad
        // pointer, bad whence, and bad fd all wrong sees only
        // EFAULT.
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x100000);
        assert_immediate(
            run(&mut host, &rt, fs_lseek(0xDEAD_BEEF, 0, 99, 0x30201)),
            errno::CELL_EFAULT.code,
            0,
        );
    }

    #[test]
    fn lseek_bad_whence_takes_precedence_over_bad_fd() {
        // Pin precedence: EINVAL on whence is checked before fd
        // lookup. A probe with valid pos_out_ptr but bad whence
        // and bad fd sees EINVAL, not EBADF.
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x100000);
        assert_immediate(
            run(&mut host, &rt, fs_lseek(0xDEAD_BEEF, 0, 99, 0x30200)),
            errno::CELL_EINVAL.code,
            0,
        );
    }

    #[test]
    fn close_fsstore_fd_returns_ok_and_removes_from_table() {
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        assert_eq!(host.fs_store().open_fd_count(), 1);
        assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
        assert_eq!(
            host.fs_store().open_fd_count(),
            0,
            "FsStore close must remove the fd from the open-fd table",
        );
    }

    #[test]
    fn read_after_close_returns_ebadf() {
        // Spec-correct invariant: closed fds are not reusable for
        // reads. FsStore's close removes the entry, FsRead's fstat
        // peek then surfaces UnknownFd as CELL_EBADF.
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"abc".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
        assert_immediate(
            run(&mut host, &rt, fs_read(fd, 0x30000, 3, 0x30100)),
            errno::CELL_EBADF.code,
            0,
        );
    }

    #[test]
    fn close_unknown_fd_returns_ok_for_legacy_whitelist_compat() {
        // Deliberate divergence from the spec-correct EBADF: the
        // legacy whitelist route in dispatch_fs_open allocates fds
        // via alloc_id without registering them in FsStore, so
        // sys_fs_close on a whitelist fd looks like an unknown-fd
        // close from FsStore's point of view. Returning EBADF
        // would break PSL1GHT's fclose on PARAM.SFO / output.txt.
        // When the whitelist is retired (by registering the
        // synthetic blobs in FsStore at boot), this test should be
        // changed to assert CELL_EBADF for truly bogus fds.
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000);
        assert_immediate(run(&mut host, &rt, fs_close(0xDEAD_BEEF)), 0, 0);
    }

    #[test]
    fn double_close_on_fsstore_fd_treats_second_as_legacy_unknown() {
        // First close removes the fd from FsStore (spec-correct).
        // Second close looks identical to a "never-allocated" fd
        // and returns CELL_OK under the legacy compat policy.
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        let (fd, rt) = open_registered(&mut host, b"/foo");
        assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
        assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
    }

    #[test]
    fn close_legacy_whitelist_fd_returns_ok() {
        // Pin: opening PARAM.SFO via the whitelist route then
        // closing it must keep the existing CELL_OK behaviour. The
        // fd is alloc_id-sourced, never lands in FsStore.
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/PARAM.SFO\0");
        let fd = extract_fd(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            0x20000,
        );
        assert_eq!(
            host.fs_store().open_fd_count(),
            0,
            "whitelist route must not register an fd in FsStore",
        );
        assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
    }

    #[test]
    fn close_does_not_decrement_fs_fd_count() {
        // Real PS3 leaves fs_fd_count untouched across sys_fs_close;
        // the sys_process ps3autotest pins this. Open via the
        // whitelist (which bumps fs_fd_count), close it, then
        // confirm the count is still 1.
        let mut host = Lv2Host::new();
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/PARAM.SFO\0");
        let fd = extract_fd(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            0x20000,
        );
        // Probe fs_fd_count via sys_process_get_number_of_object
        // (class 0x73 = SYS_FS_FD_OBJECT). 4 bytes at 0x21000.
        let probe = Lv2Request::ProcessGetNumberOfObject {
            class_id: 0x73,
            count_out_ptr: 0x21000,
        };
        let probe_count = |host: &mut Lv2Host| -> u32 {
            let effects = match run(host, &rt, probe) {
                Lv2Dispatch::Immediate { code, effects } => {
                    assert_eq!(code, 0);
                    effects
                }
                other => panic!("expected Immediate, got {other:?}"),
            };
            match &effects[0] {
                Effect::SharedWriteIntent { bytes, .. } => {
                    u32::from_be_bytes(bytes.bytes().try_into().unwrap())
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            }
        };
        assert_eq!(probe_count(&mut host), 1, "fs_fd_count after open");
        assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
        assert_eq!(
            probe_count(&mut host),
            1,
            "fs_fd_count must NOT decrement on close (PS3-pinned behaviour)",
        );
    }

    #[test]
    fn fd_exhaustion_returns_emfile() {
        // Stage the FS-layer allocator at u32::MAX so the next
        // open_fd hits FdExhausted; the dispatch arm must surface
        // that as CELL_EMFILE rather than falling through to ENOENT.
        let mut host = Lv2Host::new();
        host.fs_store_mut()
            .register_blob("/hot".into(), b"x".to_vec())
            .unwrap();
        // Burn the allocator down to the cap. open_fd at
        // u32::MAX - 1 succeeds and leaves next_fd = u32::MAX; the
        // following call exhausts.
        host.fs_store_mut().force_next_fd_for_test(u32::MAX);
        let rt = PathRuntime::empty(0x40000).write(0x10000, b"/hot\0");
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            errno::CELL_EMFILE.code,
            0,
        );
    }
}
