//! HLE (High-Level Emulation) router.
//!
//! Each PS3 system library is one file in this directory (`sys.rs`
//! for sysPrxForUser, `gcm.rs` for cellGcmSys, and so on) that
//! owns its NID constants, its handler bodies, and a `dispatch`
//! entry point returning `Option<()>`. This file chains those
//! entry points; the first module to claim a NID wins, and
//! unclaimed NIDs fall through to the CELL_OK default.
//!
//! Scaling rule: a new PS3 library means a new
//! `crates/cellgov_core/src/hle/<module>.rs` file, one `pub mod`
//! declaration below, and one line added to the chain. No
//! registration macro, no mutable global dispatch table, no
//! link-time wiring.

use cellgov_event::UnitId;

use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

pub mod context;
pub(crate) mod gcm;
pub(crate) mod sys;

impl Runtime {
    /// Dispatch an HLE import call by NID.
    ///
    /// Walks the module chain in priority order. If no module
    /// claims the NID, the default is `r3 = 0` (CELL_OK) with no
    /// effects -- the safe fallback for unobserved calls.
    pub(crate) fn dispatch_hle(&mut self, source: UnitId, nid: u32, args: &[u64; 9]) {
        let handled = sys::dispatch(self, source, nid, args)
            .or_else(|| gcm::dispatch(self, source, nid, args));
        if handled.is_none() {
            RuntimeHleAdapter {
                memory: &mut self.memory,
                registry: &mut self.registry,
                heap_ptr: &mut self.hle_heap_ptr,
                next_id: &mut self.hle_next_id,
                source,
            }
            .set_return(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::hle::gcm::tiled_pitch_lookup;
    use crate::hle::gcm::{
        NID_CELLGCM_GET_CONFIGURATION, NID_CELLGCM_GET_LABEL_ADDRESS, NID_CELLGCM_INIT_BODY,
    };
    use crate::hle::sys::{NID_SYS_PPU_THREAD_GET_ID, NID_SYS_TIME_GET_SYSTEM_TIME};

    #[test]
    fn tiled_pitch_exact_boundary() {
        assert_eq!(tiled_pitch_lookup(0x200), 0x200);
        assert_eq!(tiled_pitch_lookup(0x300), 0x300);
        assert_eq!(tiled_pitch_lookup(0x10000), 0x10000);
    }

    #[test]
    fn tiled_pitch_between_entries() {
        assert_eq!(tiled_pitch_lookup(0x250), 0x300);
        assert_eq!(tiled_pitch_lookup(0x1), 0x200);
        assert_eq!(tiled_pitch_lookup(0x801), 0xA00);
    }

    #[test]
    fn tiled_pitch_zero_returns_zero() {
        assert_eq!(tiled_pitch_lookup(0), 0);
    }

    #[test]
    fn tiled_pitch_above_max_returns_zero() {
        assert_eq!(tiled_pitch_lookup(0x10001), 0);
    }

    #[test]
    fn tiled_pitches_table_is_sorted() {
        use crate::hle::gcm::TILED_PITCHES;
        for w in TILED_PITCHES.windows(2) {
            assert!(w[0] < w[1], "table not sorted: {} >= {}", w[0], w[1]);
        }
    }

    #[test]
    fn gcm_init_body_writes_context_and_callback() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x200000), Budget::new(1), 100);
        rt.set_hle_heap_base(0x100000);
        rt.set_gcm_rsx_checkpoint(true);

        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0x10000, 0x10000, 0x8000, 0x80000, 0x20000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELLGCM_INIT_BODY, &args);

        let mem = rt.memory().as_bytes();
        let ctx_ptr = u32::from_be_bytes([mem[0x10000], mem[0x10001], mem[0x10002], mem[0x10003]]);
        assert_ne!(ctx_ptr, 0, "context pointer should be non-zero");

        let a = ctx_ptr as usize;
        let begin = u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]]);
        let end = u32::from_be_bytes([mem[a + 4], mem[a + 5], mem[a + 6], mem[a + 7]]);
        let callback = u32::from_be_bytes([mem[a + 12], mem[a + 13], mem[a + 14], mem[a + 15]]);
        assert_eq!(begin, 0x20000 + 0x1000, "begin = ioAddress + 0x1000");
        assert!(end > begin, "end > begin");
        assert_ne!(callback, 0, "callback OPD should be non-zero");

        assert_eq!(rt.gcm_state.control_addr, 0xC000_0040);
    }

    #[test]
    fn gcm_get_configuration_writes_config() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x200000), Budget::new(1), 100);
        rt.set_hle_heap_base(0x100000);
        rt.set_gcm_rsx_checkpoint(true);
        rt.gcm_state.io_address = 0x20000;
        rt.gcm_state.io_size = 0x80000;
        rt.gcm_state.local_size = 0x0f90_0000;

        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0x10000, 0x10000, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELLGCM_GET_CONFIGURATION, &args);

        let mem = rt.memory().as_bytes();
        let a = 0x10000usize;
        let local_addr = u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]]);
        let io_addr = u32::from_be_bytes([mem[a + 4], mem[a + 5], mem[a + 6], mem[a + 7]]);
        let local_size = u32::from_be_bytes([mem[a + 8], mem[a + 9], mem[a + 10], mem[a + 11]]);
        let io_size = u32::from_be_bytes([mem[a + 12], mem[a + 13], mem[a + 14], mem[a + 15]]);
        assert_eq!(local_addr, 0xC000_0000);
        assert_eq!(io_addr, 0x20000);
        assert_eq!(local_size, 0x0f90_0000);
        assert_eq!(io_size, 0x80000);
    }

    #[test]
    fn gcm_get_label_address_returns_indexed_offset() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x200000), Budget::new(1), 100);
        rt.set_gcm_rsx_checkpoint(true);
        rt.gcm_state.label_addr = 0x50000;

        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args0: [u64; 9] = [0x10000, 0, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELLGCM_GET_LABEL_ADDRESS, &args0);
        let ret0 = rt.registry_mut().drain_syscall_return(unit_id);
        assert_eq!(ret0, Some(0x50000));

        let args5: [u64; 9] = [0x10000, 5, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELLGCM_GET_LABEL_ADDRESS, &args5);
        let ret5 = rt.registry_mut().drain_syscall_return(unit_id);
        assert_eq!(ret5, Some(0x50000 + 5 * 0x10));
    }

    #[test]
    fn sys_ppu_thread_get_id_writes_thread_id_and_returns_zero() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x100000), Budget::new(1), 100);
        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0x1000, 0, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_SYS_PPU_THREAD_GET_ID, &args);

        let mem = rt.memory().as_bytes();
        let tid = u64::from_be_bytes([
            mem[0x1000],
            mem[0x1001],
            mem[0x1002],
            mem[0x1003],
            mem[0x1004],
            mem[0x1005],
            mem[0x1006],
            mem[0x1007],
        ]);
        assert_eq!(tid, 0x0100_0000);
        assert_eq!(rt.registry_mut().drain_syscall_return(unit_id), Some(0));
    }

    #[test]
    fn sys_ppu_thread_get_id_returns_primary_when_unit_seeded_in_table() {
        // Once the primary thread is recorded in the
        // PpuThreadTable, get_id must route through the table
        // lookup rather than the 0x0100_0000 fallback. The two
        // paths happen to produce the same value for the primary
        // thread, but this test nails the routing so that child
        // threads created via sys_ppu_thread_create pick up
        // their minted ids through the same path.
        use crate::runtime::Runtime;
        use cellgov_lv2::PpuThreadAttrs;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x100000), Budget::new(1), 100);
        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let attrs = PpuThreadAttrs {
            entry: 0x1_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        };
        rt.lv2_host_mut().seed_primary_ppu_thread(unit_id, attrs);

        let args: [u64; 9] = [0x2000, 0, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_SYS_PPU_THREAD_GET_ID, &args);

        let mem = rt.memory().as_bytes();
        let tid = u64::from_be_bytes([
            mem[0x2000],
            mem[0x2001],
            mem[0x2002],
            mem[0x2003],
            mem[0x2004],
            mem[0x2005],
            mem[0x2006],
            mem[0x2007],
        ]);
        assert_eq!(tid, 0x0100_0000);
        assert_eq!(rt.registry_mut().drain_syscall_return(unit_id), Some(0));
    }

    #[test]
    fn sys_ppu_thread_join_on_finished_target_writes_exit_value() {
        use crate::runtime::Runtime;
        use cellgov_lv2::{Lv2Request, PpuThreadAttrs};
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let primary = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let attrs = || PpuThreadAttrs {
            entry: 0x1_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        };
        rt.lv2_host_mut().seed_primary_ppu_thread(primary, attrs());
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let child_id = rt
            .lv2_host_mut()
            .ppu_threads_mut()
            .create(cellgov_event::UnitId::new(1), attrs())
            .expect("child create");
        rt.lv2_host_mut()
            .ppu_threads_mut()
            .mark_finished(child_id, 0xCAFE_F00D);

        rt.dispatch_lv2_request(
            Lv2Request::PpuThreadJoin {
                target: child_id.raw(),
                status_out_ptr: 0x5000,
            },
            primary,
        );

        assert_eq!(rt.registry_mut().drain_syscall_return(primary), Some(0));
        let mem = rt.memory().as_bytes();
        let status = u64::from_be_bytes([
            mem[0x5000],
            mem[0x5001],
            mem[0x5002],
            mem[0x5003],
            mem[0x5004],
            mem[0x5005],
            mem[0x5006],
            mem[0x5007],
        ]);
        assert_eq!(status, 0xCAFE_F00D);
    }

    #[test]
    fn sys_ppu_thread_join_blocks_then_wakes_with_exit_value() {
        use crate::runtime::Runtime;
        use cellgov_lv2::{Lv2Request, PpuThreadAttrs};
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let primary = cellgov_event::UnitId::new(0);
        let child = cellgov_event::UnitId::new(1);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let attrs = || PpuThreadAttrs {
            entry: 0x1_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        };
        rt.lv2_host_mut().seed_primary_ppu_thread(primary, attrs());
        let child_id = rt
            .lv2_host_mut()
            .ppu_threads_mut()
            .create(child, attrs())
            .expect("child create");

        rt.dispatch_lv2_request(
            Lv2Request::PpuThreadJoin {
                target: child_id.raw(),
                status_out_ptr: 0x5000,
            },
            primary,
        );
        assert_eq!(
            rt.registry().effective_status(primary),
            Some(cellgov_exec::UnitStatus::Blocked),
        );

        rt.dispatch_lv2_request(
            Lv2Request::PpuThreadExit {
                exit_value: 0xABCD_1234,
            },
            child,
        );

        assert_eq!(
            rt.registry().effective_status(primary),
            Some(cellgov_exec::UnitStatus::Runnable),
        );
        assert_eq!(rt.registry_mut().drain_syscall_return(primary), Some(0));
        let mem = rt.memory().as_bytes();
        let status = u64::from_be_bytes([
            mem[0x5000],
            mem[0x5001],
            mem[0x5002],
            mem[0x5003],
            mem[0x5004],
            mem[0x5005],
            mem[0x5006],
            mem[0x5007],
        ]);
        assert_eq!(status, 0xABCD_1234);
    }

    #[test]
    fn sys_ppu_thread_create_registers_child_unit_and_mints_thread_id() {
        use crate::runtime::Runtime;
        use cellgov_lv2::{Lv2Request, PpuThreadAttrs};
        use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        let primary = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let primary_attrs = PpuThreadAttrs {
            entry: 0x1_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        };
        rt.lv2_host_mut()
            .seed_primary_ppu_thread(primary, primary_attrs);

        use std::cell::Cell;
        use std::rc::Rc;
        let calls = Rc::new(Cell::new(0u32));
        let calls_clone = calls.clone();
        rt.set_ppu_factory(move |id, _init| {
            calls_clone.set(calls_clone.get() + 1);
            Box::new(cellgov_exec::FakeIsaUnit::new(
                id,
                vec![cellgov_exec::FakeOp::End],
            ))
        });

        let mut opd = [0u8; 16];
        opd[0..8].copy_from_slice(&0x4_0000u64.to_be_bytes());
        opd[8..16].copy_from_slice(&0x5_0000u64.to_be_bytes());
        rt.memory_mut()
            .apply_commit(ByteRange::new(GuestAddr::new(0x2_0000), 16).unwrap(), &opd)
            .unwrap();

        rt.dispatch_lv2_request(
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x3_0000,
                entry_opd: 0x2_0000,
                arg: 0xCAFE_BABE,
                priority: 1500,
                stacksize: 0x10_000,
                flags: 0,
            },
            primary,
        );

        assert_eq!(calls.get(), 1);
        let ids: Vec<_> = rt.registry().ids().collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(rt.registry_mut().drain_syscall_return(primary), Some(0));
        let child_unit_id = ids[1];
        let child_thread = rt
            .lv2_host()
            .ppu_thread_for_unit(child_unit_id)
            .expect("child in thread table");
        assert!(child_thread.id.raw() > cellgov_lv2::PpuThreadId::PRIMARY.raw());
        let mem = rt.memory().as_bytes();
        let written = u64::from_be_bytes([
            mem[0x3_0000],
            mem[0x3_0001],
            mem[0x3_0002],
            mem[0x3_0003],
            mem[0x3_0004],
            mem[0x3_0005],
            mem[0x3_0006],
            mem[0x3_0007],
        ]);
        assert_eq!(written, child_thread.id.raw());
    }

    #[test]
    fn sys_ppu_thread_get_id_returns_child_id_when_child_registered() {
        use crate::runtime::Runtime;
        use cellgov_lv2::PpuThreadAttrs;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x100000), Budget::new(1), 100);
        let primary = cellgov_event::UnitId::new(0);
        let child = cellgov_event::UnitId::new(1);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });
        let attrs = || PpuThreadAttrs {
            entry: 0x1_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0,
        };
        rt.lv2_host_mut().seed_primary_ppu_thread(primary, attrs());
        let child_id = rt
            .lv2_host_mut()
            .ppu_threads_mut()
            .create(child, attrs())
            .expect("child create");
        assert_ne!(child_id.raw(), 0x0100_0000);

        let args: [u64; 9] = [0x3000, 0, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(child, NID_SYS_PPU_THREAD_GET_ID, &args);

        let mem = rt.memory().as_bytes();
        let tid = u64::from_be_bytes([
            mem[0x3000],
            mem[0x3001],
            mem[0x3002],
            mem[0x3003],
            mem[0x3004],
            mem[0x3005],
            mem[0x3006],
            mem[0x3007],
        ]);
        assert_eq!(tid, child_id.raw());
    }

    #[test]
    fn sys_time_get_system_time_returns_nonzero_monotonic() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x100000), Budget::new(1), 100);
        let unit_id = cellgov_event::UnitId::new(0);
        rt.registry_mut().register_with(|id| {
            cellgov_exec::FakeIsaUnit::new(id, vec![cellgov_exec::FakeOp::End])
        });

        let args: [u64; 9] = [0; 9];
        rt.dispatch_hle(unit_id, NID_SYS_TIME_GET_SYSTEM_TIME, &args);

        let ret = rt.registry_mut().drain_syscall_return(unit_id);
        assert_eq!(ret, Some(1_000_000));
    }
}
