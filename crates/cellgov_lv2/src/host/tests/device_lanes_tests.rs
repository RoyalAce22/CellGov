//! The RSX context and the kernel cursors reach the host's sync-state
//! partial.

use super::*;

type HostChange = (&'static str, fn(&mut Lv2Host));

#[test]
fn rsx_context_fields_display_buffer_slots_and_kernel_cursors_move_the_partial() {
    let base = Lv2Host::new();
    let changes: [HostChange; 11] = [
        ("allocated", |h| h.state.rsx_context.allocated = true),
        ("mem_ctx", |h| h.state.rsx_context.mem_ctx = 1),
        ("fifo_put", |h| h.state.rsx_context.fifo_put = 1),
        ("display buffer 0 width", |h| {
            h.state.rsx_context.display_buffers[0].width = 1
        }),
        ("display buffer 1 width", |h| {
            h.state.rsx_context.display_buffers[1].width = 1
        }),
        ("display buffer 0 height", |h| {
            h.state.rsx_context.display_buffers[0].height = 1
        }),
        ("next_kernel_id", |h| h.state.next_kernel_id += 1),
        ("mem_alloc_ptr", |h| h.state.mem_alloc_ptr += 1),
        ("mmapper_addr_cursor", |h| h.state.mmapper_addr_cursor += 1),
        ("rsx_mem_alloc_ptr", |h| h.state.rsx_mem_alloc_ptr += 1),
        ("rsx_mem_handle_counter", |h| {
            h.state.rsx_mem_handle_counter += 1
        }),
    ];
    let mut seen = vec![base.sync_partial()];
    for (name, change) in changes {
        let mut host = base.clone();
        change(&mut host);
        let partial = host.sync_partial();
        assert!(!seen.contains(&partial), "{name}");
        assert_eq!(partial, host.sync_partial_from_scratch());
        seen.push(partial);
    }
}

#[test]
fn two_kernel_cursors_that_exchange_values_hash_differently() {
    let mut a = Lv2Host::new();
    a.state.mem_alloc_ptr = 7;
    a.state.rsx_mem_handle_counter = 9;
    let mut b = Lv2Host::new();
    b.state.mem_alloc_ptr = 9;
    b.state.rsx_mem_handle_counter = 7;
    assert_ne!(a.sync_partial(), b.sync_partial());
}
