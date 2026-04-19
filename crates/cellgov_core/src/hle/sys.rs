//! sysPrxForUser HLE implementations.
//!
//! Covers the userspace PRX surface exposed as sysPrxForUser (and
//! the RPCS3 sub-files `sys_lwmutex_.cpp`, `sys_ppu_thread_.cpp`,
//! `sys_heap.cpp`, `sys_libc.cpp`, `sys_prx_.cpp` that register
//! into the same module). Kernel-side syscalls (`sc` trap
//! handlers) live in `cellgov_lv2`, not here.
//!
//! The module exports a single [`dispatch`] entry that the HLE
//! router in [`crate::hle`] chains via `.or_else()`. Each module
//! file follows the same shape so adding a new library is an
//! additive change to the router and a new file, never a shared
//! match statement.
//!
//! Handler NID constants are module-local. The workspace name
//! database (`cellgov_ppu::nid_db`) is the global reverse lookup
//! for diagnostics and tracing, distinct from dispatch.

use cellgov_event::UnitId;

use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

pub(crate) const NID_SYS_INITIALIZE_TLS: u32 = 0x744680a2;
pub(crate) const NID_SYS_PROCESS_EXIT: u32 = 0xe6f2c1e7;
pub(crate) const NID_SYS_MALLOC: u32 = 0xbdb18f83;
pub(crate) const NID_SYS_FREE: u32 = 0xf7f7fb20;
pub(crate) const NID_SYS_MEMSET: u32 = 0x68b9b011;
pub(crate) const NID_SYS_LWMUTEX_CREATE: u32 = 0x2f85c0ef;
pub(crate) const NID_SYS_HEAP_CREATE_HEAP: u32 = 0xb2fcf2c8;
pub(crate) const NID_SYS_HEAP_DELETE_HEAP: u32 = 0xaede4b03;
pub(crate) const NID_SYS_HEAP_MALLOC: u32 = 0x35168520;
pub(crate) const NID_SYS_HEAP_MEMALIGN: u32 = 0x44265c08;
pub(crate) const NID_SYS_HEAP_FREE: u32 = 0x8a561d92;
pub(crate) const NID_SYS_PPU_THREAD_GET_ID: u32 = 0x350d454e;
pub(crate) const NID_SYS_THREAD_CREATE_EX: u32 = 0x24a1ea07;
pub(crate) const NID_SYS_THREAD_EXIT: u32 = 0xaff080a4;
pub(crate) const NID_SYS_TIME_GET_SYSTEM_TIME: u32 = 0x8461e528;

/// Dispatch entry point for sysPrxForUser handlers.
///
/// Returns `Some(())` if the NID was handled (the handler has
/// already written r3 and any out-pointer effects). Returns
/// `None` if the NID is not owned by this module; the caller
/// chains to the next module.
pub(crate) fn dispatch(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
) -> Option<()> {
    match nid {
        NID_SYS_INITIALIZE_TLS => {
            initialize_tls(&mut adapter(runtime, source), args);
        }
        NID_SYS_PROCESS_EXIT => {
            process_exit(&mut adapter(runtime, source));
        }
        NID_SYS_MALLOC => {
            malloc(&mut adapter(runtime, source), args);
        }
        NID_SYS_FREE | NID_SYS_HEAP_DELETE_HEAP | NID_SYS_HEAP_FREE => {
            adapter(runtime, source).set_return(0);
        }
        NID_SYS_MEMSET => {
            memset(&mut adapter(runtime, source), args);
        }
        NID_SYS_LWMUTEX_CREATE => {
            lwmutex_create(&mut adapter(runtime, source), args);
        }
        NID_SYS_HEAP_CREATE_HEAP => {
            heap_create_heap(&mut adapter(runtime, source));
        }
        NID_SYS_HEAP_MALLOC => {
            heap_malloc(&mut adapter(runtime, source), args);
        }
        NID_SYS_HEAP_MEMALIGN => {
            heap_memalign(&mut adapter(runtime, source), args);
        }
        NID_SYS_PPU_THREAD_GET_ID => {
            // Look up the caller's guest-facing PPU thread id
            // from the LV2 host's thread table. When the table
            // has not been seeded, fall back to the canonical
            // PSL1GHT primary id (0x0100_0000).
            let id: u64 = runtime
                .lv2_host()
                .ppu_thread_id_for_unit(source)
                .map(|tid| tid.raw())
                .unwrap_or(0x0100_0000);
            let ptr = args[0] as u32;
            let mut ctx = adapter(runtime, source);
            ctx.write_guest(ptr as u64, &id.to_be_bytes());
            ctx.set_return(0);
        }
        NID_SYS_TIME_GET_SYSTEM_TIME => {
            adapter(runtime, source).set_return(1_000_000);
        }
        NID_SYS_THREAD_CREATE_EX => {
            // PSL1GHT's sysThreadCreateEx maps directly onto
            // sys_ppu_thread_create. Arg layout matches: r3
            // through r8 carry id_ptr, opd_ptr, arg, priority,
            // stacksize, flags.
            runtime.dispatch_lv2_request(
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
            runtime.dispatch_lv2_request(
                cellgov_lv2::Lv2Request::PpuThreadExit {
                    exit_value: args[0],
                },
                source,
            );
        }
        _ => return None,
    }
    Some(())
}

fn adapter(runtime: &mut Runtime, source: UnitId) -> RuntimeHleAdapter<'_> {
    RuntimeHleAdapter {
        memory: &mut runtime.memory,
        registry: &mut runtime.registry,
        heap_ptr: &mut runtime.hle_heap_ptr,
        next_id: &mut runtime.hle_next_id,
        source,
    }
}

pub(crate) fn initialize_tls(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let tls_seg_addr = args[2] as u32;
    let tls_seg_size = args[3] as u32;
    let tls_mem_size = args[4] as u32;

    let slot_size = tls_mem_size + 0x30;
    let tls_base: u32 = 0x10400000;

    let src = tls_seg_addr as usize;
    let dst = (tls_base + 0x30) as usize;
    let copy_len = tls_seg_size as usize;
    let init_data: Vec<u8> =
        if src + copy_len <= ctx.guest_memory_len() && dst + copy_len <= ctx.guest_memory_len() {
            ctx.guest_memory()[src..src + copy_len].to_vec()
        } else {
            vec![]
        };
    if !init_data.is_empty() {
        ctx.write_guest(dst as u64, &init_data);
    }

    let bss_start = dst + copy_len;
    let bss_len = (slot_size - 0x30 - tls_seg_size) as usize;
    if bss_len > 0 && bss_start + bss_len <= ctx.guest_memory_len() {
        let zeros = vec![0u8; bss_len];
        ctx.write_guest(bss_start as u64, &zeros);
    }

    let r13_val = (tls_base + 0x30 + 0x7000) as u64;
    ctx.set_register(13, r13_val);
    ctx.set_return(0);
}

pub(crate) fn malloc(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let size = args[1] as u32;
    let ptr = ctx.heap_alloc(size, 16);
    ctx.set_return(ptr as u64);
}

pub(crate) fn memset(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let ptr = args[1] as usize;
    let val = args[2] as u8;
    let size = args[3] as usize;
    if ptr + size <= ctx.guest_memory_len() {
        let data = vec![val; size];
        ctx.write_guest(ptr as u64, &data);
    }
    ctx.set_return(args[1]);
}

pub(crate) fn lwmutex_create(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let mutex_ptr = args[1] as u32;
    let attr_ptr = args[2] as u32;

    let mem = ctx.guest_memory();
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
        (0x2, 0x20)
    };

    let sleep_queue = ctx.alloc_id();

    let mut buf = [0u8; 24];
    buf[0..4].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
    buf[8..12].copy_from_slice(&(recursive | protocol).to_be_bytes());
    buf[16..20].copy_from_slice(&sleep_queue.to_be_bytes());

    let target = mutex_ptr as usize;
    if target + 24 <= ctx.guest_memory_len() {
        ctx.write_guest(target as u64, &buf);
    }
    ctx.set_return(0);
}

pub(crate) fn heap_create_heap(ctx: &mut dyn HleContext) {
    let id = ctx.alloc_id();
    ctx.set_return(id as u64);
}

pub(crate) fn heap_malloc(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let size = args[2] as u32;
    let ptr = ctx.heap_alloc(size, 16);
    ctx.set_return(ptr as u64);
}

pub(crate) fn heap_memalign(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let align = (args[2] as u32).max(16);
    let size = args[3] as u32;
    let ptr = ctx.heap_alloc(size, align);
    ctx.set_return(ptr as u64);
}

pub(crate) fn process_exit(ctx: &mut dyn HleContext) {
    ctx.set_unit_finished();
    ctx.set_return(0);
}
