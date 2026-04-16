//! sysPrxForUser HLE implementations.

use crate::hle_context::HleContext;

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
