//! Stub / unsupported / unresolved-import routing and invariant-break dispositions.

use super::*;

#[test]
fn stub_dispatch_returns_cell_ok_for_process_exit() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let req = Lv2Request::ProcessExit { code: 0 };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn unsupported_dispatch_returns_cell_enosys() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let req = Lv2Request::Unsupported {
        number: 999,
        args: [0; 8],
    };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
    );
}

// The exhaustive probe below is what keeps ROUTED_UNSUPPORTED_ARMS
// honest: the typed half of the fidelity map is compile-enforced, but
// a routed number's table row is data, and this is its drift gate.
#[test]
fn routed_unsupported_fidelity_table_matches_dispatch_exactly() {
    use crate::request::fidelity::ROUTED_UNSUPPORTED_ARMS;

    // LV2's syscall table has 1024 slots; the probe covers them all,
    // so a routed number outside the range would escape it.
    for (n, name, _) in ROUTED_UNSUPPORTED_ARMS {
        assert!(*n < 1024, "{name} ({n}) outside the probed slot range");
    }

    let rt = FakeRuntime::new(0x10000);
    let mut handled = Vec::new();
    for number in 0..1024u64 {
        // Fresh host per number: arms mutate state, and the stub-site
        // counter must attribute to exactly one dispatch.
        let mut host = Lv2Host::new();
        let _ = host.dispatch(
            Lv2Request::Unsupported {
                number,
                args: [0; 8],
            },
            UnitId::new(0),
            &rt,
        );
        if host.invariant_break_site_count("dispatch.unsupported_stub") == 0 {
            handled.push(number);
        }
    }
    let tagged: Vec<u64> = {
        let mut v: Vec<u64> = ROUTED_UNSUPPORTED_ARMS.iter().map(|(n, _, _)| *n).collect();
        v.sort_unstable();
        v
    };
    assert_eq!(
        handled, tagged,
        "dispatch's routed-Unsupported set diverged from \
         ROUTED_UNSUPPORTED_ARMS; update the fidelity table (and \
         regenerate docs/lv2_fidelity.md) to match dispatch.rs"
    );
}

#[test]
fn unresolved_import_dispatch_returns_cell_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let req = Lv2Request::UnresolvedImport {
        nid: 0x744680a2, // sys_initialize_tls
    };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cellgov_ps3_abi::cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn unresolved_import_dispatch_handles_unknown_nid() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let req = Lv2Request::UnresolvedImport { nid: 0xdead_beef };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cellgov_ps3_abi::cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn syscall_621_returns_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 621,
            args: [0xa, 0, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn syscall_512_returns_zero_non_root() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 512,
            args: [0x1000500, 0, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn syscall_677_returns_ok_no_effects() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 677,
            args: [0x202, 1, 1, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn syscall_136_event_port_connect_local_on_unknown_ids_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 136,
            args: [0x4000_0002, 0x4000_0001, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
    );
}

#[test]
fn malformed_request_records_invariant_break_and_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let before = host.observability().invariant_break_count;
    let result = host.dispatch(
        Lv2Request::Malformed {
            number: 99,
            reason: "test",
            args: [0; 8],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
    assert!(host.observability().invariant_break_count > before);
}

#[test]
fn hypercall_records_invariant_break_and_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let before = host.observability().invariant_break_count;
    let result = host.dispatch(
        Lv2Request::Hypercall {
            lev: std::num::NonZeroU8::new(1).unwrap(),
            r11: 0xCAFE,
            args: [0; 8],
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
    assert!(host.observability().invariant_break_count > before);
}

#[test]
fn spu_thread_group_terminate_logs_invariant_break_and_returns_enosys() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let before = host.observability().invariant_break_count;
    let result = host.dispatch(
        Lv2Request::SpuThreadGroupTerminate {
            group_id: 1,
            value: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
    );
    assert!(host.observability().invariant_break_count > before);
}

#[test]
fn ppu_thread_create_logs_invariant_break_on_nonzero_flags() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let before = host.observability().invariant_break_count;
    let _ = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x9000,
            param_ptr: 0x4000_0000,
            arg: 0,
            priority: 1000,
            stacksize: 0x4000,
            flags: 0x1, // JOINABLE -- unmodeled
        },
        UnitId::new(0),
        &rt,
    );
    assert!(
        host.observability().invariant_break_count > before,
        "expected log_invariant_break to fire on nonzero flags"
    );
}
