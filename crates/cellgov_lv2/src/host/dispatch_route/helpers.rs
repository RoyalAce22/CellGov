//! Effect-building primitives shared across the dispatch_route arms.
//!
//! - `dispatch_tty_write` is the buffer-append + nwritten-write fast
//!   path reached from both `TtyWrite` and `FsWrite`.
//! - `immediate_write_u32` is the create-style "alloc id + write to
//!   ptr" shape; visible to every host submodule so create-style
//!   dispatch helpers in `event_queue`, `cond`, `mutex`, etc. can
//!   share the EFAULT-on-null guard.
//! - `resolve_prx_load` is the path-lookup back-end for syscalls
//!   480 / 497.
//! - `efault_if_null` is the null-pointer EFAULT short-circuit.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;

use super::super::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// Append the TTY buffer into [`Self::tty_log`] and write
    /// `nwritten` back. An unmapped buffer skips the append and
    /// still reports `len` written.
    pub(super) fn dispatch_tty_write(
        &mut self,
        buf_ptr: u32,
        len: u32,
        nwritten_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if len > 0 {
            if let Some(bytes) = rt.read_committed(buf_ptr as u64, len as usize) {
                self.tty_log.extend_from_slice(bytes);
            }
        }
        self.immediate_write_u32(len, nwritten_ptr, requester)
    }

    /// Resolve the path at `path_ptr` against [`Self::prx_registry`]
    /// for syscalls 480 / 497. Returns the registered kernel id on
    /// match; on miss returns the path pointer as a synthetic id so
    /// non-foundation modules still see a distinct non-zero value.
    pub(super) fn resolve_prx_load(&self, path_ptr: u64, rt: &dyn Lv2Runtime) -> Lv2Dispatch {
        const PATH_CAP: usize = 256;
        let bytes = rt.read_committed_until(path_ptr, PATH_CAP, 0);
        // Per [`Lv2Runtime::read_committed_until`]: a `Some` return
        // has `len < max_len` (terminator stripped). A max-sized
        // slice means the contract drifted.
        debug_assert!(
            bytes.is_none_or(|b| b.len() < PATH_CAP),
            "resolve_prx_load: read_committed_until returned a {PATH_CAP}-byte slice"
        );
        let resolved = bytes
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|s| self.prx_registry.lookup_by_path(s))
            .map(|e| e.kernel_id());
        let code = match resolved {
            Some(id) => u64::from(id),
            None => path_ptr,
        };
        Lv2Dispatch::Immediate {
            code,
            effects: vec![],
        }
    }

    /// Build an immediate dispatch that writes `value` (BE u32) to
    /// `ptr` and returns CELL_OK; shared by create-style syscalls
    /// that emit a freshly allocated id through an out-pointer.
    /// Routes `ptr == 0` to `CELL_EFAULT` via [`Self::efault_if_null`].
    pub(in crate::host) fn immediate_write_u32(
        &self,
        value: u32,
        ptr: u32,
        source: UnitId,
    ) -> Lv2Dispatch {
        if ptr == 0 {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(ptr, 4),
            bytes: WritePayload::from_slice(&value.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// Pre-formed `CELL_EFAULT` dispatch when any pointer is null;
    /// short-circuits the spec'd EFAULT path before staging a
    /// write the commit pipeline would have rejected anyway, so
    /// the unit sees the documented errno instead of a commit fault.
    pub(super) fn efault_if_null(&self, ptrs: &[u32]) -> Option<Lv2Dispatch> {
        if ptrs.contains(&0) {
            Some(Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            })
        } else {
            None
        }
    }
}
