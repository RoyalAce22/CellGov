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
mod tests {
    use super::*;
    use crate::host::rsx::state::RSX_CONTEXT_ID;
    use crate::host::rsx::test_helpers::context_allocate_request;
    use crate::host::test_support::FakeRuntime;
    use crate::request::Lv2Request;
    use cellgov_event::UnitId;

    fn allocate_context(host: &mut Lv2Host, source: UnitId) {
        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 0xA001),
            source,
            &rt,
        );
        assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
    }

    #[test]
    fn sys_rsx_context_attribute_flip_emits_flip_request() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::FLIP_BUFFER,
                a3: 0,
                a4: 0x8000_0003,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::RsxFlipRequest { buffer_index: 3 }
        ));
    }

    #[test]
    fn sys_rsx_context_attribute_flip_queued_path_out_of_range_nibble_falls_back_with_witness() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);
        let pre_breaks = host.invariant_break_count();

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::FLIP_BUFFER,
                a3: 0,
                a4: 0x8000_0009, // queued path, nibble = 0x9 (out of range)
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert!(matches!(
            effects[0],
            Effect::RsxFlipRequest { buffer_index: 0 }
        ));
        assert!(
            host.invariant_break_count() > pre_breaks,
            "out-of-range nibble fallback must witness a log_invariant_break \
             so a future consumer can disambiguate slot-0-was-requested from \
             clamped-from-9",
        );
    }

    #[test]
    fn sys_rsx_context_attribute_flip_direct_path_resolves_index_by_offset_match() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        // Register slot 0 with offset 0x10_0000, slot 1 with offset 0x20_0000.
        for (id, off) in [(0u64, 0x10_0000u32), (1u64, 0x20_0000u32)] {
            host.dispatch(
                Lv2Request::SysRsxContextAttribute {
                    context_id: RSX_CONTEXT_ID,
                    package_id: package::SET_DISPLAY_BUFFER,
                    a3: id,
                    a4: (1920u64 << 32) | 1080,
                    a5: (0x2000u64 << 32) | (off as u64),
                    a6: 0,
                },
                source,
                &rt,
            );
        }

        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::FLIP_BUFFER,
                a3: 0,
                a4: 0x20_0000, // matches slot 1
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert!(
            matches!(effects[0], Effect::RsxFlipRequest { buffer_index: 1 }),
            "direct-path FLIP_BUFFER must resolve flip_target=0x20_0000 to slot 1 \
             (whose offset matches); fabricating 0 here would silently lose the \
             buffer identity for any future consumer that reads buffer_index",
        );
    }

    #[test]
    fn sys_rsx_context_attribute_flip_direct_path_no_match_falls_back_to_zero() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);
        let pre_breaks = host.invariant_break_count();

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::FLIP_BUFFER,
                a3: 0,
                a4: 0x0000_1234, // no display buffer registered with this offset
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert!(matches!(
            effects[0],
            Effect::RsxFlipRequest { buffer_index: 0 }
        ));
        assert!(
            host.invariant_break_count() > pre_breaks,
            "no-match fallback must witness a log_invariant_break so the \
             silent-substitution is non-vacuous; otherwise the 0 we synthesize \
             would be indistinguishable from a successful match against slot 0",
        );
    }

    #[test]
    fn sys_rsx_context_attribute_set_flip_handler_records_callback() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_CELLGOV_SET_FLIP_HANDLER,
                a3: 0xDEAD_BEEF,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
        assert_eq!(host.sys_rsx_context().flip_handler_addr, 0xDEAD_BEEF);
        assert_eq!(host.sys_rsx_context().vblank_handler_addr, 0);
        assert_eq!(host.sys_rsx_context().user_handler_addr, 0);
    }

    #[test]
    fn sys_rsx_context_attribute_set_vblank_handler_records_callback() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_CELLGOV_SET_VBLANK_HANDLER,
                a3: 0xCAFE_F00D,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert_eq!(host.sys_rsx_context().vblank_handler_addr, 0xCAFE_F00D);
    }

    #[test]
    fn sys_rsx_context_attribute_set_user_handler_records_callback() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_CELLGOV_SET_USER_HANDLER,
                a3: 0xABCD_0001,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert_eq!(host.sys_rsx_context().user_handler_addr, 0xABCD_0001);
    }

    #[test]
    fn sys_rsx_context_attribute_null_flip_handler_clears() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);
        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_CELLGOV_SET_FLIP_HANDLER,
                a3: 0x1234_5678,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_CELLGOV_SET_FLIP_HANDLER,
                a3: 0,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert_eq!(host.sys_rsx_context().flip_handler_addr, 0);
    }

    #[test]
    fn sys_rsx_context_attribute_flip_mode_records_mode() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::FLIP_MODE,
                a3: 0,
                a4: 2,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert_eq!(host.sys_rsx_context().flip_mode, 2);
    }

    #[test]
    fn sys_rsx_context_attribute_set_display_buffer_records_slot() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::SET_DISPLAY_BUFFER,
                a3: 1,
                a4: (1920u64 << 32) | 1080,
                a5: (0x2000u64 << 32) | 0x10_0000,
                a6: 0,
            },
            source,
            &rt,
        );
        let ctx = host.sys_rsx_context();
        assert_eq!(ctx.display_buffers_count, 2);
        let slot = ctx.display_buffers[1];
        assert_eq!(slot.width, 1920);
        assert_eq!(slot.height, 1080);
        assert_eq!(slot.pitch, 0x2000);
        assert_eq!(slot.offset, 0x10_0000);
    }

    #[test]
    fn sys_rsx_context_attribute_set_display_buffer_sparse_registration_leaves_count_dense() {
        // Spec: `display_buffers_count = max(id + 1, count)`. The
        // state hash captures the full slot array including
        // uninitialized entries; a future compaction would break
        // determinism.
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::SET_DISPLAY_BUFFER,
                a3: 3,
                a4: (1920u64 << 32) | 1080,
                a5: (0x2000u64 << 32) | 0xAABB_CCDD,
                a6: 0,
            },
            source,
            &rt,
        );
        let ctx = host.sys_rsx_context();
        assert_eq!(
            ctx.display_buffers_count, 4,
            "sparse registration of id=3 must leave count=4 (max(3+1, 0))",
        );
        assert_eq!(ctx.display_buffers[3].offset, 0xAABB_CCDD);
        // Slots 0..3 were never written; they hold their init-fill
        // value (zero). The hash includes these.
        for slot in &ctx.display_buffers[..3] {
            assert_eq!(slot.offset, 0);
            assert_eq!(slot.pitch, 0);
            assert_eq!(slot.width, 0);
            assert_eq!(slot.height, 0);
        }
    }

    #[test]
    fn sys_rsx_context_attribute_set_display_buffer_rejects_id_over_7() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::SET_DISPLAY_BUFFER,
                a3: 8,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, .. } if code == u64::from(cell_errors::CELL_EINVAL)
        ));
    }

    #[test]
    fn sys_rsx_context_attribute_unknown_package_returns_einval() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: 0xBEEF,
                a3: 0,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        let expected = u64::from(cell_errors::CELL_EINVAL);
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, effects } if code == expected && effects.is_empty()
        ));
    }

    #[test]
    fn sys_rsx_context_attribute_rejects_unallocated_context() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: 0x102,
                a3: 0,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            UnitId::new(0),
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, .. } if code == u64::from(cell_errors::CELL_EINVAL)
        ));
    }

    #[test]
    fn sys_rsx_context_attribute_fifo_setup_records_get_and_put() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        // Default FakeRuntime memory is 64 KiB, so MMIO at
        // 0xC000_0040 / 0xC000_0044 reads as non-writable -- the
        // reserved-region title path. FIFO_SETUP records the
        // pointers and emits ZERO effects.
        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::FIFO_SETUP,
                a3: 0x1000,
                a4: 0x2000,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert!(
            effects.is_empty(),
            "reserved-region (MMIO non-writable) FIFO_SETUP must emit no \
             SharedWriteIntent effects; otherwise the next batch's commit \
             would fail validation",
        );
        let ctx = host.sys_rsx_context();
        assert_eq!(ctx.fifo_get, 0x1000);
        assert_eq!(ctx.fifo_put, 0x2000);
    }

    #[test]
    fn sys_rsx_context_attribute_fifo_setup_emits_mmio_writes_when_writable() {
        // 40F honest-consumer companion: when the MMIO control-
        // register slots ARE writable (rsx_mirror=true title that
        // re-maps the RSX region writable), FIFO_SETUP emits two
        // SharedWriteIntent effects -- a4 (put) -> 0xC000_0040,
        // a3 (get) -> 0xC000_0044 -- so the engine-side cursor
        // sees the initial pointers from syscall return without
        // waiting for the title's first store. The effect
        // ordering (put first, then get) matches the cursor->MMIO
        // writeback in commit_step::mirror_rsx_cursor_to_mmio.
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000).with_writable_override(true);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::FIFO_SETUP,
                a3: 0x1100,
                a4: 0x2200,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert_eq!(
            effects.len(),
            2,
            "writable MMIO path must emit exactly the put and get writebacks",
        );

        // Effect 0: put -> PUT_ADDR (0xC000_0040), value = a4.
        let Effect::SharedWriteIntent {
            range: range0,
            bytes: bytes0,
            ..
        } = &effects[0]
        else {
            panic!(
                "expected SharedWriteIntent at index 0, got {:?}",
                effects[0]
            );
        };
        assert_eq!(
            range0.start().raw(),
            control_register::PUT_ADDR as u64,
            "effect 0 must target PUT_ADDR, not GET_ADDR (the cursor->MMIO \
             writeback in commit_step uses the same put-then-get ordering)",
        );
        assert_eq!(range0.length(), 4);
        assert_eq!(
            u32::from_be_bytes(bytes0.bytes().try_into().unwrap()),
            0x2200,
            "PUT slot must carry a4 (the put pointer), not a3",
        );

        // Effect 1: get -> GET_ADDR (0xC000_0044), value = a3.
        let Effect::SharedWriteIntent {
            range: range1,
            bytes: bytes1,
            ..
        } = &effects[1]
        else {
            panic!(
                "expected SharedWriteIntent at index 1, got {:?}",
                effects[1]
            );
        };
        assert_eq!(
            range1.start().raw(),
            control_register::GET_ADDR as u64,
            "effect 1 must target GET_ADDR; the put/get ordering swap would \
             type-check cleanly so this assertion is the catch",
        );
        assert_eq!(range1.length(), 4);
        assert_eq!(
            u32::from_be_bytes(bytes1.bytes().try_into().unwrap()),
            0x1100,
            "GET slot must carry a3 (the get pointer), not a4",
        );

        let ctx = host.sys_rsx_context();
        assert_eq!(ctx.fifo_get, 0x1100);
        assert_eq!(ctx.fifo_put, 0x2200);
    }

    /// F5 witness: the FIFO_SETUP writability gate is the conjunction
    /// `put_writable && get_writable`. Today's tests cover both
    /// writable (emit) and both non-writable (skip), but a refactor
    /// that drops the get-side check would pass both. These two
    /// asymmetric-writability tests prove each side independently
    /// gates the emit.
    #[test]
    fn sys_rsx_context_attribute_fifo_setup_skips_emit_when_only_put_writable() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000)
            .with_writable_at(control_register::PUT_ADDR as u64, true)
            .with_writable_at(control_register::GET_ADDR as u64, false);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::FIFO_SETUP,
                a3: 0x1100,
                a4: 0x2200,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert!(
            effects.is_empty(),
            "get-side non-writable must gate the emit even with put writable; \
             dropping the get_writable check from the conjunction would let \
             this case emit and the next batch's commit would fail validation",
        );
    }

    #[test]
    fn sys_rsx_context_attribute_fifo_setup_skips_emit_when_only_get_writable() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000)
            .with_writable_at(control_register::PUT_ADDR as u64, false)
            .with_writable_at(control_register::GET_ADDR as u64, true);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::FIFO_SETUP,
                a3: 0x1100,
                a4: 0x2200,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        let Lv2Dispatch::Immediate { code: 0, effects } = d else {
            panic!("expected Immediate(0), got {d:?}");
        };
        assert!(
            effects.is_empty(),
            "put-side non-writable must gate the emit even with get writable; \
             symmetric to the only-put-writable case, locks the conjunction \
             from either direction",
        );
    }

    #[test]
    fn sys_rsx_context_attribute_after_free_still_dispatches() {
        // Pins the noop-free contract: allocate -> free ->
        // context_attribute(same id) succeeds. If a future multi-
        // context model lands, free must clear `allocated` and this
        // test gets inverted.
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        host.dispatch(
            Lv2Request::SysRsxContextFree {
                context_id: RSX_CONTEXT_ID,
            },
            source,
            &rt,
        );

        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: PACKAGE_CELLGOV_SET_FLIP_HANDLER,
                a3: 0x1234_5678,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert!(
            matches!(d, Lv2Dispatch::Immediate { code: 0, .. }),
            "context_attribute after a noop-free must still dispatch \
             cleanly in the single-context model; allocated is never cleared",
        );
        assert_eq!(host.sys_rsx_context().flip_handler_addr, 0x1234_5678);
    }

    #[test]
    fn sys_rsx_context_attribute_rejects_wrong_context_id() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: 0xDEAD_BEEF,
                package_id: 0x102,
                a3: 0,
                a4: 0,
                a5: 0,
                a6: 0,
            },
            source,
            &rt,
        );
        assert!(matches!(
            d,
            Lv2Dispatch::Immediate { code, .. } if code == u64::from(cell_errors::CELL_EINVAL)
        ));
    }
}
