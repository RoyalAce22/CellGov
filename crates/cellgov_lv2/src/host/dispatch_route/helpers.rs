//! Effect-building primitives shared across the dispatch_route arms.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::Lv2Dispatch;

use crate::host::{Lv2Host, Lv2Runtime};
use cellgov_time::GuestTicks;

impl Lv2Host {
    /// Append the TTY buffer into the observability `tty_log` and
    /// write `nwritten` back.
    ///
    /// An unmapped buffer skips the append and still reports `len`
    /// written through `pwritelen` with CELL_OK. No established kernel
    /// behaviour decides that answer: it is CellGov's own choice.
    /// `dispatch.tty_write_buffer_unmapped` records each occurrence.
    pub(super) fn dispatch_tty_write(
        &mut self,
        buf_ptr: u32,
        len: u32,
        nwritten_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if len > 0 {
            if let Some(bytes) = rt.read_committed(buf_ptr as u64, len as usize) {
                self.obs.tty_log.extend_from_slice(bytes);
            } else {
                self.log_invariant_break(
                    "dispatch.tty_write_buffer_unmapped",
                    format_args!(
                        "sys_tty_write buf={buf_ptr:#010x} len={len} is unreadable; the \
                         bytes are discarded and CELL_OK still reports {len} written"
                    ),
                );
            }
        }
        self.immediate_write_u32(len, nwritten_ptr, requester, tick)
    }

    /// Resolve the path at `path_ptr` against [`Self::prx_registry`]
    /// for syscalls 480 / 497.
    ///
    /// Miss handling: a `/dev_flash/sys/external/` path whose stem
    /// names a module retail firmware ships (see
    /// `firmware_modules::FIRMWARE_MODULE_STEMS`) but is absent from
    /// the loaded corpus registers a stub entry under a real kernel
    /// id, and a repeat load resolves that stub by stem and returns
    /// the same id. Any other miss is `CELL_ENOENT`.
    ///
    /// The stub models no kernel behaviour. A console's `dev_flash`
    /// always holds the module, so the case cannot arise there. The
    /// stub keeps a boot moving when CellGov's corpus lacks the
    /// module, and `prx_load_hle_stub_count` counts each one.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` when the pointer is unreadable or no NUL
    ///   terminator appears within the 256-byte cap.
    /// - `CELL_ENOENT` for non-UTF-8 path bytes (CellGov-side
    ///   narrowing: such a path cannot name anything in the corpus).
    pub(super) fn resolve_prx_load(&mut self, path_ptr: u64, rt: &dyn Lv2Runtime) -> Lv2Dispatch {
        const PATH_CAP: usize = 256;
        const FIRMWARE_DIR: &str = "/dev_flash/sys/external/";
        let Some(bytes) = rt.read_committed_until(path_ptr, PATH_CAP, 0) else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        debug_assert!(
            bytes.len() < PATH_CAP,
            "resolve_prx_load: read_committed_until returned a {PATH_CAP}-byte slice"
        );
        let Ok(path) = std::str::from_utf8(bytes) else {
            self.obs.prx_load_not_found_count += 1;
            return Lv2Dispatch::immediate(errno::CELL_ENOENT.into());
        };
        if let Some(entry) = self.state.prx_registry.lookup_by_path(path) {
            return Lv2Dispatch::immediate(u64::from(entry.kernel_id()));
        }
        let stem = crate::prx_registry::extract_stem(path);
        if path.starts_with(FIRMWARE_DIR) && super::firmware_modules::is_known_firmware_stem(&stem)
        {
            self.obs.prx_load_hle_stub_count += 1;
            let name = stem.clone();
            let id = self
                .state
                .prx_registry
                .register(stem, name, 0, 0, 0, None, None);
            return Lv2Dispatch::immediate(u64::from(id));
        }
        self.obs.prx_load_not_found_count += 1;
        *self.obs.prx_load_misses.entry(path.to_owned()).or_insert(0) += 1;
        Lv2Dispatch::immediate(errno::CELL_ENOENT.into())
    }

    /// Immediate dispatch writing `value` (BE u32) to `ptr` with
    /// `CELL_EFAULT` on null `ptr`.
    pub(in crate::host) fn immediate_write_u32(
        &self,
        value: u32,
        ptr: u32,
        source: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if ptr == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(ptr, 4),
            WritePayload::from_slice(&value.to_be_bytes()),
            source,
            tick,
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    pub(in crate::host) fn efault_if_null(&self, ptrs: &[u32]) -> Option<Lv2Dispatch> {
        if ptrs.contains(&0) {
            Some(Lv2Dispatch::immediate(errno::CELL_EFAULT.into()))
        } else {
            None
        }
    }
}
