//! The `CellError` codes held to the `lv2_error_codes_sync` and
//! `lv2_error_codes_fs` captures from the reference console: each code
//! the console returned is the table's value, and r3 carries it
//! sign-extended. The tests drive each sync call that CellGov answers at
//! once through dispatch and compare the answers word for word.

use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::errno::{self, Lv2ErrCode};
use cellgov_ps3_abi::lv2::sync::mutex_attribute;

use super::*;
use crate::host::test_support::{
    console_payload, extract_write_u32, seed_primary_ppu, valid_mutex_attr_bytes,
    valid_sync_attr_bytes, FakeRuntime, VALID_MUTEX_ATTR_PTR, VALID_SYNC_ATTR_PTR,
};
use crate::request::Lv2Request;
use crate::Lv2Dispatch;

/// The sync probe's cases, in payload order.
const SYNC_CASES: [Lv2ErrCode; 12] = [
    errno::CELL_EINVAL,
    errno::CELL_ESRCH,
    errno::CELL_EFAULT,
    errno::CELL_EDEADLK,
    errno::CELL_EDEADLK,
    errno::CELL_EBUSY,
    errno::CELL_EPERM,
    errno::CELL_ETIMEDOUT,
    errno::CELL_EBUSY,
    errno::CELL_EBUSY,
    errno::CELL_ENOMEM,
    errno::CELL_EALIGN,
];

/// The sync probe's setup and cleanup calls, after the cases.
const SYNC_SETUP: usize = 6;

/// The file-system probe's cases, in payload order.
const FS_CASES: [Lv2ErrCode; 8] = [
    errno::CELL_ENOENT,
    errno::CELL_EEXIST,
    errno::CELL_EISDIR,
    errno::CELL_EEXIST,
    errno::CELL_ENOTDIR,
    errno::CELL_ENOTEMPTY,
    errno::CELL_EBADF,
    errno::CELL_ENAMETOOLONG,
];

/// The file-system probe's setup and cleanup calls, after the cases.
const FS_SETUP: usize = 5;

fn words(payload: &[u8]) -> Vec<u64> {
    payload
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

fn assert_codes(probe: &str, cases: &[Lv2ErrCode], setup: usize) {
    let words = words(&console_payload(probe));
    assert_eq!(words.len(), cases.len() + setup, "{probe}: payload words");
    for (i, (&got, code)) in words.iter().zip(cases).enumerate() {
        assert_eq!(got as u32, code.code, "{probe} case {i}: {}", code.symbol);
        assert_eq!(got >> 32, 0xFFFF_FFFF, "{probe} case {i}: r3 sign-extended");
        assert_eq!(
            got,
            u64::from(*code),
            "{probe} case {i}: r3 as CellGov sets it"
        );
    }
    for (i, &got) in words[cases.len()..].iter().enumerate() {
        assert_eq!(got, 0, "{probe} setup call {i}");
    }
}

#[test]
fn sync_and_memory_codes_are_the_tables_values_sign_extended() {
    assert_codes("lv2_error_codes_sync", &SYNC_CASES, SYNC_SETUP);
}

#[test]
fn file_system_codes_are_the_tables_values_sign_extended() {
    assert_codes("lv2_error_codes_fs", &FS_CASES, FS_SETUP);
}

/// Where the protocol-0 mutex attribute block sits.
const BAD_MUTEX_ATTR_PTR: u32 = 0x200;

fn immediate(host: &mut Lv2Host, request: Lv2Request, rt: &FakeRuntime) -> u64 {
    match host.dispatch(request, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code, .. } => code,
        other => panic!("{request:?}: expected Immediate, got {other:?}"),
    }
}

/// CellGov's r3 for each sync case the probe ran, by payload index; the
/// timed semaphore wait blocks and is left out.
fn cellgov_sync_answers() -> Vec<(usize, u64)> {
    let mut bad_attr = valid_mutex_attr_bytes();
    let protocol = mutex_attribute::PROTOCOL_OFFSET;
    bad_attr[protocol..protocol + 4].copy_from_slice(&0u32.to_be_bytes());
    let mut mem = GuestMemory::new(0x10000);
    for (at, bytes) in [
        (VALID_SYNC_ATTR_PTR, &valid_sync_attr_bytes()[..]),
        (BAD_MUTEX_ATTR_PTR, &bad_attr[..]),
    ] {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(u64::from(at)), bytes.len() as u64).unwrap(),
            bytes,
        )
        .unwrap();
    }
    let rt = FakeRuntime::with_memory(mem).with_valid_mutex_attr();
    let mut host = Lv2Host::new();
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let mut out = Vec::new();

    out.push((
        0,
        immediate(
            &mut host,
            Lv2Request::MutexCreate {
                id_ptr: 0x100,
                attr_ptr: BAD_MUTEX_ATTR_PTR,
            },
            &rt,
        ),
    ));
    out.push((
        1,
        immediate(&mut host, Lv2Request::MutexDestroy { mutex_id: 0 }, &rt),
    ));
    out.push((
        2,
        immediate(
            &mut host,
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 0,
                max: 1,
            },
            &rt,
        ),
    ));

    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_MUTEX_ATTR_PTR,
        },
        src,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = &created else {
        panic!("mutex create: {created:?}");
    };
    let mutex_id = extract_write_u32(&effects[0]);
    let lock = Lv2Request::MutexLock {
        mutex_id,
        timeout: 0,
    };
    assert_eq!(immediate(&mut host, lock, &rt), 0, "first lock");
    out.push((3, immediate(&mut host, lock, &rt)));
    out.push((
        4,
        immediate(&mut host, Lv2Request::MutexTryLock { mutex_id }, &rt),
    ));
    out.push((
        5,
        immediate(&mut host, Lv2Request::MutexDestroy { mutex_id }, &rt),
    ));
    let unlock = Lv2Request::MutexUnlock { mutex_id };
    assert_eq!(immediate(&mut host, unlock, &rt), 0, "unlock");
    out.push((6, immediate(&mut host, unlock, &rt)));

    let created = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
            initial: 0,
            max: 1,
        },
        src,
        &rt,
    );
    let Lv2Dispatch::Immediate { code: 0, effects } = &created else {
        panic!("semaphore create: {created:?}");
    };
    let id = extract_write_u32(&effects[0]);
    out.push((
        8,
        immediate(&mut host, Lv2Request::SemaphoreTryWait { id }, &rt),
    ));
    out.push((
        9,
        immediate(&mut host, Lv2Request::SemaphorePost { id, val: 2 }, &rt),
    ));

    for (index, size, flags) in [(10, 0x1000_0000, 0x400), (11, 0x10001, 0x200)] {
        out.push((
            index,
            immediate(
                &mut host,
                Lv2Request::MemoryAllocate {
                    size,
                    flags,
                    alloc_addr_ptr: 0x100,
                },
                &rt,
            ),
        ));
    }
    out
}

#[test]
fn sync_and_memory_refusals_match_the_console_word_for_word() {
    let console = words(&console_payload("lv2_error_codes_sync"));
    let answers = cellgov_sync_answers();
    assert_eq!(answers.len(), 11, "every case but the timed wait");
    let wrong: Vec<String> = answers
        .into_iter()
        .filter(|&(index, got)| got != console[index])
        .map(|(index, got)| {
            format!(
                "case {index} ({}): CellGov {got:#x}, console {:#x}",
                SYNC_CASES[index].symbol, console[index]
            )
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The `lv2_memory_allocate_order` probe's `(size, flags)`, in payload
/// order.
const ALLOCATE_ORDER_CASES: [(u64, u64); 3] = [
    (0x1000_0001, 0x200),
    (0x1000_0001, 0x400),
    (0x1000_0000, 0x200),
];

#[test]
fn an_allocation_too_large_and_unaligned_is_refused_for_size_first() {
    let console = words(&console_payload("lv2_memory_allocate_order"));
    assert_eq!(console.len(), ALLOCATE_ORDER_CASES.len());
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    for (i, &(size, flags)) in ALLOCATE_ORDER_CASES.iter().enumerate() {
        let got = immediate(
            &mut host,
            Lv2Request::MemoryAllocate {
                size,
                flags,
                alloc_addr_ptr: 0x100,
            },
            &rt,
        );
        assert_eq!(got, console[i], "case {i}: size {size:#x} flags {flags:#x}");
    }
}
