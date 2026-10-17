//! The process queries held to the `lv2_process_queries` capture from
//! the reference console: get_status, getppid, get_ppu_guid, the object
//! counts around four creates, the ids those creates mint, and the ss
//! access control reads. For the difference CellGov does not close yet
//! -- the SPU thread group id's form -- a test holds the console's value
//! and refuses CellGov's.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::kernel_id;
use cellgov_ps3_abi::lv2::process::{
    BOOT_PROCESS_PID, SYS_EVENT_FLAG_OBJECT, SYS_EVENT_QUEUE_OBJECT, SYS_MUTEX_OBJECT,
    SYS_SEMAPHORE_OBJECT,
};

use super::*;
use crate::host::test_support::{
    console_payload, extract_write_u32, fake_runtime_with_valid_sync_attr, seed_primary_ppu,
    FakeRuntime, VALID_MUTEX_ATTR_PTR, VALID_SYNC_ATTR_PTR,
};
use crate::request::Lv2Request;
use crate::Lv2Dispatch;

const PROBE: &str = "lv2_process_queries";

/// Payload offsets, from the probe's main.c.
const OFF_QUERY: usize = 0;
const OFF_COUNT: usize = 80;
const COUNT_STRIDE: usize = 40;
const NCLASSES: usize = 22;
const OFF_CREATE: usize = OFF_COUNT + NCLASSES * COUNT_STRIDE;
const OFF_SS: usize = OFF_CREATE + 11 * 8 + 5 * 8;
const SS_STRIDE: usize = 40;

/// Where the guest's count, id and ss output slots sit.
const OUT_PTR: u32 = 0x100;

fn word(payload: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(payload[at..at + 8].try_into().expect("8 bytes"))
}

fn query(index: usize) -> u64 {
    word(&console_payload(PROBE), OFF_QUERY + index * 8)
}

/// The console's (rc before, count before, rc after, count after) for
/// `class`; a count is the 4 bytes the call wrote.
fn console_count(payload: &[u8], class: u32) -> (u64, u32, u64, u32) {
    let at = (0..NCLASSES)
        .map(|i| OFF_COUNT + i * COUNT_STRIDE)
        .find(|&at| word(payload, at) == u64::from(class))
        .unwrap_or_else(|| panic!("class {class:#x} not in the probe"));
    (
        word(payload, at + 8),
        (word(payload, at + 16) >> 32) as u32,
        word(payload, at + 24),
        (word(payload, at + 32) >> 32) as u32,
    )
}

fn host_and_runtime() -> (Lv2Host, FakeRuntime) {
    let rt = fake_runtime_with_valid_sync_attr(0x10000).with_valid_mutex_attr();
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));
    (host, rt)
}

fn immediate(host: &mut Lv2Host, rt: &FakeRuntime, request: Lv2Request) -> (u64, Vec<Effect>) {
    match host.dispatch(request, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code, effects } => (code, effects),
        other => panic!("{request:?}: expected Immediate, got {other:?}"),
    }
}

/// CellGov's (rc, count) for `class`; the count write is 4 bytes.
fn cellgov_count(host: &mut Lv2Host, rt: &FakeRuntime, class: u32) -> (u64, Option<u32>) {
    let (code, effects) = immediate(
        host,
        rt,
        Lv2Request::ProcessGetNumberOfObject {
            class_id: class,
            count_out_ptr: OUT_PTR,
        },
    );
    (code, effects.first().map(extract_write_u32))
}

/// Each create the probe ran on a sync class, as a CellGov request.
fn creates() -> [(u32, Lv2Request); 4] {
    [
        (
            SYS_MUTEX_OBJECT,
            Lv2Request::MutexCreate {
                id_ptr: OUT_PTR,
                attr_ptr: VALID_MUTEX_ATTR_PTR,
            },
        ),
        (
            SYS_SEMAPHORE_OBJECT,
            Lv2Request::SemaphoreCreate {
                id_ptr: OUT_PTR,
                attr_ptr: VALID_SYNC_ATTR_PTR,
                initial: 0,
                max: 1,
            },
        ),
        (
            SYS_EVENT_FLAG_OBJECT,
            Lv2Request::EventFlagCreate {
                id_ptr: OUT_PTR,
                attr_ptr: VALID_SYNC_ATTR_PTR,
                init: 0,
            },
        ),
        (
            SYS_EVENT_QUEUE_OBJECT,
            Lv2Request::EventQueueCreate {
                id_ptr: OUT_PTR,
                attr_ptr: 0,
                key: 0,
                size: 16,
            },
        ),
    ]
}

#[test]
fn get_status_reads_live_for_the_process_and_its_parent_and_esrch_otherwise() {
    let (mut host, rt) = host_and_runtime();
    let ppid = immediate(&mut host, &rt, Lv2Request::ProcessGetPpid).0;
    for (index, pid) in [(2, u64::from(BOOT_PROCESS_PID)), (3, ppid), (4, 0xDEAD)] {
        let got = immediate(
            &mut host,
            &rt,
            Lv2Request::ProcessGetStatus { pid: pid as u32 },
        )
        .0;
        assert_eq!(got, query(index), "get_status({pid:#x})");
    }
}

#[test]
fn getppid_and_the_ppu_guid_match_the_console() {
    let (mut host, rt) = host_and_runtime();
    assert_eq!(
        immediate(&mut host, &rt, Lv2Request::ProcessGetPpid).0,
        query(1)
    );
    assert_eq!(
        immediate(&mut host, &rt, Lv2Request::ProcessGetPpuGuid).0,
        query(5)
    );
}

#[test]
fn a_create_raises_its_class_count_by_one_as_on_the_console() {
    let payload = console_payload(PROBE);
    let (mut host, rt) = host_and_runtime();
    for (class, create) in creates() {
        let (rc_before, before, rc_after, after) = console_count(&payload, class);
        let (code, count) = cellgov_count(&mut host, &rt, class);
        assert_eq!(code, rc_before, "class {class:#x} rc before");
        let cellgov_before = count.expect("a 4-byte count");
        let (code, _) = immediate(&mut host, &rt, create);
        assert_eq!(code, 0, "class {class:#x} create");
        let (code, count) = cellgov_count(&mut host, &rt, class);
        assert_eq!(code, rc_after, "class {class:#x} rc after");
        assert_eq!(
            count.expect("a 4-byte count") - cellgov_before,
            after - before,
            "class {class:#x} count step"
        );
    }
}

#[test]
fn an_undefined_object_class_reads_einval_with_no_write_as_on_the_console() {
    let payload = console_payload(PROBE);
    let (mut host, rt) = host_and_runtime();
    for class in [0x25, 0x60, 0x01, 0xFF] {
        let (rc_before, before, rc_after, after) = console_count(&payload, class);
        assert_eq!(rc_after, rc_before, "class {class:#x}");
        assert_eq!(
            (before, after),
            (0xEEEE_EEEE, 0xEEEE_EEEE),
            "class {class:#x}"
        );
        assert_eq!(
            cellgov_count(&mut host, &rt, class),
            (rc_before, None),
            "class {class:#x}"
        );
    }
}

/// The console's process already held the slots below its first
/// create's when the probe ran. CellGov's boot does not create the
/// same objects, so the test fills those slots first.
#[test]
fn the_four_sync_creates_mint_the_consoles_ids() {
    let payload = console_payload(PROBE);
    let (mut host, rt) = host_and_runtime();
    let console_id = |i: usize| word(&payload, OFF_CREATE + 16 + i * 16) as u32;
    for _ in 0..kernel_id::slot_of(console_id(0)) {
        host.alloc_id(cellgov_event::UnitId::new(0), 0x23)
            .expect("a free slot");
    }
    for (i, (_, create)) in creates().into_iter().enumerate() {
        let (code, effects) = immediate(&mut host, &rt, create);
        assert_eq!(code, 0, "create {i}");
        assert_eq!(extract_write_u32(&effects[0]), console_id(i), "create {i}");
    }
}

/// An SPU thread group id takes no slot of the kernel-id table. The
/// console's has 0x04 in the top byte, 0x0100 in the low half, and a
/// middle byte that rises from one launch to the next across the
/// `lv2_process_stack_*` captures. CellGov counts group ids from 1.
#[test]
fn a_thread_group_id_has_a_form_of_its_own_on_the_console_and_counts_from_one_in_cellgov() {
    let payload = console_payload(PROBE);
    let console = word(&payload, OFF_CREATE + 16 + 4 * 16) as u32;
    assert_eq!(console >> 24, 0x04, "console id {console:#x}");
    assert_eq!(console & 0xFFFF, 0x0100, "console id {console:#x}");
    let (mut host, rt) = host_and_runtime();
    let (code, effects) = immediate(
        &mut host,
        &rt,
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: OUT_PTR,
            num_threads: 1,
            priority: 100,
            attr_ptr: 0,
        },
    );
    assert_eq!(code, 0);
    assert_eq!(
        extract_write_u32(&effects[0]),
        1,
        "CellGov's first group id"
    );
}

#[test]
fn the_ss_reads_match_the_console() {
    let payload = console_payload(PROBE);
    let (mut host, rt) = host_and_runtime();
    for (i, pkg_id) in [2u64, 0, 4].into_iter().enumerate() {
        let at = OFF_SS + i * SS_STRIDE;
        let (code, effects) = immediate(
            &mut host,
            &rt,
            Lv2Request::SsAccessControlEngine {
                pkg_id,
                a2: u64::from(OUT_PTR),
                a3: u64::from(OUT_PTR) + 0x10,
            },
        );
        assert_eq!(code, word(&payload, at), "pkg {pkg_id} rc");
        let console_a2 = word(&payload, at + 8);
        match effects.as_slice() {
            [] => assert_eq!(console_a2, u64::MAX / 0xFF * 0xEE, "pkg {pkg_id} a2"),
            [Effect::SharedWriteIntent { bytes, .. }] => {
                assert_eq!(bytes.bytes(), console_a2.to_be_bytes(), "pkg {pkg_id} a2")
            }
            other => panic!("pkg {pkg_id}: {other:?}"),
        }
    }
}
