//! Stub / unsupported / unresolved-import routing and invariant-break dispositions.

use super::*;
use cellgov_ps3_abi::lv2::syscall::SYSCALL_TABLE_SLOTS;
use std::num::NonZeroU8;

#[test]
fn stub_dispatch_returns_cell_ok_for_process_exit() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let req = Lv2Request::ProcessExit { code: 0 };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn extracted_census_rejects_an_ordinal_outside_its_table() {
    let mut host = Lv2Host::new();
    host.set_firmware_identity(
        "3.21",
        [
            0x06, 0xa2, 0x23, 0x62, 0xf3, 0x1e, 0xaf, 0x8c, 0x3a, 0x35, 0x28, 0x3b, 0xe3, 0x5a,
            0x1f, 0xfa, 0x31, 0x40, 0x8c, 0xa3, 0x3b, 0x15, 0x0e, 0x65, 0x64, 0xd0, 0x24, 0x2e,
            0x4d, 0xbf, 0x4e, 0x25,
        ],
    );
    let rt = FakeRuntime::new(256);

    let result = host.dispatch_with_ordinal(
        Lv2Request::ProcessExit { code: 0 },
        UnitId::new(0),
        &rt,
        SYSCALL_TABLE_SLOTS,
    );

    assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_ENOSYS.into()));
    assert_eq!(
        host.observability()
            .dispatch_nonzero_returns
            .get(&u64::from(errno::CELL_ENOSYS)),
        Some(&1)
    );
    assert_eq!(
        host.observability()
            .dispatch_return_pairs
            .get(&("ProcessExit", u64::from(errno::CELL_ENOSYS))),
        Some(&1)
    );
}

#[test]
fn unextracted_census_keeps_normal_dispatch() {
    let mut host = Lv2Host::new();
    host.set_firmware_identity("unextracted", [0; 32]);
    let rt = FakeRuntime::new(256);

    let result = host.dispatch_with_ordinal(
        Lv2Request::ProcessExit { code: 0 },
        UnitId::new(0),
        &rt,
        SYSCALL_TABLE_SLOTS,
    );

    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn census_gating_does_not_replace_a_hypercall_rejection() {
    let mut host = Lv2Host::new();
    host.set_firmware_identity(
        "3.21",
        [
            0x06, 0xa2, 0x23, 0x62, 0xf3, 0x1e, 0xaf, 0x8c, 0x3a, 0x35, 0x28, 0x3b, 0xe3, 0x5a,
            0x1f, 0xfa, 0x31, 0x40, 0x8c, 0xa3, 0x3b, 0x15, 0x0e, 0x65, 0x64, 0xd0, 0x24, 0x2e,
            0x4d, 0xbf, 0x4e, 0x25,
        ],
    );
    let rt = FakeRuntime::new(256);

    let result = host.dispatch_with_ordinal(
        Lv2Request::Hypercall {
            lev: NonZeroU8::new(1).unwrap(),
            r11: SYSCALL_TABLE_SLOTS,
            args: [0; 8],
        },
        UnitId::new(0),
        &rt,
        SYSCALL_TABLE_SLOTS,
    );

    assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_EINVAL.into()));
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
    assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_ENOSYS.into()));
}

#[test]
fn routed_unsupported_fidelity_table_matches_dispatch_exactly() {
    use crate::request::fidelity::ROUTED_UNSUPPORTED_ARMS;
    // The probe covers every table slot, so a routed number outside
    // the range would escape it.
    for (n, arm, _) in ROUTED_UNSUPPORTED_ARMS {
        assert!(
            *n < SYSCALL_TABLE_SLOTS,
            "{arm} ({n}) outside the probed slot range"
        );
    }

    let rt = FakeRuntime::new(0x10000);
    let mut handled = Vec::new();
    for number in 0..SYSCALL_TABLE_SLOTS {
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
        let mut v: Vec<u64> = ROUTED_UNSUPPORTED_ARMS.iter().map(|(n, ..)| *n).collect();
        v.sort_unstable();
        v
    };
    assert_eq!(
        handled, tagged,
        "dispatch's routed-Unsupported set diverged from \
         ROUTED_UNSUPPORTED_ARMS; update the fidelity table (and \
         regenerate docs/lv2/) to match dispatch.rs"
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
        Lv2Dispatch::immediate(cellgov_ps3_abi::lv2::errno::CELL_EINVAL.into())
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
        Lv2Dispatch::immediate(cellgov_ps3_abi::lv2::errno::CELL_EINVAL.into())
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
    assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_ESRCH.into()));
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
    assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_EINVAL.into()));
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
    assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_EINVAL.into()));
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
    assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_ENOSYS.into()));
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
            unk: 0,
            priority: 1000,
            stacksize: 0x4000,
            flags: 0x1, // JOINABLE -- unmodeled
            threadname_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(
        host.observability().invariant_break_count > before,
        "expected log_invariant_break to fire on nonzero flags"
    );
}

#[test]
fn ppu_thread_create_nonzero_unk_is_witnessed() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let before = host.invariant_break_site_count("dispatch.ppu_thread_create_unconsumed_unk");
    let _ = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x9000,
            param_ptr: 0x4000_0000,
            arg: 0,
            unk: 0x10,
            priority: 1000,
            stacksize: 0x4000,
            flags: 0,
            threadname_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        host.invariant_break_site_count("dispatch.ppu_thread_create_unconsumed_unk"),
        before + 1
    );
}

#[test]
fn ppu_thread_create_joinable_plus_interrupt_flags_return_eperm() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let before = host.observability().invariant_break_count;
    let result = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x9000,
            param_ptr: 0x4000_0000,
            arg: 0,
            unk: 0,
            priority: 1000,
            stacksize: 0x4000,
            flags: 0x3, // JOINABLE | INTERRUPT
            threadname_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_EPERM.into()));
    assert_eq!(
        host.observability().invariant_break_count,
        before,
        "a modeled refusal must not report an invariant break"
    );
}
