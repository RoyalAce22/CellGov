//! HLE (High-Level Emulation) dispatch for PS3 sysPrxForUser imports.
//!
//! Dispatches NID-based HLE calls from the runtime. Unknown NIDs
//! return 0. The dispatch table handles 11 NIDs with real behavior:
//! TLS initialization, malloc/free, memset, process exit, lwmutex
//! create, heap create/delete, heap malloc/memalign/free.

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
}
