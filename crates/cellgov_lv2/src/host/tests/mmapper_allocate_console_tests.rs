//! The mmapper allocate calls held to the `lv2_mmapper_allocate`
//! capture from the reference console, which a game process took: the
//! probe's calls run through the classifier and dispatch in its order,
//! and every return code, and every free's code, is compared with the
//! captured word. Area bases and segment ids are CellGov's own.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::memory::ext_entry;
use cellgov_ps3_abi::lv2::syscall;

use crate::host::test_support::{console_payload, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::classify;
use crate::Lv2Dispatch;

const KEYLESS: u64 = 0xFFFF_0000_0000_0000;
const KEY_K: u64 = 0x0000_C0DE_1486_0001;
const FLAG_64K: u64 = 0x200;
const FLAG_1M: u64 = 0x400;
const AREA: u64 = 0x1000_0000;
const MIB: u64 = 0x10_0000;
const K64: u64 = 0x1_0000;
const OUT: u64 = 0x100;
const ENTRIES: u64 = 0x200;
const SKIP: u64 = 0xDEAD_D00D;

fn console() -> Vec<u64> {
    console_payload("lv2_mmapper_allocate")
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// A runtime whose 17 ext entries at `ENTRIES` all carry `entry_type`.
fn runtime_with_entries(entry_type: u64) -> FakeRuntime {
    let mut memory = GuestMemory::new(0x1000);
    for i in 0..17u64 {
        let at = ENTRIES + i * u64::from(ext_entry::LEN) + u64::from(ext_entry::TYPE_OFFSET);
        memory
            .apply_commit(
                ByteRange::new(GuestAddr::new(at), 8).expect("range"),
                &entry_type.to_be_bytes(),
            )
            .expect("commit");
    }
    FakeRuntime::with_memory(memory)
}

/// The probe's `sc`: the return code and the u32 of the first write.
fn sc(host: &mut Lv2Host, rt: &FakeRuntime, number: u64, args: [u64; 6]) -> (u64, Option<u64>) {
    let [a, b, c, d, e, f] = args;
    let request = classify(number, &[a, b, c, d, e, f, 0, 0]);
    match host.dispatch(request, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code, effects } => {
            let written = effects.iter().find_map(|e| match e {
                Effect::SharedWriteIntent { bytes, .. } => Some(u64::from(u32::from_be_bytes(
                    bytes.bytes()[..4].try_into().expect("a u32 write"),
                ))),
                _ => None,
            });
            (code, written)
        }
        other => panic!("syscall {number}: expected Immediate, got {other:?}"),
    }
}

struct Probe {
    host: Lv2Host,
    words: Vec<u64>,
}

impl Probe {
    /// A call whose rc is in `slot`; returns its out word when it
    /// succeeded and wrote one.
    fn rc(&mut self, slot: usize, rt: &FakeRuntime, number: u64, args: [u64; 6]) -> Option<u64> {
        let (code, written) = sc(&mut self.host, rt, number, args);
        assert_eq!(
            code, self.words[slot],
            "slot {slot}: syscall {number} {args:x?}"
        );
        if code == 0 {
            written
        } else {
            None
        }
    }

    /// A segment create in `slot` and its free (or SKIP) in `slot + 2`.
    fn segment(&mut self, slot: usize, rt: &FakeRuntime, number: u64, args: [u64; 6]) {
        let id = self.rc(slot, rt, number, args);
        match id {
            Some(id) => {
                self.rc(
                    slot + 2,
                    rt,
                    syscall::MMAPPER_FREE_SHARED_MEMORY,
                    [id, 0, 0, 0, 0, 0],
                );
            }
            None => assert_eq!(self.words[slot + 2], SKIP, "slot {}", slot + 2),
        }
    }
}

#[test]
fn mmapper_allocate_answers_as_on_the_console_for_a_game_process() {
    let rt = FakeRuntime::new(0x1000);
    let mut p = Probe {
        host: Lv2Host::new(),
        words: console(),
    };
    const ALLOC_ADDR: u64 = syscall::MMAPPER_ALLOCATE_ADDRESS;
    const SHM: u64 = syscall::MMAPPER_ALLOCATE_SHARED_MEMORY;
    const EXT: u64 = syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_EXT;
    const FROM: u64 = syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_FROM_CONTAINER;
    const FREE: u64 = syscall::MMAPPER_FREE_SHARED_MEMORY;

    // A. 330, each area freed with 331.
    for (slot, size, alignment, out) in [
        (0, 0x1000, AREA, OUT),
        (3, AREA, AREA, 0),
        (6, AREA, 0, OUT),
        (9, AREA, 0x3000, OUT),
        (12, AREA, AREA, OUT),
    ] {
        match p.rc(slot, &rt, ALLOC_ADDR, [size, FLAG_1M, alignment, out, 0, 0]) {
            Some(base) => {
                p.rc(
                    slot + 2,
                    &rt,
                    syscall::MMAPPER_FREE_ADDRESS,
                    [base, 0, 0, 0, 0, 0],
                );
            }
            None => assert_eq!(p.words[slot + 2], SKIP, "slot {}", slot + 2),
        }
    }

    // B. 332.
    p.segment(15, &rt, SHM, [KEYLESS, 0, FLAG_1M, OUT, 0, 0]);
    p.rc(18, &rt, SHM, [KEYLESS, MIB, FLAG_1M, 0, 0, 0]);
    p.segment(21, &rt, SHM, [KEYLESS, MIB, 0x300, OUT, 0, 0]);
    let id1 = p
        .rc(24, &rt, SHM, [0, MIB, FLAG_1M, OUT, 0, 0])
        .unwrap_or(0xFFFF_FFFF);
    let id2 = p
        .rc(26, &rt, SHM, [0, MIB, FLAG_1M, OUT, 0, 0])
        .unwrap_or(0xFFFF_FFFF);
    p.rc(28, &rt, FREE, [id1, 0, 0, 0, 0, 0]);
    p.rc(29, &rt, FREE, [id2, 0, 0, 0, 0, 0]);
    p.segment(30, &rt, SHM, [KEYLESS, K64, FLAG_64K, OUT, 0, 0]);

    // C. 339; the key K stays held across slot 36.
    let zeros = runtime_with_entries(0);
    let k = p.rc(33, &zeros, EXT, [KEY_K, K64, FLAG_64K, ENTRIES, 1, OUT]);
    let again = p.rc(36, &zeros, EXT, [KEY_K, K64, FLAG_64K, ENTRIES, 1, OUT]);
    assert!(again.is_none(), "slot 36 holds the key");
    assert_eq!(p.words[67], SKIP, "slot 67");
    p.rc(35, &zeros, FREE, [k.expect("slot 33"), 0, 0, 0, 0, 0]);
    for (slot, entry_type, count) in [
        (37, 1, 1),
        (40, 2, 1),
        (43, 3, 1),
        (46, 4, 1),
        (49, 5, 1),
        (52, 6, 1),
        (55, 0, 0),
        (58, 0, 16),
        (61, 0, 17),
    ] {
        let rt = runtime_with_entries(entry_type);
        p.segment(
            slot,
            &rt,
            EXT,
            [KEYLESS, K64, FLAG_64K, ENTRIES, count, OUT],
        );
    }

    // D. 362.
    p.segment(64, &rt, FROM, [KEYLESS, K64, 0xDEAD_BEEF, FLAG_64K, OUT, 0]);
}

#[test]
fn a_root_process_creates_a_segment_under_a_key_no_one_created() {
    let rt = FakeRuntime::new(0x1000);
    let mut host = Lv2Host::new();
    host.set_control_flags1(cellgov_ps3_abi::format::sce::CTRL_FLAGS1_ROOT_MASK);
    let shm = syscall::MMAPPER_ALLOCATE_SHARED_MEMORY;
    let (code, first) = sc(&mut host, &rt, shm, [0, MIB, FLAG_1M, OUT, 0, 0]);
    assert_eq!(code, 0);
    let (code, again) = sc(&mut host, &rt, shm, [0, MIB, FLAG_1M, OUT, 0, 0]);
    assert_eq!(code, 0);
    assert_eq!(
        first, again,
        "the second call attaches to the first segment"
    );
}

#[test]
fn a_free_removes_the_segment_and_the_key_it_registered() {
    let rt = runtime_with_entries(0);
    let mut host = Lv2Host::new();
    let ext = syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_EXT;
    let free = syscall::MMAPPER_FREE_SHARED_MEMORY;
    let (code, id) = sc(&mut host, &rt, ext, [KEY_K, K64, FLAG_64K, ENTRIES, 1, OUT]);
    assert_eq!(code, 0);
    let id = id.expect("an id");
    let (code, attached) = sc(
        &mut host,
        &rt,
        syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
        [KEY_K, K64, FLAG_64K, OUT, 0, 0],
    );
    assert_eq!(
        (code, attached),
        (0, Some(id)),
        "332 attaches to the 339 segment"
    );
    assert_eq!(sc(&mut host, &rt, free, [id, 0, 0, 0, 0, 0]).0, 0);
    let esrch = u64::from(cellgov_ps3_abi::lv2::errno::CELL_ESRCH);
    assert_eq!(sc(&mut host, &rt, free, [id, 0, 0, 0, 0, 0]).0, esrch);
    assert_eq!(
        sc(
            &mut host,
            &rt,
            syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            [KEY_K, K64, FLAG_64K, OUT, 0, 0]
        )
        .0,
        esrch,
        "the key went with the segment"
    );
}
