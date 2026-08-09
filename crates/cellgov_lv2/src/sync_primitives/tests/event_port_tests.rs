use super::*;

const P: u32 = 7;
const Q: u32 = 42;

fn table_with_ipc_port() -> EventPortTable {
    let mut t = EventPortTable::new();
    t.create_with_id(P, SYS_EVENT_PORT_IPC, 0xAABB);
    t
}

#[test]
fn a_new_port_is_unbound() {
    let t = table_with_ipc_port();
    let port = t.lookup(P).expect("port present");
    assert_eq!(port.queue(), None);
    assert_eq!(port.port_type(), SYS_EVENT_PORT_IPC);
    assert_eq!(port.name(), 0xAABB);
}

#[test]
fn connect_binds_the_queue() {
    let mut t = table_with_ipc_port();
    assert_eq!(t.connect(P, Q, SYS_EVENT_PORT_IPC), Ok(()));
    assert_eq!(t.lookup(P).unwrap().queue(), Some(Q));
}

#[test]
fn connect_rejects_a_port_of_the_other_type() {
    let mut t = table_with_ipc_port();
    assert_eq!(
        t.connect(P, Q, SYS_EVENT_PORT_LOCAL),
        Err(EventPortConnectError::WrongType)
    );
    assert_eq!(t.lookup(P).unwrap().queue(), None);
}

#[test]
fn a_second_connect_keeps_the_first_binding() {
    let mut t = table_with_ipc_port();
    t.connect(P, Q, SYS_EVENT_PORT_IPC).unwrap();
    assert_eq!(
        t.connect(P, Q + 1, SYS_EVENT_PORT_IPC),
        Err(EventPortConnectError::AlreadyConnected)
    );
    // The refusal must not have retargeted the port.
    assert_eq!(t.lookup(P).unwrap().queue(), Some(Q));
}

#[test]
fn connect_on_an_unknown_port_is_refused() {
    let mut t = EventPortTable::new();
    assert_eq!(
        t.connect(P, Q, SYS_EVENT_PORT_IPC),
        Err(EventPortConnectError::UnknownPort)
    );
}

#[test]
fn disconnect_clears_the_binding_and_allows_a_reconnect() {
    let mut t = table_with_ipc_port();
    t.connect(P, Q, SYS_EVENT_PORT_IPC).unwrap();
    assert_eq!(t.disconnect(P), Ok(()));
    assert_eq!(t.lookup(P).unwrap().queue(), None);
    assert_eq!(t.connect(P, Q + 1, SYS_EVENT_PORT_IPC), Ok(()));
}

#[test]
fn disconnect_without_a_binding_is_refused() {
    let mut t = table_with_ipc_port();
    assert_eq!(t.disconnect(P), Err(EventPortDisconnectError::NotConnected));
    assert_eq!(
        t.disconnect(P + 1),
        Err(EventPortDisconnectError::UnknownPort)
    );
}

#[test]
fn destroy_is_refused_while_connected() {
    let mut t = table_with_ipc_port();
    t.connect(P, Q, SYS_EVENT_PORT_IPC).unwrap();
    assert_eq!(t.destroy(P), Err(EventPortDestroyError::Connected));
    assert!(t.lookup(P).is_some(), "a refused destroy keeps the port");
    t.disconnect(P).unwrap();
    assert_eq!(t.destroy(P), Ok(()));
    assert!(t.lookup(P).is_none());
}

#[test]
fn destroy_on_an_unknown_port_is_refused() {
    let mut t = EventPortTable::new();
    assert_eq!(t.destroy(P), Err(EventPortDestroyError::UnknownPort));
}

#[test]
fn unbind_queue_clears_only_ports_targeting_that_queue() {
    let mut t = EventPortTable::new();
    t.create_with_id(1, SYS_EVENT_PORT_IPC, 0);
    t.create_with_id(2, SYS_EVENT_PORT_IPC, 0);
    t.create_with_id(3, SYS_EVENT_PORT_LOCAL, 0);
    t.connect(1, Q, SYS_EVENT_PORT_IPC).unwrap();
    t.connect(2, Q + 1, SYS_EVENT_PORT_IPC).unwrap();
    t.connect(3, Q, SYS_EVENT_PORT_LOCAL).unwrap();

    t.unbind_queue(Q);

    assert_eq!(t.lookup(1).unwrap().queue(), None);
    assert_eq!(t.lookup(2).unwrap().queue(), Some(Q + 1));
    assert_eq!(t.lookup(3).unwrap().queue(), None);
    // An unbound port is destroyable, which is the point of unbinding
    // on queue destroy rather than leaving a dangling id.
    assert_eq!(t.destroy(1), Ok(()));
}
