//! `sys_rsx_context_attribute` (674) dispatch and package-id sub-handlers.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_rsx::{control_register, display_buffer, package};
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::runtime::Lv2Runtime;
use crate::host::Lv2Host;

use super::state::RsxDisplayBuffer;

/// CellGov-internal package id for the flip-handler callback. High bit set so
/// CellGov-internal ids stay disjoint from guest-visible PS3 package ids.
pub const PACKAGE_CELLGOV_SET_FLIP_HANDLER: u32 = 0x8000_0108;
/// CellGov-internal package id for the vblank-handler callback.
pub const PACKAGE_CELLGOV_SET_VBLANK_HANDLER: u32 = 0x8000_010C;
/// CellGov-internal package id for the user-handler callback.
pub const PACKAGE_CELLGOV_SET_USER_HANDLER: u32 = 0x8000_010D;

impl Lv2Host {
    /// `sys_rsx_context_attribute` (674): dispatches on `package_id`.
    ///
    /// # Cross-module contract
    ///
    /// Every package arm here commits on dispatch -- the mutation
    /// lands on `Lv2Host` / `RsxFlipState` at syscall-dispatch time,
    /// not through the commit pipeline's staging buffer. See
    /// `cellgov_core::runtime::Runtime::apply_lv2_effects` for the
    /// direct-commit contract. None of these mutations roll back if
    /// the containing batch's staged effects fail validation; the
    /// syscall has already returned to the guest.
    #[allow(
        clippy::too_many_arguments,
        reason = "Mirrors the syscall ABI surface (context_id + package_id + four \
                  payload words) plus the runtime view. Bundling payload words into \
                  a struct would obscure their per-package semantic interpretation \
                  at the dispatch site."
    )]
    pub(in crate::host) fn dispatch_sys_rsx_context_attribute(
        &mut self,
        context_id: u32,
        package_id: u32,
        a3: u64,
        a4: u64,
        a5: u64,
        _a6: u64,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        // `allocated && matching id` is a validity gate, not a
        // use-after-free check: `sys_rsx_context_free` (671) is a
        // noop and does not clear `allocated`, so a freed-then-
        // reallocated context re-attributing under the same id
        // passes here. Pinned by
        // sys_rsx_context_attribute_after_free_still_dispatches.
        if !self.rsx_context.allocated || context_id != self.rsx_context.context_id {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        match package_id {
            package::FIFO_SETUP => self.sys_rsx_attribute_fifo_setup(a3, a4, rt),
            package::FLIP_MODE => {
                self.rsx_context.flip_mode = a4 as u32;
                Lv2Dispatch::immediate(0)
            }
            package::FLIP_BUFFER => self.sys_rsx_attribute_flip(a3, a4),
            package::SET_DISPLAY_BUFFER => self.sys_rsx_attribute_set_display_buffer(a3, a4, a5),
            PACKAGE_CELLGOV_SET_FLIP_HANDLER => {
                self.rsx_context.flip_handler_addr = a3 as u32;
                Lv2Dispatch::immediate(0)
            }
            PACKAGE_CELLGOV_SET_VBLANK_HANDLER => {
                self.rsx_context.vblank_handler_addr = a3 as u32;
                Lv2Dispatch::immediate(0)
            }
            PACKAGE_CELLGOV_SET_USER_HANDLER => {
                self.rsx_context.user_handler_addr = a3 as u32;
                Lv2Dispatch::immediate(0)
            }
            _ => self.sys_rsx_attribute_unknown(package_id),
        }
    }

    /// FIFO_SETUP (0x001): records the initial FIFO get / put pointers
    /// and, when the MMIO control-register slots are guest-writable,
    /// emits the matching `SharedWriteIntent` effects. Reserved-
    /// region titles (mmio non-writable) skip the emit.
    fn sys_rsx_attribute_fifo_setup(
        &mut self,
        a3: u64,
        a4: u64,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        self.rsx_context.fifo_get = a3 as u32;
        self.rsx_context.fifo_put = a4 as u32;
        let put_writable = rt.writable(control_register::PUT_ADDR as u64, 4);
        let get_writable = rt.writable(control_register::GET_ADDR as u64, 4);
        if !(put_writable && get_writable) {
            return Lv2Dispatch::immediate(0);
        }
        let now = self.current_tick;
        let effects = mmio_init_effects(a3 as u32, a4 as u32, now);
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    /// SET_DISPLAY_BUFFER (0x104): records slot `id`; `display_buffers_count`
    /// only advances (monotonic to `id + 1`).
    ///
    /// Sparse registration is guest-legal: writing id=3 then id=1
    /// leaves `count == 4` with slots 0/2 holding their init-fill
    /// values, and the `RsxContext` state hash captures the full
    /// slot array including uninitialized entries.
    fn sys_rsx_attribute_set_display_buffer(&mut self, a3: u64, a4: u64, a5: u64) -> Lv2Dispatch {
        let id = (a3 & 0xFF) as usize;
        if id >= display_buffer::COUNT_MAX {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let width = (a4 >> 32) as u32;
        let height = a4 as u32;
        let pitch = (a5 >> 32) as u32;
        let offset = a5 as u32;
        self.rsx_context.display_buffers[id] = RsxDisplayBuffer {
            offset,
            pitch,
            width,
            height,
        };
        let next_count = (id as u32) + 1;
        if next_count > self.rsx_context.display_buffers_count {
            self.rsx_context.display_buffers_count = next_count;
        }
        Lv2Dispatch::immediate(0)
    }

    /// FLIP_BUFFER (0x102): emits [`Effect::RsxFlipRequest`].
    ///
    /// - Queued path (`flip_target & 0x8000_0000` set): low 4 bits
    ///   carry the buffer index. Nibbles >= `COUNT_MAX` clamp to 0
    ///   with a `log_invariant_break` (RPCS3's `lastQueuedBufferId`
    ///   fallback is not modeled).
    /// - Direct path: `flip_target` is a display-buffer offset; the
    ///   first slot in `display_buffers[0..display_buffers_count]`
    ///   whose `offset == flip_target` wins. No match clamps to 0
    ///   with a log.
    fn sys_rsx_attribute_flip(&mut self, _head: u64, flip_target: u64) -> Lv2Dispatch {
        let buffer_index: u8 = if (flip_target & 0x8000_0000) != 0 {
            self.resolve_queued_flip_buffer_index((flip_target & 0x0F) as u8)
        } else {
            self.resolve_direct_flip_buffer_index(flip_target as u32)
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::RsxFlipRequest { buffer_index }],
        }
    }

    /// Returns `nibble` if it indexes a valid display-buffer slot
    /// (`< COUNT_MAX`), else 0 with a `log_invariant_break`.
    fn resolve_queued_flip_buffer_index(&mut self, nibble: u8) -> u8 {
        if (nibble as usize) < display_buffer::COUNT_MAX {
            return nibble;
        }
        self.log_invariant_break(
            "dispatch.sys_rsx_context_attribute_flip_queued_nibble_out_of_range",
            format_args!(
                "FLIP_BUFFER queued path: low-nibble buffer index {nibble} \
                 is out of range (COUNT_MAX={cap}); RPCS3 would substitute \
                 lastQueuedBufferId here. Falling back to buffer_index=0 \
                 (no consumer for lastQueuedBufferId yet; the witness keeps \
                 the gap loud).",
                cap = display_buffer::COUNT_MAX,
            ),
        );
        0
    }

    /// Returns the first display-buffer slot whose recorded `offset`
    /// equals `target_offset`, else 0 with a `log_invariant_break`.
    fn resolve_direct_flip_buffer_index(&mut self, target_offset: u32) -> u8 {
        let count = self.rsx_context.display_buffers_count as usize;
        for (i, slot) in self.rsx_context.display_buffers[..count].iter().enumerate() {
            if slot.offset == target_offset {
                return i as u8;
            }
        }
        self.log_invariant_break(
            "dispatch.sys_rsx_context_attribute_flip_direct_no_buffer_match",
            format_args!(
                "FLIP_BUFFER direct path: no display_buffers[i].offset == \
                 0x{target_offset:08x} for i in 0..{count}; falling back to \
                 buffer_index=0 (matches RPCS3's case-0x102 else-branch error path)",
            ),
        );
        0
    }

    /// Fallback for unknown `package_id`: logs once via
    /// [`Lv2Host::log_invariant_break`], returns `CELL_EINVAL`.
    fn sys_rsx_attribute_unknown(&mut self, package_id: u32) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.sys_rsx_context_attribute_unsupported_package",
            format_args!(
                "sys_rsx_context_attribute package_id {package_id:#x} not yet wired; \
                 returning CELL_EINVAL (matches RPCS3 default-arm errno). \
                 Honest not-implemented, not an internal-invariant violation."
            ),
        );
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    }
}

/// FIFO_SETUP MMIO initialiser: write `put` -> 0xC000_0040 and
/// `get` -> 0xC000_0044 so the engine-side control-register slots
/// reflect FIFO_SETUP's a3/a4 from the moment the syscall returns.
///
/// # Cross-module contract
///
/// Same-batch ordering vs. a unit's PPU store to the same range is
/// set by commit_step: `commit_pipeline.process` runs first, then
/// `apply_lv2_effects` (these effects). The
/// `(PriorityClass, source_time, source)` triple is unused for
/// ordering today but tracks the workspace LV2-host convention.
fn mmio_init_effects(fifo_get: u32, fifo_put: u32, now: GuestTicks) -> Vec<Effect> {
    let make = |addr: u32, value: u32| {
        let range = ByteRange::new(GuestAddr::new(addr as u64), 4)
            .expect("MMIO control-register address + 4 fits in u64");
        Effect::SharedWriteIntent {
            range,
            bytes: WritePayload::from_slice(&value.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: UnitId::new(0),
            source_time: now,
        }
    };
    vec![
        make(control_register::PUT_ADDR, fifo_put),
        make(control_register::GET_ADDR, fifo_get),
    ]
}

#[cfg(test)]
#[path = "tests/attribute_tests.rs"]
mod tests;
