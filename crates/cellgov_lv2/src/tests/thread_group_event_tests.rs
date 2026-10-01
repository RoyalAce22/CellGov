//! SPU thread-group event-binding state tests: sync-partial folding and
//! queue removal.

use super::*;

fn img(raw: u32) -> SpuImageHandle {
    SpuImageHandle::new(raw).unwrap()
}

fn table_with_thread() -> ThreadGroupTable {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.initialize_thread(gid, 0, img(1), [0; 4]).unwrap();
    t
}

const THREAD: u32 = MAX_SLOTS_PER_GROUP;

#[test]
fn a_group_event_connection_folds_into_the_sync_partial() {
    let mut a = table_with_thread();
    let b = table_with_thread();
    assert!(a.set_group_event_queue(1, event::GROUP_RUN, Some(0xD100_0001)));
    assert_ne!(a.sync_partial(), b.sync_partial());
    assert_eq!(a.sync_partial(), a.sync_partial_from_scratch());
    assert!(a.set_group_event_queue(1, event::GROUP_RUN, None));
    assert_eq!(a.sync_partial(), b.sync_partial());
}

#[test]
fn a_thread_port_connection_folds_into_the_sync_partial() {
    let mut a = table_with_thread();
    let b = table_with_thread();
    assert!(a.set_thread_port_queue(THREAD, 5, Some(0xD100_0001)));
    assert_ne!(a.sync_partial(), b.sync_partial());
    assert_eq!(a.sync_partial(), a.sync_partial_from_scratch());
    let mut c = table_with_thread();
    assert!(c.set_thread_port_queue(THREAD, 6, Some(0xD100_0001)));
    assert_ne!(a.sync_partial(), c.sync_partial());
    assert!(a.set_thread_port_queue(THREAD, 5, None));
    assert_eq!(a.sync_partial(), b.sync_partial());
}

#[test]
fn a_queue_binding_folds_into_the_sync_partial() {
    let mut a = table_with_thread();
    let b = table_with_thread();
    a.bind_thread_queue(THREAD, 7, 0xD100_0001).unwrap();
    assert_ne!(a.sync_partial(), b.sync_partial());
    assert_eq!(a.sync_partial(), a.sync_partial_from_scratch());
    let mut c = table_with_thread();
    c.bind_thread_queue(THREAD, 8, 0xD100_0001).unwrap();
    assert_ne!(a.sync_partial(), c.sync_partial());
    a.unbind_thread_queue(THREAD, 7).unwrap();
    assert_eq!(a.sync_partial(), b.sync_partial());
}

#[test]
fn a_binding_lookup_on_an_uninitialized_thread_is_unknown() {
    let mut t = table_with_thread();
    assert_eq!(t.thread_slot(THREAD + 1), None);
    assert_eq!(
        t.bind_thread_queue(THREAD + 1, 1, 0xD100_0001),
        Err(BindQueueError::UnknownThread)
    );
    assert_eq!(
        t.unbind_thread_queue(THREAD + 1, 1),
        Err(UnbindQueueError::UnknownThread)
    );
    assert!(!t.set_thread_port_queue(THREAD + 1, 0, Some(1)));
    assert_eq!(t.thread_port_queue(THREAD + 1, 0), None);
}

#[test]
fn removing_a_queue_clears_only_the_references_to_it() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(2).unwrap();
    t.initialize_thread(gid, 0, img(1), [0; 4]).unwrap();
    t.initialize_thread(gid, 1, img(1), [0; 4]).unwrap();
    let (gone, kept) = (0xD100_0001, 0xD100_0002);
    assert!(t.set_group_event_queue(gid, event::GROUP_RUN, Some(gone)));
    assert!(t.set_group_event_queue(gid, event::GROUP_EXCEPTION, Some(kept)));
    assert!(t.set_group_port_queue(gid, 4, Some(gone)));
    assert!(t.set_thread_port_queue(THREAD + 1, 5, Some(kept)));
    t.bind_thread_queue(THREAD, 1, gone).unwrap();
    t.bind_thread_queue(THREAD, 2, kept).unwrap();

    t.unbind_event_queue(gone);

    assert_eq!(t.group_event_queue(gid, event::GROUP_RUN), None);
    assert_eq!(t.group_event_queue(gid, event::GROUP_EXCEPTION), Some(kept));
    assert_eq!(t.thread_port_queue(THREAD, 4), None);
    assert_eq!(t.thread_port_queue(THREAD + 1, 4), None);
    assert_eq!(t.thread_port_queue(THREAD + 1, 5), Some(kept));
    assert_eq!(
        t.thread_queue_bindings(THREAD)
            .unwrap()
            .values()
            .copied()
            .collect::<Vec<_>>(),
        vec![kept]
    );
    assert!(!t.get(gid).unwrap().port_is_free(5));
    assert!(t.get(gid).unwrap().port_is_free(4));
    assert_eq!(t.sync_partial(), t.sync_partial_from_scratch());
}
