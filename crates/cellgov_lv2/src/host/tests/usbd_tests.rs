//! USB host-driver dispatch: handles, the event wait, finalize's
//! terminate wake, and the no-device refusals.

use super::*;
use crate::host::process::ProcessEntry;
use crate::host::test_support::{primary_attrs, seed_primary_ppu, FakeRuntime};
use crate::request::Lv2Request;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;

const HANDLE_PTR: u32 = 0x2000;
const ARGS: [u32; 3] = [0x3000, 0x3008, 0x3010];
const ARGS_B: [u32; 3] = [0x4000, 0x4100, 0x4200];
const PRODUCT_PTR: u32 = 0x5000;

fn src() -> UnitId {
    UnitId::new(0)
}

fn rt() -> FakeRuntime {
    FakeRuntime::with_memory(GuestMemory::new(0x10000))
}

fn code_of(d: &Lv2Dispatch) -> u64 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn init(host: &mut Lv2Host, rt: &FakeRuntime) -> u32 {
    let d = host.dispatch(
        Lv2Request::UsbdInitialize {
            handle_ptr: HANDLE_PTR,
        },
        src(),
        rt,
    );
    assert_eq!(code_of(&d), 0);
    match &d {
        Lv2Dispatch::Immediate { effects, .. } => {
            crate::host::test_support::extract_write_u32(&effects[0])
        }
        _ => unreachable!(),
    }
}

fn receive(
    host: &mut Lv2Host,
    rt: &FakeRuntime,
    unit: UnitId,
    handle: u32,
    ptrs: [u32; 3],
) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::UsbdReceiveEvent {
            handle,
            arg1_ptr: ptrs[0],
            arg2_ptr: ptrs[1],
            arg3_ptr: ptrs[2],
        },
        unit,
        rt,
    )
}

fn assert_parked(d: &Lv2Dispatch, handle: u32) {
    assert!(
        matches!(
            d,
            Lv2Dispatch::Block {
                reason: Lv2BlockReason::UsbdEvent { handle: h },
                pending: PendingResponse::ReturnCode { code: 0 },
                ..
            } if *h == handle
        ),
        "got {d:?}"
    );
}

#[test]
fn initialize_mints_a_handle_and_a_second_initialize_mints_another() {
    let rt = rt();
    let mut host = Lv2Host::new();
    assert!(host.state.usbd.is_pristine());
    let a = init(&mut host, &rt);
    let b = init(&mut host, &rt);
    assert_ne!(a, b);
    assert_eq!(host.state.usbd.handles().len(), 2);
    assert!(!host.state.usbd.is_pristine());
    let d = host.dispatch(Lv2Request::UsbdInitialize { handle_ptr: 0 }, src(), &rt);
    assert_eq!(code_of(&d), u64::from(errno::CELL_EFAULT));
}

#[test]
fn every_arm_refuses_a_handle_no_initialize_minted() {
    let rt = rt();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, src());
    let bogus = 0x7fff_ffff;
    let einval = u64::from(errno::CELL_EINVAL);
    assert_eq!(
        code_of(&host.dispatch(Lv2Request::UsbdFinalize { handle: bogus }, src(), &rt)),
        einval
    );
    assert_eq!(
        code_of(&host.dispatch(
            Lv2Request::UsbdGetDeviceList {
                handle: bogus,
                list_ptr: 0x6000,
                max_devices: 4
            },
            src(),
            &rt
        )),
        einval
    );
    assert_eq!(
        code_of(&host.dispatch(
            Lv2Request::UsbdRegisterLdd {
                handle: bogus,
                product_ptr: PRODUCT_PTR,
                product_len: 4
            },
            src(),
            &rt
        )),
        einval
    );
    assert_eq!(
        code_of(&receive(&mut host, &rt, src(), bogus, ARGS)),
        einval
    );
    assert_eq!(
        code_of(&host.dispatch(
            Lv2Request::UsbdOpenDefaultPipe {
                handle: bogus,
                device: 1
            },
            src(),
            &rt
        )),
        einval
    );
    assert_eq!(
        host.obs.usbd_no_device_refusals, 0,
        "an unknown handle is the handle gate, not a device refusal"
    );
}

#[test]
fn an_empty_bus_lists_no_devices_and_acknowledges_ldd_registration() {
    let rt = rt();
    let mut host = Lv2Host::new();
    let handle = init(&mut host, &rt);
    let d = host.dispatch(
        Lv2Request::UsbdGetDeviceList {
            handle,
            list_ptr: 0x6000,
            max_devices: 4,
        },
        src(),
        &rt,
    );
    assert!(
        matches!(&d, Lv2Dispatch::Immediate { code: 0, effects } if effects.is_empty()),
        "zero devices, nothing written: {d:?}"
    );
    for req in [
        Lv2Request::UsbdRegisterLdd {
            handle,
            product_ptr: PRODUCT_PTR,
            product_len: 8,
        },
        Lv2Request::UsbdUnregisterLdd {
            handle,
            product_ptr: PRODUCT_PTR,
            product_len: 8,
        },
        Lv2Request::UsbdDetectEvent,
    ] {
        assert_eq!(code_of(&host.dispatch(req, src(), &rt)), 0);
    }
    let d = host.dispatch(
        Lv2Request::UsbdRegisterLdd {
            handle,
            product_ptr: 0xffff_fff0,
            product_len: 0x40,
        },
        src(),
        &rt,
    );
    assert_eq!(code_of(&d), u64::from(errno::CELL_EFAULT));
}

#[test]
fn device_and_pipe_scoped_calls_on_a_live_handle_are_einval_and_counted() {
    let rt = rt();
    let mut host = Lv2Host::new();
    let handle = init(&mut host, &rt);
    let reqs = [
        Lv2Request::UsbdGetDescriptorSize { handle, device: 1 },
        Lv2Request::UsbdGetDescriptor {
            handle,
            device: 1,
            desc_ptr: 0x6000,
            desc_size: 0x40,
        },
        Lv2Request::UsbdOpenPipe {
            handle,
            device: 1,
            endpoint: 0x81,
        },
        Lv2Request::UsbdOpenDefaultPipe { handle, device: 1 },
        Lv2Request::UsbdClosePipe { handle, pipe: 7 },
    ];
    let n = reqs.len() as u64;
    for req in reqs {
        assert_eq!(
            code_of(&host.dispatch(req, src(), &rt)),
            u64::from(errno::CELL_EINVAL)
        );
    }
    assert_eq!(host.obs.usbd_no_device_refusals, n);
}

#[test]
fn receive_event_parks_and_finalize_wakes_every_reader_with_the_terminate_triple() {
    let rt = rt();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, src());
    let second = UnitId::new(1);
    host.ppu_threads_mut()
        .create(second, primary_attrs())
        .expect("a second thread id");
    let handle = init(&mut host, &rt);

    assert_parked(&receive(&mut host, &rt, src(), handle, ARGS), handle);
    assert_parked(&receive(&mut host, &rt, second, handle, ARGS_B), handle);
    assert_eq!(host.state.usbd.waiters().len(), 2);
    let parked_hash = host.state.usbd.sync_term();

    let d = host.dispatch(Lv2Request::UsbdFinalize { handle }, src(), &rt);
    let Lv2Dispatch::WakeAndReturn {
        code,
        woken_unit_ids,
        response_updates,
        effects,
    } = d
    else {
        panic!("expected WakeAndReturn, got {d:?}");
    };
    assert_eq!(code, 0);
    assert_eq!(woken_unit_ids, vec![src(), second]);
    assert_eq!(
        response_updates,
        vec![
            (src(), PendingResponse::ReturnCode { code: 0 }),
            (second, PendingResponse::ReturnCode { code: 0 }),
        ]
    );
    assert_eq!(effects.len(), 6);
    let expect = [ARGS, ARGS_B]
        .concat()
        .into_iter()
        .zip([4u64, 0, 0, 4, 0, 0]);
    for (effect, (ptr, value)) in effects.iter().zip(expect) {
        let Effect::SharedWriteIntent { range, bytes, .. } = effect else {
            panic!("expected a write, got {effect:?}");
        };
        assert_eq!(range.start().raw(), u64::from(ptr));
        assert_eq!(bytes.bytes(), &value.to_be_bytes());
    }
    assert!(host.state.usbd.waiters().is_empty());
    assert!(host.state.usbd.handles().is_empty());
    assert_ne!(host.state.usbd.sync_term(), parked_hash);
    assert!(
        host.state.usbd.is_pristine(),
        "no handles and no readers reads as pristine again"
    );
}

#[test]
fn finalize_with_no_reader_is_immediate_and_a_reparked_thread_is_a_named_break() {
    let rt = rt();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, src());
    let handle = init(&mut host, &rt);
    assert_parked(&receive(&mut host, &rt, src(), handle, ARGS), handle);
    let d = receive(&mut host, &rt, src(), handle, ARGS_B);
    assert_eq!(code_of(&d), u64::from(errno::CELL_ESRCH));
    assert_eq!(
        host.observability()
            .invariant_break_sites
            .get("dispatch.usbd_reader_reparked"),
        Some(&1)
    );
    assert_eq!(host.state.usbd.waiters().len(), 1);

    let other = init(&mut host, &rt);
    let d = host.dispatch(Lv2Request::UsbdFinalize { handle: other }, src(), &rt);
    assert!(
        matches!(d, Lv2Dispatch::WakeAndReturn { .. }),
        "finalize on any handle wakes every reader, as the oracle does: {d:?}"
    );
    let d = host.dispatch(Lv2Request::UsbdFinalize { handle }, src(), &rt);
    assert_eq!(code_of(&d), 0, "no reader left: immediate");
}

#[test]
fn receive_event_refuses_a_null_or_unwritable_out_pointer_before_parking() {
    let rt = rt();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, src());
    let handle = init(&mut host, &rt);
    let d = receive(&mut host, &rt, src(), handle, [ARGS[0], 0, ARGS[2]]);
    assert_eq!(code_of(&d), u64::from(errno::CELL_EFAULT));
    let d = receive(
        &mut host,
        &rt,
        src(),
        handle,
        [ARGS[0], ARGS[1], 0xffff_fffc],
    );
    assert_eq!(code_of(&d), u64::from(errno::CELL_EFAULT));
    assert!(host.state.usbd.waiters().is_empty());
    let d = receive(&mut host, &rt, UnitId::new(9), handle, ARGS);
    assert_eq!(
        code_of(&d),
        u64::from(errno::CELL_ESRCH),
        "no thread record"
    );
}

#[test]
fn a_child_exit_purges_its_parked_reader() {
    let rt = rt();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, src());
    let child_pid = BOOT_PROCESS_PID + 0x100;
    let child_unit = UnitId::new(10);
    assert!(host.state.processes.insert_child(
        child_pid,
        ProcessEntry {
            ppid: BOOT_PROCESS_PID,
            authority_id: 0,
            control_flags1: 0,
            exit_status: None,
        }
    ));
    host.ppu_threads_mut()
        .create(child_unit, primary_attrs())
        .unwrap();
    host.bind_unit_process(child_unit, child_pid);
    let handle = init(&mut host, &rt);
    assert_parked(&receive(&mut host, &rt, child_unit, handle, ARGS_B), handle);
    assert_parked(&receive(&mut host, &rt, src(), handle, ARGS), handle);

    host.mark_process_exited(child_pid, 0);
    assert_eq!(
        host.observability().process_exit_waiter_purges.get("usbd"),
        Some(&1)
    );
    assert_eq!(host.state.usbd.waiters().len(), 1);
    assert_eq!(host.state.usbd.waiters()[0].out_ptrs, ARGS);

    let d = host.dispatch(Lv2Request::UsbdFinalize { handle }, src(), &rt);
    let Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } = d else {
        panic!("expected WakeAndReturn, got {d:?}");
    };
    assert_eq!(woken_unit_ids, vec![src()]);
}

#[test]
fn a_finalized_handle_is_refused_everywhere_and_counted_nowhere() {
    let rt = rt();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, src());
    let handle = init(&mut host, &rt);
    assert_eq!(
        code_of(&host.dispatch(Lv2Request::UsbdFinalize { handle }, src(), &rt)),
        0
    );
    let einval = u64::from(errno::CELL_EINVAL);
    for req in [
        Lv2Request::UsbdFinalize { handle },
        Lv2Request::UsbdGetDeviceList {
            handle,
            list_ptr: 0x6000,
            max_devices: 4,
        },
        Lv2Request::UsbdRegisterLdd {
            handle,
            product_ptr: PRODUCT_PTR,
            product_len: 4,
        },
        Lv2Request::UsbdOpenDefaultPipe { handle, device: 1 },
        Lv2Request::UsbdClosePipe { handle, pipe: 7 },
    ] {
        assert_eq!(code_of(&host.dispatch(req, src(), &rt)), einval);
    }
    assert_eq!(
        code_of(&receive(&mut host, &rt, src(), handle, ARGS)),
        einval,
        "a reader cannot park on a dropped handle"
    );
    assert!(host.state.usbd.waiters().is_empty());
    assert_eq!(host.obs.usbd_no_device_refusals, 0);
    assert!(host.state.usbd.is_pristine());

    let again = init(&mut host, &rt);
    assert_ne!(again, handle, "a dropped handle value is never re-minted");
}

#[test]
fn unregistering_a_product_no_register_recorded_is_esrch() {
    let rt = rt();
    let mut host = Lv2Host::new();
    let handle = init(&mut host, &rt);
    let unregister = |host: &mut Lv2Host, rt: &FakeRuntime| {
        code_of(&host.dispatch(
            Lv2Request::UsbdUnregisterLdd {
                handle,
                product_ptr: PRODUCT_PTR,
                product_len: 8,
            },
            src(),
            rt,
        ))
    };
    assert_eq!(unregister(&mut host, &rt), u64::from(errno::CELL_ESRCH));
    assert_eq!(
        code_of(&host.dispatch(
            Lv2Request::UsbdRegisterLdd {
                handle,
                product_ptr: PRODUCT_PTR,
                product_len: 8,
            },
            src(),
            &rt,
        )),
        0
    );
    assert_eq!(host.state.usbd.ldds().len(), 1);
    assert_eq!(unregister(&mut host, &rt), 0);
    assert!(host.state.usbd.ldds().is_empty());
    assert_eq!(unregister(&mut host, &rt), u64::from(errno::CELL_ESRCH));
}

#[test]
fn a_null_descriptor_pointer_is_a_pointer_refusal_not_a_no_device_one() {
    let rt = rt();
    let mut host = Lv2Host::new();
    let handle = init(&mut host, &rt);
    let d = host.dispatch(
        Lv2Request::UsbdGetDescriptor {
            handle,
            device: 1,
            desc_ptr: 0,
            desc_size: 0x40,
        },
        src(),
        &rt,
    );
    assert_eq!(code_of(&d), u64::from(errno::CELL_EINVAL));
    assert_eq!(host.obs.usbd_no_device_refusals, 0);
}
