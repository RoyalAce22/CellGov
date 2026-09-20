use super::*;

#[test]
fn cond_create_writes_id_and_binds_mutex() {
    let (mut host, rt, src) = cond_fixture();
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let entry = host.conds().lookup(cond_id).unwrap();
    assert_eq!(entry.mutex_id(), mutex_id);
    assert_eq!(entry.mutex_kind(), CondMutexKind::Mutex);
    assert!(entry.waiters().is_empty());
}

#[test]
fn cond_create_unknown_mutex_returns_esrch() {
    let (mut host, rt, src) = cond_fixture();
    let r = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id: 0xDEAD,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, errno::CELL_ESRCH.into());
    assert!(host.conds().is_empty());
}

#[test]
fn cond_destroy_empty_succeeds() {
    let (mut host, rt, src) = cond_fixture();
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    let r = host.dispatch(Lv2Request::CondDestroy { id: cond_id }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert!(host.conds().lookup(cond_id).is_none());
}

#[test]
fn cond_destroy_unknown_returns_esrch() {
    let (mut host, rt, src) = cond_fixture();
    let r = host.dispatch(Lv2Request::CondDestroy { id: 0xDEAD }, src, &rt);
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, errno::CELL_ESRCH.into());
}

#[test]
fn cond_destroy_with_waiter_returns_ebusy() {
    let (mut host, rt, src) = cond_fixture();
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    let r = host.dispatch(Lv2Request::CondDestroy { id: cond_id }, src, &rt);
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, errno::CELL_EBUSY.into());
}

#[test]
fn cond_create_with_pshared_attr_captures_ipc_key() {
    let rt = runtime_with_cond_attr(0x100, 0x8006_0100_0000_0030, &[]);
    let (mut host, rt, src) = cond_fixture_with(rt, UnitId::new(0));
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    let cond_id = create_cond_with_attr(&mut host, src, &rt, mutex_id);
    assert_eq!(
        host.derived.cond_ipc_keys.get(&cond_id),
        Some(&0x8006_0100_0000_0030)
    );
}

#[test]
fn cond_create_without_pshared_stays_keyless() {
    let rt = runtime_with_cond_attr(0x200, 0x8006_0100_0000_0030, &[]);
    let (mut host, rt, src) = cond_fixture_with(rt, UnitId::new(0));
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    let cond_id = create_cond_with_attr(&mut host, src, &rt, mutex_id);
    assert!(!host.derived.cond_ipc_keys.contains_key(&cond_id));
}

#[test]
fn cond_destroy_drops_captured_ipc_key() {
    let rt = runtime_with_cond_attr(0x100, 0x8006_0100_0000_0030, &[]);
    let (mut host, rt, src) = cond_fixture_with(rt, UnitId::new(0));
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    let cond_id = create_cond_with_attr(&mut host, src, &rt, mutex_id);
    host.dispatch(Lv2Request::CondDestroy { id: cond_id }, src, &rt);
    assert!(!host.derived.cond_ipc_keys.contains_key(&cond_id));
}
