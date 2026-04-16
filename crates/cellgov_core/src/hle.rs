//! HLE (High-Level Emulation) dispatch for PS3 system imports.
//!
//! Dispatches NID-based HLE calls from the runtime. Unknown NIDs
//! return 0. The dispatch table handles NIDs with real behavior:
//! TLS initialization, malloc/free, memset, process exit, lwmutex
//! create, heap create/delete, heap malloc/memalign/free, and
//! cellGcmSys functions needed for RSX initialization.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

use crate::runtime::Runtime;

// NID constants for functions that need non-trivial behavior.
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
const NID_CELLGCM_GET_TILED_PITCH_SIZE: u32 = 0x055bd74d;
const NID_CELLGCM_INIT_BODY: u32 = 0x15bae46b;
const NID_CELLGCM_GET_CONFIGURATION: u32 = 0xe315a0b2;
const NID_CELLGCM_GET_CONTROL_REGISTER: u32 = 0xa547adde;
const NID_CELLGCM_GET_LABEL_ADDRESS: u32 = 0xf80196c1;

const TILED_PITCHES: &[u32] = &[
    0x000, 0x200, 0x300, 0x400, 0x500, 0x600, 0x700, 0x800, 0xA00, 0xC00, 0xD00, 0xE00, 0x1000,
    0x1400, 0x1800, 0x1A00, 0x1C00, 0x2000, 0x2800, 0x3000, 0x3400, 0x3800, 0x4000, 0x5000, 0x6000,
    0x6800, 0x7000, 0x8000, 0xA000, 0xC000, 0xD000, 0xE000, 0x10000,
];

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
            NID_SYS_LWMUTEX_CREATE => self.hle_lwmutex_create(source, args),
            NID_SYS_HEAP_CREATE_HEAP => self.hle_heap_create_heap(source),
            NID_SYS_HEAP_DELETE_HEAP => {
                self.registry.set_syscall_return(source, 0);
            }
            NID_SYS_HEAP_MALLOC => self.hle_heap_malloc(source, args),
            NID_SYS_HEAP_MEMALIGN => self.hle_heap_memalign(source, args),
            NID_SYS_HEAP_FREE => {
                self.registry.set_syscall_return(source, 0);
            }
            NID_CELLGCM_GET_TILED_PITCH_SIZE if self.gcm_rsx_checkpoint => {
                self.hle_gcm_get_tiled_pitch_size(source, args);
            }
            NID_CELLGCM_INIT_BODY if self.gcm_rsx_checkpoint => {
                self.hle_gcm_init_body(source, args);
            }
            NID_CELLGCM_GET_CONFIGURATION if self.gcm_rsx_checkpoint => {
                self.hle_gcm_get_configuration(source, args);
            }
            NID_CELLGCM_GET_CONTROL_REGISTER if self.gcm_rsx_checkpoint => {
                self.hle_gcm_get_control_register(source);
            }
            NID_CELLGCM_GET_LABEL_ADDRESS if self.gcm_rsx_checkpoint => {
                self.hle_gcm_get_label_address(source, args);
            }
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

    /// Initialize a `sys_lwmutex_t` (24-byte struct) in guest memory.
    ///
    /// PS3's sysPrxForUser sys_lwmutex_create writes five fields into the
    /// caller-provided struct before returning success. If we skip this,
    /// any inline fast-path CAS that expects `lock_var.owner == lwmutex_free
    /// (0xFFFFFFFF)` sees zero instead and falls into error handling.
    ///
    /// Mirrors RPCS3 `Emu/Cell/Modules/sys_lwmutex_.cpp`:
    ///
    /// ```text
    /// lwmutex->lock_var.store({ lwmutex_free, 0 });
    /// lwmutex->attribute = attr->recursive | attr->protocol;
    /// lwmutex->recursive_count = 0;
    /// lwmutex->sleep_queue = *out_id;
    /// ```
    fn hle_lwmutex_create(&mut self, source: UnitId, args: &[u64; 9]) {
        let mutex_ptr = args[1] as u32;
        let attr_ptr = args[2] as u32;

        // Read attribute struct (at minimum: protocol @ 0, recursive @ 4).
        let mem = self.memory.as_bytes();
        let attr_offset = attr_ptr as usize;
        let (protocol, recursive) = if attr_offset + 8 <= mem.len() {
            let p = u32::from_be_bytes([
                mem[attr_offset],
                mem[attr_offset + 1],
                mem[attr_offset + 2],
                mem[attr_offset + 3],
            ]);
            let r = u32::from_be_bytes([
                mem[attr_offset + 4],
                mem[attr_offset + 5],
                mem[attr_offset + 6],
                mem[attr_offset + 7],
            ]);
            (p, r)
        } else {
            // SYS_SYNC_PRIORITY | SYS_SYNC_NOT_RECURSIVE as a safe default.
            (0x2, 0x20)
        };

        let sleep_queue = self.hle_next_id;
        self.hle_next_id = self.hle_next_id.wrapping_add(1);

        // Build the 24-byte lwmutex struct (big-endian u32 fields):
        //   +0  owner           = 0xFFFFFFFF (lwmutex_free)
        //   +4  waiter          = 0
        //   +8  attribute       = recursive | protocol
        //   +12 recursive_count = 0
        //   +16 sleep_queue     = allocated id
        //   +20 pad             = 0
        let mut buf = [0u8; 24];
        buf[0..4].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        buf[8..12].copy_from_slice(&(recursive | protocol).to_be_bytes());
        buf[16..20].copy_from_slice(&sleep_queue.to_be_bytes());

        let target = mutex_ptr as usize;
        if target + 24 <= self.memory.as_bytes().len() {
            if let Some(range) =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(target as u64), 24)
            {
                let _ = self.memory.apply_commit(range, &buf);
            }
        }
        self.registry.set_syscall_return(source, 0);
    }

    /// Allocate a fresh heap id. Matches RPCS3 `_sys_heap_create_heap`:
    /// returns an opaque handle the caller stores and passes back to
    /// `_sys_heap_malloc` / `_sys_heap_free`. Real backing memory is
    /// served through `hle_heap_malloc` from the HLE bump arena, not
    /// from a per-heap pool.
    fn hle_heap_create_heap(&mut self, source: UnitId) {
        let id = self.hle_next_id;
        self.hle_next_id = self.hle_next_id.wrapping_add(1);
        self.registry.set_syscall_return(source, id as u64);
    }

    /// `_sys_heap_malloc(heap, size)`. The heap argument is ignored;
    /// allocation bumps the shared HLE arena like `_sys_malloc` does.
    fn hle_heap_malloc(&mut self, source: UnitId, args: &[u64; 9]) {
        let size = args[2] as u32;
        let aligned_ptr = (self.hle_heap_ptr + 15) & !15;
        let new_ptr = aligned_ptr + size;
        if (new_ptr as usize) <= self.memory.as_bytes().len() {
            self.hle_heap_ptr = new_ptr;
            self.registry.set_syscall_return(source, aligned_ptr as u64);
        } else {
            self.registry.set_syscall_return(source, 0);
        }
    }

    /// `_sys_heap_memalign(heap, align, size)`. Rounds the bump pointer
    /// up to `max(align, 16)`, then allocates from the shared HLE arena.
    fn hle_heap_memalign(&mut self, source: UnitId, args: &[u64; 9]) {
        let align = (args[2] as u32).max(16);
        let size = args[3] as u32;
        let mask = align - 1;
        let aligned_ptr = (self.hle_heap_ptr + mask) & !mask;
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

    fn hle_gcm_get_tiled_pitch_size(&mut self, source: UnitId, args: &[u64; 9]) {
        let size = args[1] as u32;
        let result = tiled_pitch_lookup(size);
        self.registry.set_syscall_return(source, result as u64);
    }

    fn hle_gcm_init_body(&mut self, source: UnitId, args: &[u64; 9]) {
        let context_pp = args[1] as u32;
        let _cmd_size = args[2] as u32;
        let io_size = args[3] as u32;
        let io_address = args[4] as u32;

        self.gcm_io_address = io_address;
        self.gcm_io_size = io_size;
        self.gcm_local_size = 0x0f90_0000;

        // Allocate callback stub: OPD (8 bytes) + body (8 bytes).
        // Body: li r3, 0 (0x38600000); blr (0x4E800020)
        let cb_base = (self.hle_heap_ptr + 15) & !15;
        let cb_opd = cb_base;
        let cb_body = cb_base + 8;
        self.hle_heap_ptr = cb_base + 16;
        let mut cb_buf = [0u8; 16];
        cb_buf[0..4].copy_from_slice(&cb_body.to_be_bytes());
        // OPD TOC = 0 (bytes 4..8 already zero)
        cb_buf[8..12].copy_from_slice(&0x3860_0000u32.to_be_bytes()); // li r3, 0
        cb_buf[12..16].copy_from_slice(&0x4E80_0020u32.to_be_bytes()); // blr
        if let Some(range) =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(cb_base as u64), 16)
        {
            let _ = self.memory.apply_commit(range, &cb_buf);
        }

        // Allocate CellGcmContextData (16 bytes).
        let ctx_addr = (self.hle_heap_ptr + 15) & !15;
        self.hle_heap_ptr = ctx_addr + 16;
        self.gcm_context_addr = ctx_addr;

        let begin = io_address + 0x1000;
        let end = io_address + io_size - 4;
        let mut ctx_buf = [0u8; 16];
        ctx_buf[0..4].copy_from_slice(&begin.to_be_bytes());
        ctx_buf[4..8].copy_from_slice(&end.to_be_bytes());
        ctx_buf[8..12].copy_from_slice(&begin.to_be_bytes()); // current = begin
        ctx_buf[12..16].copy_from_slice(&cb_opd.to_be_bytes()); // callback OPD
        if let Some(range) =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(ctx_addr as u64), 16)
        {
            let _ = self.memory.apply_commit(range, &ctx_buf);
        }

        // Write context address to *context (the caller's output).
        if let Some(range) =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(context_pp as u64), 4)
        {
            let _ = self.memory.apply_commit(range, &ctx_addr.to_be_bytes());
        }

        // Allocate CellGcmControl (12 bytes: put, get, ref).
        // When gcm_rsx_checkpoint is set, place the control register
        // in the RSX reserved region so the game's first put-pointer
        // write triggers a ReservedWrite -> FirstRsxWrite checkpoint.
        let ctrl_addr = if self.gcm_rsx_checkpoint {
            0xC000_0040u32
        } else {
            let a = (self.hle_heap_ptr + 15) & !15;
            self.hle_heap_ptr = a + 12;
            a
        };
        self.gcm_control_addr = ctrl_addr;

        // Allocate RSX label area (256 labels * 16 bytes = 4096 bytes).
        // Pre-fill with 0xFFFFFFFF so any label-poll that checks for a
        // non-zero completion value sees "done" immediately. Real RSX
        // would write specific values; we simulate universal completion.
        let label_addr = (self.hle_heap_ptr + 15) & !15;
        self.hle_heap_ptr = label_addr + 4096;
        self.gcm_label_addr = label_addr;
        let label_fill = vec![0xFFu8; 4096];
        if let Some(range) =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(label_addr as u64), 4096)
        {
            let _ = self.memory.apply_commit(range, &label_fill);
        }

        self.registry.set_syscall_return(source, 0);
    }

    fn hle_gcm_get_configuration(&mut self, source: UnitId, args: &[u64; 9]) {
        let config_ptr = args[1] as u32;
        let mut buf = [0u8; 24];
        // localAddress: RSX local mem base (0xC0000000 is the standard PS3 base)
        buf[0..4].copy_from_slice(&0xC000_0000u32.to_be_bytes());
        buf[4..8].copy_from_slice(&self.gcm_io_address.to_be_bytes());
        buf[8..12].copy_from_slice(&self.gcm_local_size.to_be_bytes());
        buf[12..16].copy_from_slice(&self.gcm_io_size.to_be_bytes());
        buf[16..20].copy_from_slice(&650_000_000u32.to_be_bytes());
        buf[20..24].copy_from_slice(&500_000_000u32.to_be_bytes());
        if let Some(range) =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(config_ptr as u64), 24)
        {
            let _ = self.memory.apply_commit(range, &buf);
        }
        self.registry.set_syscall_return(source, 0);
    }

    fn hle_gcm_get_control_register(&mut self, source: UnitId) {
        self.registry
            .set_syscall_return(source, self.gcm_control_addr as u64);
    }

    fn hle_gcm_get_label_address(&mut self, source: UnitId, args: &[u64; 9]) {
        let index = args[1] as u32;
        let addr = self.gcm_label_addr + 0x10 * index;
        self.registry.set_syscall_return(source, addr as u64);
    }
}

fn tiled_pitch_lookup(size: u32) -> u32 {
    TILED_PITCHES
        .windows(2)
        .find(|w| w[0] < size && size <= w[1])
        .map(|w| w[1])
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

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

        // Context pointer should have been written at 0x10000.
        let mem = rt.memory().as_bytes();
        let ctx_ptr = u32::from_be_bytes([mem[0x10000], mem[0x10001], mem[0x10002], mem[0x10003]]);
        assert_ne!(ctx_ptr, 0, "context pointer should be non-zero");

        // Read the CellGcmContextData at ctx_ptr.
        let a = ctx_ptr as usize;
        let begin = u32::from_be_bytes([mem[a], mem[a + 1], mem[a + 2], mem[a + 3]]);
        let end = u32::from_be_bytes([mem[a + 4], mem[a + 5], mem[a + 6], mem[a + 7]]);
        let callback = u32::from_be_bytes([mem[a + 12], mem[a + 13], mem[a + 14], mem[a + 15]]);
        assert_eq!(begin, 0x20000 + 0x1000, "begin = ioAddress + 0x1000");
        assert!(end > begin, "end > begin");
        assert_ne!(callback, 0, "callback OPD should be non-zero");

        // Control register should be in RSX space.
        assert_eq!(rt.gcm_control_addr, 0xC000_0040);
    }

    #[test]
    fn gcm_get_configuration_writes_config() {
        use crate::runtime::Runtime;
        use cellgov_mem::GuestMemory;
        use cellgov_time::Budget;

        let mut rt = Runtime::new(GuestMemory::new(0x200000), Budget::new(1), 100);
        rt.set_hle_heap_base(0x100000);
        rt.set_gcm_rsx_checkpoint(true);
        rt.gcm_io_address = 0x20000;
        rt.gcm_io_size = 0x80000;
        rt.gcm_local_size = 0x0f90_0000;

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
        rt.gcm_label_addr = 0x50000;

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
}
