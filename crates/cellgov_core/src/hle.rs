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
                let id: u64 = 0x0100_0000;
                ctx!().write_guest(ptr as u64, &id.to_be_bytes());
                ctx!().set_return(0);
            }
            NID_SYS_TIME_GET_SYSTEM_TIME => {
                ctx!().set_return(1_000_000);
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
