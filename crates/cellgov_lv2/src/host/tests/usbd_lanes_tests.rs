//! Every USB driver field moves the driver's sync-state term.

use super::*;

type UsbdChange = (&'static str, fn(&mut UsbdState));

#[test]
fn every_usbd_field_moves_the_term() {
    let base = UsbdState::new();
    let changes: [UsbdChange; 4] = [
        ("handle", |s| {
            s.handles.insert(1);
        }),
        ("product", |s| {
            s.ldds.insert(b"a".to_vec());
        }),
        ("empty product", |s| {
            s.ldds.insert(Vec::new());
        }),
        ("waiter", |s| {
            s.waiters.push_back(UsbdWaiter {
                thread: PpuThreadId::PRIMARY,
                handle: 1,
                out_ptrs: [1, 2, 3],
            })
        }),
    ];
    let mut seen = vec![base.sync_term()];
    for (name, change) in changes {
        let mut s = base.clone();
        change(&mut s);
        let term = s.sync_term();
        assert!(!seen.contains(&term), "{name}");
        seen.push(term);
    }
}

#[test]
fn initialize_moves_the_host_partial_by_the_usbd_and_id_cursor_terms() {
    use cellgov_mem::lanes::{source, value_term};
    let rt =
        crate::host::test_support::FakeRuntime::with_memory(cellgov_mem::GuestMemory::new(0x10000));
    let base = crate::host::Lv2Host::new();
    let mut host = base.clone();
    host.dispatch(
        crate::request::Lv2Request::UsbdInitialize { handle_ptr: 0x2000 },
        cellgov_event::UnitId::new(0),
        &rt,
    );
    let id_cursor = |h: &crate::host::Lv2Host| {
        value_term(
            source::KERNEL_CURSORS,
            0,
            &u64::from(h.state.next_kernel_id),
        )
    };
    let usbd_delta = host
        .state
        .usbd
        .sync_term()
        .wrapping_sub(base.state.usbd.sync_term());
    assert_ne!(usbd_delta, 0);
    assert_eq!(
        host.sync_partial().wrapping_sub(base.sync_partial()),
        usbd_delta.wrapping_add(id_cursor(&host).wrapping_sub(id_cursor(&base)))
    );
}

#[test]
fn every_waiter_pointer_moves_the_term() {
    let with = |out_ptrs| {
        let mut s = UsbdState::new();
        s.waiters.push_back(UsbdWaiter {
            thread: PpuThreadId::PRIMARY,
            handle: 1,
            out_ptrs,
        });
        s.sync_term()
    };
    let base = with([1, 2, 3]);
    for moved in [[9, 2, 3], [1, 9, 3], [1, 2, 9]] {
        assert_ne!(with(moved), base, "{moved:?}");
    }
}
