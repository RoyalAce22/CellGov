//! `sys_rsx_context_attribute` (674) dispatch and package-id sub-handlers.

use cellgov_effects::Effect;
use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_rsx::{display_buffer, package};

use crate::dispatch::Lv2Dispatch;
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
    pub(in crate::host) fn dispatch_sys_rsx_context_attribute(
        &mut self,
        context_id: u32,
        package_id: u32,
        _a3: u64,
        _a4: u64,
        _a5: u64,
        _a6: u64,
    ) -> Lv2Dispatch {
        if !self.rsx_context.allocated || context_id != self.rsx_context.context_id {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        match package_id {
            package::FIFO_SETUP => self.sys_rsx_attribute_fifo_setup(_a3, _a4),
            package::FLIP_MODE => {
                self.rsx_context.flip_mode = _a4 as u32;
                Lv2Dispatch::immediate(0)
            }
            package::FLIP_BUFFER => self.sys_rsx_attribute_flip(_a3, _a4),
            package::SET_DISPLAY_BUFFER => self.sys_rsx_attribute_set_display_buffer(_a3, _a4, _a5),
            PACKAGE_CELLGOV_SET_FLIP_HANDLER => {
                self.rsx_context.flip_handler_addr = _a3 as u32;
                Lv2Dispatch::immediate(0)
            }
            PACKAGE_CELLGOV_SET_VBLANK_HANDLER => {
                self.rsx_context.vblank_handler_addr = _a3 as u32;
                Lv2Dispatch::immediate(0)
            }
            PACKAGE_CELLGOV_SET_USER_HANDLER => {
                self.rsx_context.user_handler_addr = _a3 as u32;
                Lv2Dispatch::immediate(0)
            }
            _ => self.sys_rsx_attribute_unknown(package_id),
        }
    }

    /// FIFO_SETUP (0x001): records the initial FIFO get / put pointers.
    fn sys_rsx_attribute_fifo_setup(&mut self, a3: u64, a4: u64) -> Lv2Dispatch {
        self.rsx_context.fifo_get = a3 as u32;
        self.rsx_context.fifo_put = a4 as u32;
        Lv2Dispatch::immediate(0)
    }

    /// SET_DISPLAY_BUFFER (0x104): records slot `id`; `display_buffers_count`
    /// only advances (monotonic to `id + 1`).
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

    /// FLIP_BUFFER (0x102): emits [`Effect::RsxFlipRequest`]; the commit
    /// pipeline drives WAITING -> DONE on the flip-status state machine.
    fn sys_rsx_attribute_flip(&self, _head: u64, flip_target: u64) -> Lv2Dispatch {
        // Queued path (high bit set): low 4 bits carry the buffer index.
        // Direct path: state machine keys on pending/done, not the index.
        let buffer_index: u8 = if (flip_target & 0x8000_0000) != 0 {
            (flip_target & 0x0F) as u8
        } else {
            0
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::RsxFlipRequest { buffer_index }],
        }
    }

    /// Fallback for unknown `package_id`: logs an invariant break, returns CELL_EINVAL.
    fn sys_rsx_attribute_unknown(&mut self, package_id: u32) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.sys_rsx_context_attribute_unknown_package",
            format_args!(
                "sys_rsx_context_attribute package_id {package_id:#x} not yet wired; \
                 returning CELL_EINVAL (matches RPCS3 default-arm errno)"
            ),
        );
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    }
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
    fn sys_rsx_context_attribute_flip_direct_path_uses_zero_index() {
        let mut host = Lv2Host::new();
        let source = UnitId::new(0);
        allocate_context(&mut host, source);

        let rt = FakeRuntime::new(0x1_0000);
        let d = host.dispatch(
            Lv2Request::SysRsxContextAttribute {
                context_id: RSX_CONTEXT_ID,
                package_id: package::FLIP_BUFFER,
                a3: 0,
                a4: 0x0000_1234,
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
        assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
        let ctx = host.sys_rsx_context();
        assert_eq!(ctx.fifo_get, 0x1000);
        assert_eq!(ctx.fifo_put, 0x2000);
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
