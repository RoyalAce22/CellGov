//! Effect-building primitives shared across the dispatch_route arms.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;

use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// Append the TTY buffer into [`Self::tty_log`] and write
    /// `nwritten` back.
    ///
    /// An unmapped buffer skips the append and still reports `len`
    /// written.
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
    /// for syscalls 480 / 497.
    ///
    /// On miss, returns the path pointer as a synthetic id so modules
    /// outside the minimum viable PRX set still get a distinct
    /// non-zero value.
    pub(super) fn resolve_prx_load(&self, path_ptr: u64, rt: &dyn Lv2Runtime) -> Lv2Dispatch {
        const PATH_CAP: usize = 256;
        let bytes = rt.read_committed_until(path_ptr, PATH_CAP, 0);
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

    /// Immediate dispatch writing `value` (BE u32) to `ptr` with
    /// `CELL_EFAULT` on null `ptr`.
    pub(in crate::host) fn immediate_write_u32(
        &self,
        value: u32,
        ptr: u32,
        source: UnitId,
    ) -> Lv2Dispatch {
        if ptr == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
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

    /// Returns a `CELL_EFAULT` dispatch when any of `ptrs` is null.
    pub(super) fn efault_if_null(&self, ptrs: &[u32]) -> Option<Lv2Dispatch> {
        if ptrs.contains(&0) {
            Some(Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()))
        } else {
            None
        }
    }
}
