//! The semaphore gates and the attribute-block gate of the four sync
//! creates answer the `lv2_semaphore_gates` probe as the reference
//! console does, word for word against its committed capture.
//!
//! The probe's steps C and F (the allocation, its page attributes and
//! its release) prepare the unmapped page and are not replayed: here the
//! end of the fake runtime's memory is that page boundary.

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::sync::{
    SYS_SYNC_FIFO, SYS_SYNC_NOT_ADAPTIVE, SYS_SYNC_NOT_PROCESS_SHARED, SYS_SYNC_NOT_RECURSIVE,
    SYS_SYNC_WAITER_SINGLE,
};

use super::*;
use crate::host::test_support::{
    console_payload, extract_write_u32, payload_u32, seed_primary_ppu, FakeRuntime,
    VALID_MUTEX_ATTR_PTR,
};
use crate::request::Lv2Request;

const PROBE: &str = "lv2_semaphore_gates";

/// The guest memory size; its end stands for the probe's page end.
const MEM_SIZE: u32 = 0x10000;

/// Where the semaphore block of steps A and B sits.
const SEM_ATTR_PTR: u32 = 0x800;

const ID_PTR: u32 = 0x100;

const VALUE_PTR: u32 = 0x200;

/// The four creates of steps D and E, in payload order.
#[derive(Clone, Copy)]
enum Create {
    Semaphore,
    Mutex,
    EventFlag,
    Cond,
}

impl Create {
    /// Distance from the block's start to its name.
    fn name_offset(self) -> u32 {
        match self {
            Create::Semaphore | Create::EventFlag => 24,
            Create::Mutex => 32,
            Create::Cond => 16,
        }
    }

    /// The words the create reads, by byte offset; the name is never
    /// written.
    fn words(self) -> Vec<(u32, u32)> {
        match self {
            Create::Semaphore => sem_words(),
            Create::Mutex => vec![
                (0, SYS_SYNC_FIFO),
                (4, SYS_SYNC_NOT_RECURSIVE),
                (8, SYS_SYNC_NOT_PROCESS_SHARED),
                (12, SYS_SYNC_NOT_ADAPTIVE),
            ],
            Create::EventFlag => vec![
                (0, SYS_SYNC_FIFO),
                (4, SYS_SYNC_NOT_PROCESS_SHARED),
                (20, SYS_SYNC_WAITER_SINGLE),
            ],
            Create::Cond => vec![(0, SYS_SYNC_NOT_PROCESS_SHARED)],
        }
    }

    fn request(self, attr_ptr: u32, helper_mutex: u32) -> Lv2Request {
        match self {
            Create::Semaphore => Lv2Request::SemaphoreCreate {
                id_ptr: ID_PTR,
                attr_ptr,
                initial: 0,
                max: 1,
            },
            Create::Mutex => Lv2Request::MutexCreate {
                id_ptr: ID_PTR,
                attr_ptr,
            },
            Create::EventFlag => Lv2Request::EventFlagCreate {
                id_ptr: ID_PTR,
                attr_ptr,
                init: 0,
            },
            Create::Cond => Lv2Request::CondCreate {
                id_ptr: ID_PTR,
                mutex_id: helper_mutex,
                attr_ptr,
            },
        }
    }
}

fn sem_words() -> Vec<(u32, u32)> {
    vec![(0, SYS_SYNC_FIFO), (4, SYS_SYNC_NOT_PROCESS_SHARED)]
}

/// A runtime holding the step A/B semaphore block, the valid mutex
/// block, and the words of one more block at `base`.
fn runtime(base: u32, words: &[(u32, u32)]) -> FakeRuntime {
    let mut mem = GuestMemory::new(MEM_SIZE as usize);
    let mut put = |at: u32, word: u32| {
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(u64::from(at)), 4).unwrap(),
            &word.to_be_bytes(),
        )
        .unwrap();
    };
    for (off, word) in sem_words() {
        put(SEM_ATTR_PTR + off, word);
    }
    for (off, word) in words {
        put(base + off, *word);
    }
    FakeRuntime::with_memory(mem).with_valid_mutex_attr()
}

fn code_of(d: &Lv2Dispatch) -> u32 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code as u32,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn written_id(d: &Lv2Dispatch) -> u32 {
    match d {
        Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

/// The return codes of the four creates with blocks whose name starts
/// at `name_at`. Each create runs on its own host and runtime, as the
/// probe destroys each object before the next create.
fn four_creates(name_at: u32) -> Vec<u32> {
    let src = UnitId::new(0);
    [
        Create::Semaphore,
        Create::Mutex,
        Create::EventFlag,
        Create::Cond,
    ]
    .into_iter()
    .map(|create| {
        let base = name_at - create.name_offset();
        let rt = runtime(base, &create.words());
        let mut host = Lv2Host::new();
        seed_primary_ppu(&mut host, src);
        let helper = written_id(&host.dispatch(
            Lv2Request::MutexCreate {
                id_ptr: ID_PTR,
                attr_ptr: VALID_MUTEX_ATTR_PTR,
            },
            src,
            &rt,
        ));
        code_of(&host.dispatch(create.request(base, helper), src, &rt))
    })
    .collect()
}

/// CellGov's words 2..=12 of the payload: steps A and B.
fn steps_a_b() -> Vec<u32> {
    let rt = runtime(0, &[]);
    let mut host = Lv2Host::new();
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let mut call = |req: Lv2Request| host.dispatch(req, src, &rt);
    let null_id = code_of(&call(Lv2Request::SemaphoreCreate {
        id_ptr: 0,
        attr_ptr: SEM_ATTR_PTR,
        initial: 5,
        max: 1,
    }));
    let null_attr = code_of(&call(Lv2Request::SemaphoreCreate {
        id_ptr: ID_PTR,
        attr_ptr: 0,
        initial: -1,
        max: 0,
    }));
    let stale = code_of(&call(Lv2Request::SemaphoreGetValue {
        id: 0xDEAD_BEEF,
        out_ptr: 0,
    }));
    let created = call(Lv2Request::SemaphoreCreate {
        id_ptr: ID_PTR,
        attr_ptr: SEM_ATTR_PTR,
        initial: 0,
        max: 1,
    });
    let create_rc = code_of(&created);
    let id = written_id(&created);
    let mut post_and_read = |val: i32| {
        let post = code_of(&call(Lv2Request::SemaphorePost { id, val }));
        let got = call(Lv2Request::SemaphoreGetValue {
            id,
            out_ptr: VALUE_PTR,
        });
        [post, code_of(&got), written_id(&got)]
    };
    let zero = post_and_read(0);
    let negative = post_and_read(-1);
    let destroy = code_of(&call(Lv2Request::SemaphoreDestroy { id }));
    let mut out = vec![null_id, null_attr, stale, create_rc];
    out.extend(zero);
    out.extend(negative);
    out.push(destroy);
    out
}

#[test]
fn the_semaphore_gates_answer_as_the_console_does() {
    let payload = console_payload(PROBE);
    let word = |i: usize| payload_u32(&payload, 4 * i);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");
    let console: Vec<u32> = (2..=12).map(word).collect();
    assert_eq!(steps_a_b(), console, "payload words 2..=12");
}

#[test]
fn the_sync_creates_gate_the_whole_attribute_block_as_the_console_does() {
    let payload = console_payload(PROBE);
    let word = |i: usize| payload_u32(&payload, 4 * i);
    assert_ne!(word(15), 0, "the page past the allocation is unmapped");
    let across: Vec<u32> = (16..=19).map(word).collect();
    let inside: Vec<u32> = (20..=23).map(word).collect();
    assert_eq!(four_creates(MEM_SIZE), across, "name across the end");
    assert_eq!(four_creates(MEM_SIZE - 64), inside, "block inside");
}
