//! Effect-building primitives shared across the dispatch_route arms.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::Lv2Dispatch;

use crate::host::{Lv2Host, Lv2Runtime};
use cellgov_time::GuestTicks;

impl Lv2Host {
    /// Bind `N` u32-typed guest fields out of the 64-bit argument
    /// registers that carry them, or `None` when a register carries
    /// high bits.
    ///
    /// [`crate::request::classify`] gates every u32 slot it binds, so a
    /// typed request never carries a narrowed field. An arm that reads
    /// `Lv2Request::Unsupported`'s raw arguments has no such gate ahead
    /// of it. Every such arm binds its u32 fields here, before its own
    /// argument tests. That is the position the classifier's gate holds.
    ///
    /// Whether the kernel masks a field to 32 bits or refuses it is
    /// unestablished: every observed caller passes a value that already
    /// fits, so none separates the two answers. A reading of the
    /// kernel's syscall prologue fixes it.
    ///
    /// # Cross-module contract
    ///
    /// A caller answers `None` with `CELL_EINVAL`, the answer the
    /// classifier gives a malformed request. This method records each
    /// occurrence under `dispatch.arg_high_bits`.
    pub(in crate::host::dispatch_route) fn narrow_u32_args<const N: usize>(
        &mut self,
        number: u64,
        fields: [(&'static str, u64); N],
    ) -> Option<[u32; N]> {
        let mut narrowed = [0u32; N];
        for (slot, (name, value)) in narrowed.iter_mut().zip(fields) {
            let Ok(v) = u32::try_from(value) else {
                self.log_invariant_break(
                    "dispatch.arg_high_bits",
                    format_args!(
                        "syscall {number} u32 field {name}={value:#018x} carries high bits; \
                         returning CELL_EINVAL instead of answering about {low:#010x}",
                        low = value as u32
                    ),
                );
                return None;
            };
            *slot = v;
        }
        Some(narrowed)
    }

    /// Bind `N` `int`-typed guest fields out of the 64-bit argument
    /// registers that carry them, or `None` when a register is no sign
    /// extension of its own low word.
    ///
    /// PPC64 hands a guest `int x = -1` over as
    /// `0xFFFF_FFFF_FFFF_FFFF`, so the low word alone does not say what
    /// the caller passed. A register that reproduces its low word under
    /// sign extension carries an `int`. This method refuses any other
    /// register, such as `0x1_0000_0001`.
    ///
    /// [`Self::narrow_u32_args`] is the unsigned sibling. It gives the
    /// position both gates take ahead of an arm's own argument tests.
    /// It also names the kernel behaviour both gates leave
    /// unestablished.
    ///
    /// # Cross-module contract
    ///
    /// A caller answers `None` with `CELL_EINVAL`, the answer the
    /// classifier gives a malformed request. This method records each
    /// occurrence under `dispatch.arg_not_sign_extended`.
    pub(in crate::host::dispatch_route) fn narrow_i32_args<const N: usize>(
        &mut self,
        number: u64,
        fields: [(&'static str, u64); N],
    ) -> Option<[i32; N]> {
        let mut narrowed = [0i32; N];
        for (slot, (name, value)) in narrowed.iter_mut().zip(fields) {
            let Ok(v) = i32::try_from(value as i64) else {
                self.log_invariant_break(
                    "dispatch.arg_not_sign_extended",
                    format_args!(
                        "syscall {number} int field {name}={value:#018x} is no sign extension of \
                         its low word; returning CELL_EINVAL instead of answering about {low}",
                        low = value as i32
                    ),
                );
                return None;
            };
            *slot = v;
        }
        Some(narrowed)
    }

    /// Append the TTY buffer into the observability `tty_log` and
    /// write `nwritten` back.
    ///
    /// An unmapped buffer skips the append and still reports `len`
    /// written through `pwritelen` with CELL_OK. No established kernel
    /// behaviour decides that answer: it is CellGov's own choice.
    /// `dispatch.tty_write_buffer_unmapped` records each occurrence.
    /// The arm reads no `fd`: every descriptor appends to the one log.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` for a null `nwritten_ptr`. The append precedes
    ///   this check.
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
    /// for `_sys_prx_load_module` (480) and
    /// `_sys_prx_load_module_on_memcontainer` (497).
    ///
    /// Both syscalls share this arm. A registered path answers its
    /// kernel id as the syscall code. The arm does not read 497's
    /// container argument.
    ///
    /// Miss handling: a `/dev_flash/sys/external/` path whose stem
    /// names a module retail firmware ships (see
    /// `firmware_modules::FIRMWARE_MODULE_STEMS`) but is absent from
    /// the loaded firmware registers a stub entry under a real kernel
    /// id, and a repeat load resolves that stub by stem and returns
    /// the same id. Any other miss is `CELL_ENOENT`.
    ///
    /// The stub models no kernel behaviour. A console's `dev_flash`
    /// always holds the module, so the case cannot arise there. The
    /// stub keeps a boot moving when CellGov's installed firmware lacks the
    /// module, and `prx_load_hle_stub_count` counts each one.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` when the pointer is unreadable or no NUL
    ///   terminator appears within the 256-byte cap.
    /// - `CELL_ENOENT` for non-UTF-8 path bytes (CellGov-side
    ///   narrowing: such a path cannot name anything in the installed firmware).
    /// - `CELL_EINVAL` when `path_arg` carries high bits, per
    ///   [`Self::narrow_u32_args`].
    pub(super) fn resolve_prx_load(
        &mut self,
        number: u64,
        path_arg: u64,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        const PATH_CAP: usize = 256;
        const FIRMWARE_DIR: &str = "/dev_flash/sys/external/";
        let Some([path_ptr]) = self.narrow_u32_args(number, [("path", path_arg)]) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let Some(bytes) = rt.read_committed_until(u64::from(path_ptr), PATH_CAP, 0) else {
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
