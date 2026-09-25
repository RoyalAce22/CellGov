//! The config table keeps its partial of the sync-state sum through
//! every path that changes it.

use super::table::{ConfigListener, ConfigTable, ServiceSpec};

/// Apply `$change`; the partial must move and equal the rebuild, in
/// release too.
macro_rules! step {
    ($t:ident, $h:ident, $what:expr, $change:expr) => {{
        let _ = $change;
        let now = $t.sync_partial();
        assert_eq!(now, $t.sync_partial_from_scratch(), "after {}", $what);
        assert_ne!(now, $h, "{} did not move the partial", $what);
        $h = now;
        let _ = $h;
    }};
}

fn listener(data: Vec<u8>) -> ConfigListener {
    ConfigListener {
        handle: 1,
        queue_id: 2,
        service_id: 3,
        min_verbosity: 0,
        listener_type: 0,
        data,
        delivered: 0,
    }
}

#[test]
fn every_config_change_moves_the_partial() {
    let mut t = ConfigTable::new();
    let mut h = t.sync_partial();
    step!(t, h, "handle", t.insert_handle(1, 2));
    step!(t, h, "service", {
        t.insert_service(
            10,
            ServiceSpec {
                service_id: 3,
                user_id: 4,
                verbosity: 1,
                data_ptr: 0,
                size: 0,
            },
            vec![5],
        )
    });
    step!(t, h, "listener", t.insert_listener(20, listener(vec![1])));
    let (event, _, _) = t.stage_event(20, 10).unwrap();
    assert_ne!(t.sync_partial(), h, "stage_event did not move the partial");
    h = t.sync_partial();
    step!(t, h, "discard", t.discard_event(event));
    step!(t, h, "unregister", t.unregister(10));
    step!(t, h, "remove listener", t.remove_listener(20));
    step!(t, h, "remove handle", t.remove_handle(1));
}

#[test]
fn listener_data_moves_the_partial() {
    let build = |data: Vec<u8>| {
        let mut t = ConfigTable::new();
        t.insert_listener(20, listener(data));
        t.sync_partial()
    };
    assert_ne!(build(vec![1]), build(vec![2]));
    assert_ne!(build(vec![]), build(vec![0]));
}

#[test]
fn a_config_change_reaches_the_host_partial() {
    let mut host = crate::host::Lv2Host::new();
    let before = host.sync_partial();
    host.state.config.insert_handle(1, 2);
    assert_ne!(host.sync_partial(), before);
    assert_eq!(host.sync_partial(), host.sync_partial_from_scratch());
}

/// Computed outside the crate from the SplitMix64 key stream of the
/// sync-state lanes.
#[test]
fn config_partial_wire_format_golden() {
    let mut t = ConfigTable::new();
    t.insert_handle(1, 2);
    assert_eq!(t.sync_partial(), 0x7372_44b6_6b65_c3f9_3ea9_67f8_8a39_1abb);
}
