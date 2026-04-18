//! HLE (High-Level Emulation) dispatch for PS3 system imports.
//!
//! Thin dispatch layer: maps NID to the module handler that
//! implements it. Module implementations live in separate files
//! (hle_sys, hle_gcm) and operate through the [`HleContext`] trait,
//! never touching Runtime directly.

use cellgov_event::UnitId;

use crate::hle_context::{HleContext, RuntimeHleAdapter};
use crate::hle_gcm;
use crate::hle_sys;
use crate::runtime::Runtime;

const NID_SYS_INITIALIZE_TLS: u32 = 0x744680a2;
const NID_SYS_PROCESS_EXIT: u32 = 0xe6f2c1e7;
const NID_SYS_MALLOC: u32 = 0xbdb18f83;
const NID_SYS_FREE: u32 = 0xf7f7fb20;
const NID_SYS_MEMSET: u32 = 0x68b9b011;
const NID_SYS_LWMUTEX_CREATE: u32 = 0x2f85c0ef;
const NID_SYS_HEAP_CREATE_HEAP: u32 = 0xb2fcf2c8;
const NID_SYS_HEAP_DELETE_HEAP: u32 = 0xaede4b03;
const NID_SYS_HEAP_MALLOC: u32 = 0x35168520;
const NID_SYS_HEAP_MEMALIGN: u32 = 0x44265c08;
const NID_SYS_HEAP_FREE: u32 = 0x8a561d92;
const NID_SYS_PPU_THREAD_GET_ID: u32 = 0x350d454e;
const NID_SYS_THREAD_CREATE_EX: u32 = 0x24a1ea07;
const NID_SYS_THREAD_EXIT: u32 = 0xaff080a4;
const NID_SYS_TIME_GET_SYSTEM_TIME: u32 = 0x8461e528;
const NID_CELLGCM_GET_TILED_PITCH_SIZE: u32 = 0x055bd74d;
const NID_CELLGCM_INIT_BODY: u32 = 0x15bae46b;
const NID_CELLGCM_GET_CONFIGURATION: u32 = 0xe315a0b2;
const NID_CELLGCM_GET_CONTROL_REGISTER: u32 = 0xa547adde;
const NID_CELLGCM_GET_LABEL_ADDRESS: u32 = 0xf80196c1;

impl Runtime {
    /// Dispatch an HLE import call by NID.
    pub(crate) fn dispatch_hle(&mut self, source: UnitId, nid: u32, args: &[u64; 9]) {
        let gcm_enabled = self.gcm_state.rsx_checkpoint;

        macro_rules! ctx {
            () => {
                RuntimeHleAdapter {
                    memory: &mut self.memory,
                    registry: &mut self.registry,
                    heap_ptr: &mut self.hle_heap_ptr,
                    next_id: &mut self.hle_next_id,
                    source,
                }
            };
        }

        match nid {
            NID_SYS_INITIALIZE_TLS => hle_sys::initialize_tls(&mut ctx!(), args),
            NID_SYS_PROCESS_EXIT => hle_sys::process_exit(&mut ctx!()),
            NID_SYS_MALLOC => hle_sys::malloc(&mut ctx!(), args),
            NID_SYS_FREE | NID_SYS_HEAP_DELETE_HEAP | NID_SYS_HEAP_FREE => {
                ctx!().set_return(0);
            }
            NID_SYS_MEMSET => hle_sys::memset(&mut ctx!(), args),
            NID_SYS_LWMUTEX_CREATE => hle_sys::lwmutex_create(&mut ctx!(), args),
            NID_SYS_HEAP_CREATE_HEAP => hle_sys::heap_create_heap(&mut ctx!()),
            NID_SYS_HEAP_MALLOC => hle_sys::heap_malloc(&mut ctx!(), args),
            NID_SYS_HEAP_MEMALIGN => hle_sys::heap_memalign(&mut ctx!(), args),
            NID_SYS_PPU_THREAD_GET_ID => {
                let ptr = args[0] as u32;
                // Look up the calling thread's guest-facing id in
                // the PpuThreadTable. Returns PRIMARY for the
                // seeded primary thread and the minted id for
                // each child spawned via sys_ppu_thread_create.
                // When the table has not been seeded (boot paths
                // that never call sys_ppu_thread_create), fall
                // back to 0x0100_0000 -- the canonical PSL1GHT
                // primary thread id.
                let id: u64 = self
                    .lv2_host()
                    .ppu_thread_id_for_unit(source)
                    .map(|tid| tid.raw())
                    .unwrap_or(0x0100_0000);
                ctx!().write_guest(ptr as u64, &id.to_be_bytes());
                ctx!().set_return(0);
            }
            NID_SYS_TIME_GET_SYSTEM_TIME => {
                ctx!().set_return(1_000_000);
            }
            NID_SYS_THREAD_CREATE_EX => {
                // PSL1GHT's sysThreadCreateEx (the HLE wrapper
                // sysThreadCreate resolves to) maps directly onto
                // sys_ppu_thread_create. Arg layout matches: r3
                // through r8 carry id_ptr, opd_ptr, arg,
                // priority, stacksize, flags.
                self.dispatch_lv2_request(
                    cellgov_lv2::Lv2Request::PpuThreadCreate {
                        id_ptr: args[0] as u32,
                        entry_opd: args[1] as u32,
                        arg: args[2],
                        priority: args[3] as u32,
                        stacksize: args[4],
                        flags: args[5],
                    },
                    source,
                );
            }
            NID_SYS_THREAD_EXIT => {
                self.dispatch_lv2_request(
                    cellgov_lv2::Lv2Request::PpuThreadExit {
                        exit_value: args[0],
                    },
                    source,
                );
            }
            NID_CELLGCM_GET_TILED_PITCH_SIZE if gcm_enabled => {
                hle_gcm::get_tiled_pitch_size(&mut ctx!(), args);
            }
            NID_CELLGCM_INIT_BODY if gcm_enabled => {
                hle_gcm::init_body(&mut ctx!(), args, &mut self.gcm_state);
            }
            NID_CELLGCM_GET_CONFIGURATION if gcm_enabled => {
                hle_gcm::get_configuration(&mut ctx!(), args, &self.gcm_state);
            }
            NID_CELLGCM_GET_CONTROL_REGISTER if gcm_enabled => {
                hle_gcm::get_control_register(&mut ctx!(), &self.gcm_state);
            }
            NID_CELLGCM_GET_LABEL_ADDRESS if gcm_enabled => {
                hle_gcm::get_label_address(&mut ctx!(), args, &self.gcm_state);
            }
            _ => {
                ctx!().set_return(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hle_gcm::tiled_pitch_lookup;

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
        use crate::hle_gcm::TILED_PITCHES;
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

        // arg[0] is the guest pointer to receive the thread id (u64 BE).
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
        // Seed the primary PPU thread mapping unit 0 -> PRIMARY.
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
        // End-to-end join: create a child, mark it finished with
        // a known exit value, dispatch join from the parent, and
        // verify the parent reads the exit value back.
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
        // Register a dummy child and mark it finished with a
        // known exit value.
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

        // CELL_OK to the caller.
        assert_eq!(rt.registry_mut().drain_syscall_return(primary), Some(0));
        // Exit value landed at status_out_ptr as big-endian u64.
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
        // Running target: parent joins and blocks. When the
        // child subsequently calls sys_ppu_thread_exit, the
        // parent wakes with the exit value written to its out
        // pointer.
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

        // Parent joins while child is still Runnable.
        rt.dispatch_lv2_request(
            Lv2Request::PpuThreadJoin {
                target: child_id.raw(),
                status_out_ptr: 0x5000,
            },
            primary,
        );
        // Parent is now Blocked.
        assert_eq!(
            rt.registry().effective_status(primary),
            Some(cellgov_exec::UnitStatus::Blocked),
        );

        // Child exits.
        rt.dispatch_lv2_request(
            Lv2Request::PpuThreadExit {
                exit_value: 0xABCD_1234,
            },
            child,
        );

        // Parent is now Runnable with CELL_OK in r3 and exit
        // value at status_out_ptr.
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
        // End-to-end check: dispatch a PpuThreadCreate request
        // through the runtime's full path, verify the runtime
        // (a) registered a second unit, (b) minted a new
        // PpuThreadId > PRIMARY, (c) wrote the id back to the
        // caller's pointer.
        use crate::runtime::Runtime;
        use cellgov_lv2::{Lv2Request, PpuThreadAttrs};
        use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x10_0000), Budget::new(1), 100);
        // Primary unit + PPU thread table seeding so the caller
        // has a known id.
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

        // Stub PPU factory: returns a FakeIsaUnit in place of a
        // real PpuExecutionUnit. Records whether the factory was
        // called via a shared Rc<Cell>.
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

        // Seed a valid OPD at 0x2_0000: code=0x4_0000, toc=0x5_0000.
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

        // Factory invoked exactly once.
        assert_eq!(calls.get(), 1);
        // Registry now has two units; the child sits above the
        // primary in id order.
        let ids: Vec<_> = rt.registry().ids().collect();
        assert_eq!(ids.len(), 2);
        // Caller received CELL_OK.
        assert_eq!(rt.registry_mut().drain_syscall_return(primary), Some(0));
        // Thread table has the child with a minted id > PRIMARY.
        let child_unit_id = ids[1];
        let child_thread = rt
            .lv2_host()
            .ppu_thread_for_unit(child_unit_id)
            .expect("child in thread table");
        assert!(child_thread.id.raw() > cellgov_lv2::PpuThreadId::PRIMARY.raw());
        // The minted id landed at the caller's output pointer,
        // big-endian u64.
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
        // A second PPU unit created via the table must return its
        // own minted id, not the primary's id. This is the
        // capability the two-thread microtest exercises
        // end-to-end through sys_ppu_thread_create.
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

        // Returned in r3; the deterministic oracle picks a fixed
        // nonzero value so guest code that checks time > 0 does not
        // wedge.
        let ret = rt.registry_mut().drain_syscall_return(unit_id);
        assert_eq!(ret, Some(1_000_000));
    }
}
