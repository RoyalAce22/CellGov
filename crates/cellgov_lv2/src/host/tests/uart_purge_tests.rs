//! A parked reader whose process exits is purged before the next send.

use std::collections::BTreeSet;

use super::*;
use crate::host::process::ProcessEntry;
use crate::host::test_support::{primary_attrs, seed_primary_ppu, FakeRuntime};
use crate::request::Lv2Request;
use cellgov_mem::{GuestAddr, GuestMemory};
use cellgov_ps3_abi::sys_process::BOOT_PROCESS_PID;

const ROOT: u32 = 0x4000_0000;
const PKT_PTR: u32 = 0x1000;
const CHILD_BUF: u32 = 0x3000;
const BOOT_BUF: u32 = 0x4000;
const CHILD_PID: u32 = BOOT_PROCESS_PID + 0x100;
const HW_CONF_REPLY_LEN: u64 = 20;

fn boot_unit() -> UnitId {
    UnitId::new(0)
}

fn child_unit() -> UnitId {
    UnitId::new(10)
}

/// Root host with the boot thread and one child-process thread; the
/// child's reader parks first so it sits at the queue front.
fn host_with_child_reader_in_front() -> (Lv2Host, FakeRuntime, PpuThreadId) {
    let p = hw_conf_packet();
    let mut mem = GuestMemory::new(0x10000);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(PKT_PTR)), p.len() as u64).unwrap(),
        &p,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mut host = Lv2Host::new();
    host.set_control_flags1(ROOT);
    seed_primary_ppu(&mut host, boot_unit());
    assert!(host.state.processes.insert_child(
        CHILD_PID,
        ProcessEntry {
            ppid: BOOT_PROCESS_PID,
            authority_id: 0,
            control_flags1: 0,
            exit_status: None,
        }
    ));
    let child_thread = host
        .ppu_threads_mut()
        .create(child_unit(), primary_attrs())
        .unwrap();
    host.bind_unit_process(child_unit(), CHILD_PID);
    let d = host.dispatch(Lv2Request::UartInitialize, boot_unit(), &rt);
    assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
    park(&mut host, &rt, child_unit(), CHILD_BUF);
    park(&mut host, &rt, boot_unit(), BOOT_BUF);
    assert_eq!(host.state.uart.readers().len(), 2);
    assert_eq!(host.state.uart.readers()[0].thread, child_thread);
    (host, rt, child_thread)
}

fn hw_conf_packet() -> Vec<u8> {
    let mut p = Vec::with_capacity(8);
    p.extend_from_slice(&av::PS3AV_VERSION.to_be_bytes());
    p.extend_from_slice(&4u16.to_be_bytes());
    p.extend_from_slice(&av::PS3AV_CID_AV_GET_HW_CONF.to_be_bytes());
    p
}

fn park(host: &mut Lv2Host, rt: &FakeRuntime, unit: UnitId, buf_ptr: u32) {
    let d = host.dispatch(
        Lv2Request::UartReceive {
            buf_ptr,
            size: 0x100,
            mode: av::SYS_UART_MODE_BLOCKING_BIG_OP,
        },
        unit,
        rt,
    );
    assert!(
        matches!(
            d,
            Lv2Dispatch::Block {
                reason: Lv2BlockReason::Uart,
                ..
            }
        ),
        "{unit:?} should park, got {d:?}"
    );
}

fn send_hw_conf(host: &mut Lv2Host, rt: &FakeRuntime) -> Lv2Dispatch {
    let len = hw_conf_packet().len() as u64;
    host.dispatch(
        Lv2Request::UartSend {
            buf_ptr: PKT_PTR,
            size: len,
            mode: av::SYS_UART_MODE_NOT_BLOCKING_OP,
        },
        boot_unit(),
        rt,
    )
}

#[test]
fn a_child_exit_purges_its_reader_and_the_next_send_reaches_the_boot_reader() {
    let (mut host, rt, child_thread) = host_with_child_reader_in_front();
    host.mark_process_exited(CHILD_PID, 0);
    assert_eq!(
        host.observability().process_exit_waiter_purges.get("uart"),
        Some(&1)
    );
    assert_eq!(host.state.uart.readers().len(), 1);
    assert_eq!(host.state.uart.readers()[0].buf_ptr, BOOT_BUF);
    let dead: BTreeSet<_> = [child_thread].into_iter().collect();
    assert!(host.state.uart.purge_readers_of(&dead).is_empty());

    let d = send_hw_conf(&mut host, &rt);
    let Lv2Dispatch::WakeAndReturn {
        woken_unit_ids,
        response_updates,
        effects,
        ..
    } = d
    else {
        panic!("expected WakeAndReturn, got {d:?}");
    };
    assert_eq!(woken_unit_ids, vec![boot_unit()]);
    assert_eq!(
        response_updates,
        vec![(
            boot_unit(),
            PendingResponse::ReturnCode {
                code: HW_CONF_REPLY_LEN
            }
        )]
    );
    let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] else {
        panic!("expected a write");
    };
    assert_eq!(range.start().raw(), u64::from(BOOT_BUF));
    assert_eq!(bytes.bytes().len(), HW_CONF_REPLY_LEN as usize);
    assert!(host.state.uart.readers().is_empty());
    assert!(host.state.uart.pending_bytes().is_empty());
}

#[test]
fn without_the_purge_the_dead_reader_would_have_taken_the_bytes() {
    let (mut host, rt, _child_thread) = host_with_child_reader_in_front();
    let d = send_hw_conf(&mut host, &rt);
    let Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } = d else {
        panic!("expected WakeAndReturn, got {d:?}");
    };
    assert_eq!(
        woken_unit_ids,
        vec![child_unit()],
        "the front reader takes the whole 20-byte reply; the boot reader stays parked"
    );
    assert_eq!(host.state.uart.readers().len(), 1);
    assert_eq!(host.state.uart.readers()[0].buf_ptr, BOOT_BUF);
}

#[test]
fn a_purge_keeps_survivors_in_park_order() {
    let mut uart = UartState::new();
    let live_a = PpuThreadId::new(1);
    let dead = PpuThreadId::new(2);
    let live_b = PpuThreadId::new(3);
    for (thread, buf_ptr) in [(live_a, 0x10), (dead, 0x20), (live_b, 0x30)] {
        uart.readers.push_back(UartReader {
            thread,
            buf_ptr,
            size: 8,
        });
    }
    let purged = uart.purge_readers_of(&[dead].into_iter().collect());
    assert_eq!(purged.len(), 1);
    assert_eq!(purged[0].buf_ptr, 0x20);
    let survivors: Vec<u32> = uart.readers().iter().map(|r| r.buf_ptr).collect();
    assert_eq!(survivors, vec![0x10, 0x30]);
    assert!(uart
        .purge_readers_of(&[PpuThreadId::new(9)].into_iter().collect())
        .is_empty());
}
