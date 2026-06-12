//! Process queries, SS access control, lock-line classification, and SPU init/groups.

use super::*;

#[test]
fn syscall_48_writes_priority_to_priop() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::Unsupported {
            number: 48,
            args: [0x0100_0000, 0x9000, 0, 0, 0, 0, 0, 0],
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9000);
                assert_eq!(range.length(), 4);
                assert_eq!(bytes.bytes(), &1001u32.to_be_bytes());
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn ss_access_control_engine_pkg_id_1_returns_enosys() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SsAccessControlEngine {
            pkg_id: 1,
            a2: 0x9000,
            a3: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
    );
}

#[test]
fn ss_access_control_engine_pkg_id_2_writes_program_authority_id() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SsAccessControlEngine {
            pkg_id: 2,
            a2: 0x9000,
            a3: 0,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 1);
            if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                assert_eq!(range.start().raw(), 0x9000);
                assert_eq!(range.length(), 8);
                let v = u64::from_be_bytes(bytes.bytes().try_into().unwrap());
                assert_eq!(v, 0x1070_0000_3A00_0001);
            } else {
                panic!("expected SharedWriteIntent");
            }
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn ss_access_control_engine_pkg_id_2_efault_on_zero_a2() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SsAccessControlEngine {
            pkg_id: 2,
            a2: 0,
            a3: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn ss_access_control_engine_pkg_id_2_efault_when_a2_overflows_u32() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SsAccessControlEngine {
            pkg_id: 2,
            a2: u64::from(u32::MAX) + 1,
            a3: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn ss_access_control_engine_default_pkg_id_returns_ss_status() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SsAccessControlEngine {
            pkg_id: 99,
            a2: 0,
            a3: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(result, Lv2Dispatch::immediate(0x8001_051D));
}

#[test]
fn ppu_thread_yield_is_no_op_returning_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(Lv2Request::PpuThreadYield, UnitId::new(0), &rt);
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn ppu_thread_start_returns_ok_because_auto_started_at_create() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::PpuThreadStart { target: 0x101 },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(result, Lv2Dispatch::immediate(0));
}

#[test]
fn process_is_stack_returns_real_answer_from_tracked_thread_ranges() {
    use crate::ppu_thread::PpuThreadAttrs;
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(
        UnitId::new(0),
        PpuThreadAttrs {
            entry: 0x1000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x1_0000,
            priority: 1001,
            tls_base: 0,
        },
    );
    let rt = FakeRuntime::new(0x10000);

    let on_stack = host.dispatch(
        Lv2Request::ProcessIsStack { addr: 0xD000_0500 },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(on_stack, Lv2Dispatch::immediate(1));

    let below = host.dispatch(
        Lv2Request::ProcessIsStack { addr: 0xCFFF_FFFF },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(below, Lv2Dispatch::immediate(0));

    // Half-open: stack_base + stack_size is not on stack.
    let at_end = host.dispatch(
        Lv2Request::ProcessIsStack { addr: 0xD001_0000 },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(at_end, Lv2Dispatch::immediate(0));
}

fn dispatch_lock_line(addr: u32, flags: u64) -> Lv2Dispatch {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    host.dispatch(
        Lv2Request::ProcessIsSpuLockLineReservationAddress { addr, flags },
        UnitId::new(0),
        &rt,
    )
}

#[test]
fn process_is_spu_lock_line_reservation_address_zero_flags_is_einval() {
    let result = dispatch_lock_line(0xE000_0000, 0);
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_unknown_flag_bit_is_einval() {
    let result = dispatch_lock_line(0xE000_0000, 0x4);
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_raw_spu_region_returns_ok() {
    let result = dispatch_lock_line(0xE000_0000, 0x1);
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_private_spu_rejects_raw_flag() {
    let result = dispatch_lock_line(0xF000_0000, 0x1);
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, cell_errors::CELL_EPERM.into());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_private_spu_accepts_thr_flag() {
    let result = dispatch_lock_line(0xF000_0000, 0x2);
    match result {
        Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, 0),
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_ppu_stack_is_eperm() {
    let result = dispatch_lock_line(0xD000_0000, 0x2);
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, cell_errors::CELL_EPERM.into());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn process_is_spu_lock_line_reservation_address_unknown_region_is_einval() {
    let result = dispatch_lock_line(0x3000_0000, 0x2);
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn dispatch_spu_init(max_usable_spu: u32, max_raw_spu: u32) -> Lv2Dispatch {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    host.dispatch(
        Lv2Request::SpuInitialize {
            max_usable_spu,
            max_raw_spu,
        },
        UnitId::new(0),
        &rt,
    )
}

#[test]
fn spu_initialize_accepts_typical_lv2_caps() {
    let result = dispatch_spu_init(6, 1);
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn spu_initialize_rejects_max_raw_above_five() {
    let result = dispatch_spu_init(6, 6);
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn spu_initialize_accepts_zero_raw_spu() {
    let result = dispatch_spu_init(6, 0);
    match result {
        Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, 0),
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn spu_thread_group_destroy_unknown_id_is_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::SpuThreadGroupDestroy { id: 0xDEAD },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, cell_errors::CELL_ESRCH.into()),
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn spu_thread_group_destroy_created_group_returns_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let create = host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x9000,
            num_threads: 1,
            priority: 100,
            attr_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    let group_id = match create {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            let payload = match &effects[0] {
                cellgov_effects::Effect::SharedWriteIntent { bytes, .. } => bytes.bytes(),
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            };
            u32::from_be_bytes(payload[..4].try_into().unwrap())
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    let destroy = host.dispatch(
        Lv2Request::SpuThreadGroupDestroy { id: group_id },
        UnitId::new(0),
        &rt,
    );
    match destroy {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
    let second = host.dispatch(
        Lv2Request::SpuThreadGroupDestroy { id: group_id },
        UnitId::new(0),
        &rt,
    );
    match second {
        Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, cell_errors::CELL_ESRCH.into()),
        other => panic!("expected Immediate, got {other:?}"),
    }
}
