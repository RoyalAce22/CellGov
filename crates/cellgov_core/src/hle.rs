//! HLE (High-Level Emulation) dispatch for PS3 sysPrxForUser imports.
//!
//! Dispatches NID-based HLE calls from the runtime. Most functions
//! are noop-safe (return 0). A few need real behavior: TLS
//! initialization, malloc, memset, process exit.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

use crate::runtime::Runtime;

// NID constants for functions that need non-trivial behavior.
const NID_SYS_INITIALIZE_TLS: u32 = 0x744680a2;
const NID_SYS_PROCESS_EXIT: u32 = 0xe6f2c1e7;
const NID_SYS_MALLOC: u32 = 0xebe5f72f;
const NID_SYS_FREE: u32 = 0xfc52a7a9;
const NID_SYS_MEMSET: u32 = 0x1573dc3f;

impl Runtime {
    /// Dispatch an HLE import call by NID. Most functions are
    /// noop-safe (return 0). A few need real behavior.
    pub(crate) fn dispatch_hle(&mut self, source: UnitId, nid: u32, args: &[u64; 9]) {
        match nid {
            NID_SYS_INITIALIZE_TLS => self.hle_initialize_tls(source, args),
            NID_SYS_PROCESS_EXIT => {
                self.registry
                    .set_status_override(source, UnitStatus::Finished);
                self.registry.set_syscall_return(source, 0);
            }
            NID_SYS_MALLOC => self.hle_malloc(source, args),
            NID_SYS_FREE => {
                self.registry.set_syscall_return(source, 0);
            }
            NID_SYS_MEMSET => self.hle_memset(source, args),
            _ => {
                self.registry.set_syscall_return(source, 0);
            }
        }
    }

    fn hle_initialize_tls(&mut self, source: UnitId, args: &[u64; 9]) {
        let tls_seg_addr = args[2] as u32;
        let tls_seg_size = args[3] as u32;
        let tls_mem_size = args[4] as u32;

        let slot_size = tls_mem_size + 0x30;
        let tls_base: u32 = 0x10400000;

        // Copy TLS initialization image.
        let src = tls_seg_addr as usize;
        let dst = (tls_base + 0x30) as usize;
        let copy_len = tls_seg_size as usize;
        let mem = self.memory.as_bytes();
        if src + copy_len <= mem.len() && dst + copy_len <= mem.len() {
            let init_data: Vec<u8> = mem[src..src + copy_len].to_vec();
            if let Some(range) = cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new(dst as u64),
                copy_len as u64,
            ) {
                let _ = self.memory.apply_commit(range, &init_data);
            }
        }

        // Zero BSS portion.
        let bss_start = dst + copy_len;
        let bss_len = (slot_size - 0x30 - tls_seg_size) as usize;
        if bss_len > 0 && bss_start + bss_len <= self.memory.as_bytes().len() {
            let zeros = vec![0u8; bss_len];
            if let Some(range) = cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new(bss_start as u64),
                bss_len as u64,
            ) {
                let _ = self.memory.apply_commit(range, &zeros);
            }
        }

        // Set r13 = tls_base + 0x30 + 0x7000 (PPC64 TLS bias).
        let r13_val = (tls_base + 0x30 + 0x7000) as u64;
        self.registry.push_register_write(source, 13, r13_val);
        self.registry.set_syscall_return(source, 0);

        let _ = slot_size;
    }

    fn hle_malloc(&mut self, source: UnitId, args: &[u64; 9]) {
        let size = args[1] as u32;
        let aligned_ptr = (self.hle_heap_ptr + 15) & !15;
        let new_ptr = aligned_ptr + size;
        if (new_ptr as usize) <= self.memory.as_bytes().len() {
            self.hle_heap_ptr = new_ptr;
            self.registry.set_syscall_return(source, aligned_ptr as u64);
        } else {
            self.registry.set_syscall_return(source, 0);
        }
    }

    fn hle_memset(&mut self, source: UnitId, args: &[u64; 9]) {
        let ptr = args[1] as usize;
        let val = args[2] as u8;
        let size = args[3] as usize;
        if ptr + size <= self.memory.as_bytes().len() {
            let data = vec![val; size];
            if let Some(range) =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(ptr as u64), size as u64)
            {
                let _ = self.memory.apply_commit(range, &data);
            }
        }
        self.registry.set_syscall_return(source, args[1]);
    }
}
