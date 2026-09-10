//! Several blocking readers parked on the reply stream at once.

use super::*;
use crate::host::test_support::{seed_primary_ppu, FakeRuntime};
use crate::ppu_thread::PpuThreadAttrs;
use crate::request::Lv2Request;
use cellgov_mem::{GuestAddr, GuestMemory};

const ROOT: u32 = 0x4000_0000;
const PKT_PTR: u32 = 0x1000;
const FIRST_BUF: u32 = 0x3000;
const SECOND_BUF: u32 = 0x4000;
/// `AV_GET_HW_CONF` answers a 12-byte reply header plus 8 body bytes.
const HW_CONF_REPLY_LEN: u64 = 20;

fn first() -> UnitId {
    UnitId::new(0)
}

fn second() -> UnitId {
    UnitId::new(1)
}

/// Root host with two PPU threads and the packet staged in guest memory.
fn two_thread_host(packet: &[u8]) -> (Lv2Host, FakeRuntime) {
    let mut mem = GuestMemory::new(0x10000);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(PKT_PTR)), packet.len() as u64).unwrap(),
        packet,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mut host = Lv2Host::new();
    host.set_control_flags1(ROOT);
    seed_primary_ppu(&mut host, first());
    host.state
        .ppu_threads
        .create(
            second(),
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .expect("a second thread id");
    let d = host.dispatch(Lv2Request::UartInitialize, first(), &rt);
    assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
    (host, rt)
}

fn hw_conf_packet() -> Vec<u8> {
    let mut p = Vec::with_capacity(8);
    p.extend_from_slice(&av::PS3AV_VERSION.to_be_bytes());
    p.extend_from_slice(&4u16.to_be_bytes());
    p.extend_from_slice(&av::PS3AV_CID_AV_GET_HW_CONF.to_be_bytes());
    p
}

fn park(host: &mut Lv2Host, rt: &FakeRuntime, unit: UnitId, buf_ptr: u32, size: u64) {
    let d = host.dispatch(
        Lv2Request::UartReceive {
            buf_ptr,
            size,
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

fn send_hw_conf(host: &mut Lv2Host, rt: &FakeRuntime, len: usize) -> Lv2Dispatch {
    host.dispatch(
        Lv2Request::UartSend {
            buf_ptr: PKT_PTR,
            size: len as u64,
            mode: av::SYS_UART_MODE_NOT_BLOCKING_OP,
        },
        first(),
        rt,
    )
}

fn write_of(effect: &Effect) -> (u32, usize) {
    match effect {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            (range.start().raw() as u32, bytes.bytes().len())
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn a_second_blocking_reader_parks_behind_the_first_instead_of_ebusy() {
    let p = hw_conf_packet();
    let (mut host, rt) = two_thread_host(&p);
    park(&mut host, &rt, first(), FIRST_BUF, 0x100);
    park(&mut host, &rt, second(), SECOND_BUF, 0x100);
    let readers = host.state.uart.readers();
    assert_eq!(readers.len(), 2);
    assert_eq!(readers[0].buf_ptr, FIRST_BUF);
    assert_eq!(readers[1].buf_ptr, SECOND_BUF);
    assert_eq!(host.obs.uart_readers_queued, 1);
}

#[test]
fn one_send_serves_two_parked_readers_in_park_order() {
    let p = hw_conf_packet();
    let (mut host, rt) = two_thread_host(&p);
    park(&mut host, &rt, first(), FIRST_BUF, 12);
    park(&mut host, &rt, second(), SECOND_BUF, 0x100);
    let d = send_hw_conf(&mut host, &rt, p.len());
    let Lv2Dispatch::WakeAndReturn {
        code,
        woken_unit_ids,
        response_updates,
        effects,
    } = d
    else {
        panic!("expected WakeAndReturn, got {d:?}");
    };
    assert_eq!(code, p.len() as u64);
    assert_eq!(woken_unit_ids, vec![first(), second()]);
    assert_eq!(
        response_updates,
        vec![
            (first(), PendingResponse::ReturnCode { code: 12 }),
            (
                second(),
                PendingResponse::ReturnCode {
                    code: HW_CONF_REPLY_LEN - 12
                }
            ),
        ]
    );
    assert_eq!(write_of(&effects[0]), (FIRST_BUF, 12));
    assert_eq!(
        write_of(&effects[1]),
        (SECOND_BUF, (HW_CONF_REPLY_LEN - 12) as usize)
    );
    assert!(host.state.uart.readers().is_empty());
    assert!(host.state.uart.pending_bytes().is_empty());
}

#[test]
fn a_send_the_first_reader_drains_whole_leaves_the_second_parked() {
    let p = hw_conf_packet();
    let (mut host, rt) = two_thread_host(&p);
    park(&mut host, &rt, first(), FIRST_BUF, 0x100);
    park(&mut host, &rt, second(), SECOND_BUF, 0x100);
    let d = send_hw_conf(&mut host, &rt, p.len());
    let Lv2Dispatch::WakeAndReturn {
        woken_unit_ids,
        response_updates,
        effects,
        ..
    } = d
    else {
        panic!("expected WakeAndReturn, got {d:?}");
    };
    assert_eq!(woken_unit_ids, vec![first()]);
    assert_eq!(
        response_updates,
        vec![(
            first(),
            PendingResponse::ReturnCode {
                code: HW_CONF_REPLY_LEN
            }
        )]
    );
    assert_eq!(effects.len(), 1);
    assert_eq!(host.state.uart.readers().len(), 1);
    assert_eq!(host.state.uart.readers()[0].buf_ptr, SECOND_BUF);

    let d = send_hw_conf(&mut host, &rt, p.len());
    let Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } = d else {
        panic!("expected WakeAndReturn, got {d:?}");
    };
    assert_eq!(woken_unit_ids, vec![second()]);
    assert!(host.state.uart.readers().is_empty());
}

#[test]
fn a_thread_already_parked_cannot_park_a_second_record() {
    let p = hw_conf_packet();
    let (mut host, rt) = two_thread_host(&p);
    park(&mut host, &rt, first(), FIRST_BUF, 0x100);
    let d = host.dispatch(
        Lv2Request::UartReceive {
            buf_ptr: SECOND_BUF,
            size: 0x100,
            mode: av::SYS_UART_MODE_BLOCKING_BIG_OP,
        },
        first(),
        &rt,
    );
    assert!(
        matches!(d, Lv2Dispatch::Immediate { code, .. } if code == u64::from(errno::CELL_ESRCH)),
        "got {d:?}"
    );
    assert_eq!(host.state.uart.readers().len(), 1);
    assert_eq!(
        host.observability()
            .invariant_break_sites
            .get("dispatch.uart_reader_reparked"),
        Some(&1)
    );
    assert_eq!(host.obs.uart_readers_queued, 0);
}

#[test]
fn the_state_hash_distinguishes_one_parked_reader_from_two() {
    let p = hw_conf_packet();
    let (mut host, rt) = two_thread_host(&p);
    park(&mut host, &rt, first(), FIRST_BUF, 0x100);
    let one = host.state.uart.state_hash();
    park(&mut host, &rt, second(), SECOND_BUF, 0x100);
    let two = host.state.uart.state_hash();
    assert_ne!(one, two);
    let d = send_hw_conf(&mut host, &rt, p.len());
    assert!(matches!(d, Lv2Dispatch::WakeAndReturn { .. }));
    assert_ne!(host.state.uart.state_hash(), two);
}

/// A thread id the table never issued, parked at the front of the
/// queue by hand: the thread-table / primitive divergence
/// `resolve_wake_thread` guards against.
fn queue_a_reader_with_no_thread_record(host: &mut Lv2Host) {
    host.state.uart.readers.push_front(UartReader {
        thread: PpuThreadId::new(0xdead),
        buf_ptr: FIRST_BUF,
        size: 0x100,
    });
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "uart_deliver.reader")]
fn a_queued_reader_with_no_thread_record_is_a_named_break_in_debug() {
    let p = hw_conf_packet();
    let (mut host, rt) = two_thread_host(&p);
    park(&mut host, &rt, second(), SECOND_BUF, 0x100);
    queue_a_reader_with_no_thread_record(&mut host);
    let _ = send_hw_conf(&mut host, &rt, p.len());
}

#[cfg(not(debug_assertions))]
#[test]
fn a_queued_reader_with_no_thread_record_is_dropped_and_takes_no_bytes() {
    let p = hw_conf_packet();
    let (mut host, rt) = two_thread_host(&p);
    park(&mut host, &rt, second(), SECOND_BUF, 0x100);
    queue_a_reader_with_no_thread_record(&mut host);
    let d = send_hw_conf(&mut host, &rt, p.len());
    let Lv2Dispatch::WakeAndReturn {
        woken_unit_ids,
        response_updates,
        effects,
        ..
    } = d
    else {
        panic!("expected WakeAndReturn, got {d:?}");
    };
    assert_eq!(woken_unit_ids, vec![second()]);
    assert_eq!(
        response_updates,
        vec![(
            second(),
            PendingResponse::ReturnCode {
                code: HW_CONF_REPLY_LEN
            }
        )]
    );
    assert_eq!(effects.len(), 1);
    assert_eq!(
        write_of(&effects[0]),
        (SECOND_BUF, HW_CONF_REPLY_LEN as usize)
    );
    assert!(host.state.uart.readers().is_empty());
    assert!(host.state.uart.pending_bytes().is_empty());
    assert_eq!(
        host.observability()
            .invariant_break_sites
            .get("uart_deliver.reader"),
        Some(&1)
    );
}
