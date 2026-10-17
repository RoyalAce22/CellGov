//! The sync creates answer the `lv2_sync_attr_values` probe's 84
//! attribute blocks as the reference console does: each enumerant swept
//! one field at a time, three swapped fields that pin the block offsets,
//! and each block ending at the end of mapped memory and 4 bytes past
//! it. The same return code, id store and destroy answer, read from the
//! committed capture.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::sync::{
    cond_attribute, event_flag_attribute, mutex_attribute, semaphore_attribute, SYS_SYNC_FIFO,
    SYS_SYNC_NOT_ADAPTIVE, SYS_SYNC_NOT_PROCESS_SHARED, SYS_SYNC_NOT_RECURSIVE,
    SYS_SYNC_PROCESS_SHARED, SYS_SYNC_WAITER_SINGLE,
};
use cellgov_ps3_abi::lv2::syscall::{
    COND_CREATE, COND_DESTROY, EVENT_FLAG_CREATE, EVENT_FLAG_DESTROY, EVENT_PORT_CREATE,
    EVENT_PORT_DESTROY, MUTEX_CREATE, MUTEX_DESTROY, SEMAPHORE_CREATE, SEMAPHORE_DESTROY,
};

use super::*;
use crate::host::test_support::{console_payload, payload_u32, seed_primary_ppu, FakeRuntime};
use crate::request::classify;

const PROBE: &str = "lv2_sync_attr_values";
const NOT_RUN: u32 = 0xFFFF_FFFF;
const ID_PTR: u64 = 0x100;
const BLOCK: u64 = 0x800;
/// The end of CellGov's guest memory: the probe's allocation end.
const END: u64 = 0x2000;
/// The probe's process-shared key high word; the low word is the case.
const KEY_HI: u32 = 0x4347_0000;
const PSHARED: [u32; 6] = [0, 0x80, 0x100, 0x200, 0x300, 0x400];
const RECURSIVE: [u32; 6] = [0, 0x8, 0x10, 0x20, 0x30, 0x40];
const TYPE: [u32; 6] = [0, 0x8000, 0x1_0000, 0x2_0000, 0x3_0000, 0x4_0000];

/// One create: the syscall, its destroy, the block's words, where the
/// block sits (`None` for the event port, whose words are its
/// arguments), and how many of the words are mapped.
struct Create {
    number: u64,
    destroy: u64,
    words: Vec<u32>,
    at: Option<u64>,
    mapped: usize,
}

/// The key and flags words of case `case`: its own key and flags 1 when
/// `pshared` is process-shared, else zeros.
fn key(case: usize, pshared: u32) -> [u32; 3] {
    if pshared == SYS_SYNC_PROCESS_SHARED {
        [KEY_HI, case as u32, 1]
    } else {
        [0; 3]
    }
}

fn sem(case: usize, protocol: u32, pshared: u32) -> Vec<u32> {
    let [hi, lo, flags] = key(case, pshared);
    vec![protocol, pshared, hi, lo, flags, 0, 0, 0]
}

fn mutex(case: usize, protocol: u32, recursive: u32, pshared: u32, adaptive: u32) -> Vec<u32> {
    let [hi, lo, flags] = key(case, pshared);
    vec![
        protocol, recursive, pshared, adaptive, hi, lo, flags, 0, 0, 0,
    ]
}

fn flag(case: usize, protocol: u32, pshared: u32, flags_word: u32, ty: u32) -> Vec<u32> {
    let [hi, lo, flags] = key(case, pshared);
    vec![protocol, pshared, hi, lo, flags | flags_word, ty, 0, 0]
}

fn cond(case: usize, pshared: u32) -> Vec<u32> {
    let [hi, lo, flags] = key(case, pshared);
    vec![pshared, flags, hi, lo, 0, 0]
}

/// The probe's 84 cases, in payload order.
fn cases() -> Vec<Create> {
    let mut out: Vec<Create> = Vec::new();
    let push = |out: &mut Vec<Create>, number, destroy, words: Vec<u32>| {
        let mapped = words.len();
        out.push(Create {
            number,
            destroy,
            words,
            at: Some(BLOCK),
            mapped,
        });
    };
    for p in 0..9 {
        let k = out.len();
        push(
            &mut out,
            SEMAPHORE_CREATE,
            SEMAPHORE_DESTROY,
            sem(k, p, SYS_SYNC_NOT_PROCESS_SHARED),
        );
    }
    for s in PSHARED {
        let k = out.len();
        push(
            &mut out,
            SEMAPHORE_CREATE,
            SEMAPHORE_DESTROY,
            sem(k, SYS_SYNC_FIFO, s),
        );
    }
    let m = |k, p, r, s, a| mutex(k, p, r, s, a);
    for p in 0..9 {
        let k = out.len();
        let w = m(
            k,
            p,
            SYS_SYNC_NOT_RECURSIVE,
            SYS_SYNC_NOT_PROCESS_SHARED,
            SYS_SYNC_NOT_ADAPTIVE,
        );
        push(&mut out, MUTEX_CREATE, MUTEX_DESTROY, w);
    }
    for r in RECURSIVE {
        let k = out.len();
        let w = m(
            k,
            SYS_SYNC_FIFO,
            r,
            SYS_SYNC_NOT_PROCESS_SHARED,
            SYS_SYNC_NOT_ADAPTIVE,
        );
        push(&mut out, MUTEX_CREATE, MUTEX_DESTROY, w);
    }
    for s in PSHARED {
        let k = out.len();
        let w = m(
            k,
            SYS_SYNC_FIFO,
            SYS_SYNC_NOT_RECURSIVE,
            s,
            SYS_SYNC_NOT_ADAPTIVE,
        );
        push(&mut out, MUTEX_CREATE, MUTEX_DESTROY, w);
    }
    for p in 0..9 {
        let k = out.len();
        let w = flag(k, p, SYS_SYNC_NOT_PROCESS_SHARED, 0, SYS_SYNC_WAITER_SINGLE);
        push(&mut out, EVENT_FLAG_CREATE, EVENT_FLAG_DESTROY, w);
    }
    for t in TYPE {
        let k = out.len();
        let w = flag(k, SYS_SYNC_FIFO, SYS_SYNC_NOT_PROCESS_SHARED, 0, t);
        push(&mut out, EVENT_FLAG_CREATE, EVENT_FLAG_DESTROY, w);
    }
    for s in PSHARED {
        let k = out.len();
        let w = flag(k, SYS_SYNC_FIFO, s, 0, SYS_SYNC_WAITER_SINGLE);
        push(&mut out, EVENT_FLAG_CREATE, EVENT_FLAG_DESTROY, w);
    }
    for s in PSHARED {
        let k = out.len();
        push(&mut out, COND_CREATE, COND_DESTROY, cond(k, s));
    }
    // The event ports: the type, then the name, as the two words.
    for t in 0..9 {
        out.push(Create {
            number: EVENT_PORT_CREATE,
            destroy: EVENT_PORT_DESTROY,
            words: vec![t, 0, 0],
            at: None,
            mapped: 0,
        });
    }
    let k = out.len() as u32;
    out.push(Create {
        number: EVENT_PORT_CREATE,
        destroy: EVENT_PORT_DESTROY,
        words: vec![3, KEY_HI, k],
        at: None,
        mapped: 0,
    });
    // The swaps.
    let k = out.len();
    let w = m(
        k,
        SYS_SYNC_NOT_RECURSIVE,
        SYS_SYNC_FIFO,
        SYS_SYNC_NOT_PROCESS_SHARED,
        SYS_SYNC_NOT_ADAPTIVE,
    );
    push(&mut out, MUTEX_CREATE, MUTEX_DESTROY, w);
    let k = out.len();
    let w = m(
        k,
        SYS_SYNC_FIFO,
        SYS_SYNC_NOT_RECURSIVE,
        SYS_SYNC_NOT_ADAPTIVE,
        SYS_SYNC_NOT_PROCESS_SHARED,
    );
    push(&mut out, MUTEX_CREATE, MUTEX_DESTROY, w);
    let k = out.len();
    let w = flag(
        k,
        SYS_SYNC_FIFO,
        SYS_SYNC_NOT_PROCESS_SHARED,
        SYS_SYNC_WAITER_SINGLE,
        0,
    );
    push(&mut out, EVENT_FLAG_CREATE, EVENT_FLAG_DESTROY, w);
    // The straddles: each block ends at END, then 4 bytes past it.
    for past in [0u64, 4] {
        for (number, destroy, size) in [
            (
                SEMAPHORE_CREATE,
                SEMAPHORE_DESTROY,
                semaphore_attribute::SIZE,
            ),
            (MUTEX_CREATE, MUTEX_DESTROY, mutex_attribute::SIZE),
            (
                EVENT_FLAG_CREATE,
                EVENT_FLAG_DESTROY,
                event_flag_attribute::SIZE,
            ),
            (COND_CREATE, COND_DESTROY, cond_attribute::SIZE),
        ] {
            let k = out.len();
            let words = match number {
                SEMAPHORE_CREATE => sem(k, SYS_SYNC_FIFO, SYS_SYNC_NOT_PROCESS_SHARED),
                MUTEX_CREATE => m(
                    k,
                    SYS_SYNC_FIFO,
                    SYS_SYNC_NOT_RECURSIVE,
                    SYS_SYNC_NOT_PROCESS_SHARED,
                    SYS_SYNC_NOT_ADAPTIVE,
                ),
                EVENT_FLAG_CREATE => flag(
                    k,
                    SYS_SYNC_FIFO,
                    SYS_SYNC_NOT_PROCESS_SHARED,
                    0,
                    SYS_SYNC_WAITER_SINGLE,
                ),
                _ => cond(k, SYS_SYNC_NOT_PROCESS_SHARED),
            };
            assert_eq!(4 * words.len(), size as usize, "the block is SIZE bytes");
            let mapped = words.len() - (past / 4) as usize;
            out.push(Create {
                number,
                destroy,
                words,
                at: Some(END - u64::from(size) + past),
                mapped,
            });
        }
    }
    out
}

/// The answer to `number` with `args`, and the id it stored, if any.
fn call(host: &mut Lv2Host, rt: &FakeRuntime, number: u64, args: [u64; 4]) -> (u32, Option<u32>) {
    let mut words = [0; 8];
    words[..4].copy_from_slice(&args);
    match host.dispatch(classify(number, &words), UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code, effects } => {
            let id = effects.iter().find_map(|effect| match effect {
                cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. }
                    if range.start().raw() == ID_PTR =>
                {
                    Some(u32::from_be_bytes(
                        bytes.bytes().try_into().expect("4 bytes"),
                    ))
                }
                _ => None,
            });
            (code as u32, id)
        }
        other => panic!("call {number}: unexpected dispatch {other:?}"),
    }
}

#[test]
fn the_sync_creates_answer_each_attribute_block_as_the_console_does() {
    let payload = console_payload(PROBE);
    let cases = cases();
    assert_eq!(payload.len(), 16 + 16 * cases.len());
    assert_eq!(payload_u32(&payload, 0), 0, "no step of the probe failed");

    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));
    let helper_rt = FakeRuntime::new(END as usize).with_valid_mutex_attr();
    let (code, helper) = call(
        &mut host,
        &helper_rt,
        MUTEX_CREATE,
        [
            ID_PTR,
            u64::from(crate::host::test_support::VALID_MUTEX_ATTR_PTR),
            0,
            0,
        ],
    );
    assert_eq!(code, 0, "the helper mutex");
    let helper = u64::from(helper.expect("the helper's id"));

    let mut differing = Vec::new();
    for (k, case) in cases.iter().enumerate() {
        let mut mem = GuestMemory::new(END as usize);
        if let Some(at) = case.at {
            let bytes: Vec<u8> = case.words[..case.mapped]
                .iter()
                .flat_map(|w| w.to_be_bytes())
                .collect();
            mem.apply_commit(
                ByteRange::new(GuestAddr::new(at), bytes.len() as u64).expect("in range"),
                &bytes,
            )
            .expect("mapped");
        }
        let rt = FakeRuntime::with_memory(mem);
        let args = match (case.number, case.at) {
            (SEMAPHORE_CREATE, Some(at)) => [ID_PTR, at, 0, 1],
            (MUTEX_CREATE, Some(at)) => [ID_PTR, at, 0, 0],
            (EVENT_FLAG_CREATE, Some(at)) => [ID_PTR, at, 0, 0],
            (COND_CREATE, Some(at)) => [ID_PTR, helper, at, 0],
            _ => [
                ID_PTR,
                u64::from(case.words[0]),
                u64::from(case.words[1]) << 32 | u64::from(case.words[2]),
                0,
            ],
        };
        let (code, id) = call(&mut host, &rt, case.number, args);
        let destroy = match id {
            Some(id) if code == 0 => call(&mut host, &rt, case.destroy, [u64::from(id), 0, 0, 0]).0,
            _ => NOT_RUN,
        };
        let at = 16 + 16 * k;
        let cellgov = [code, u32::from(id.is_some()), destroy];
        let console = [
            payload_u32(&payload, at),
            payload_u32(&payload, at + 4),
            payload_u32(&payload, at + 8),
        ];
        if cellgov != console {
            differing.push(format!(
                "case {k}: cellgov {cellgov:x?}, console {console:x?}"
            ));
        }
    }
    assert!(
        differing.is_empty(),
        "[create rc, id written, destroy rc] differs:\n{}",
        differing.join("\n")
    );
}
