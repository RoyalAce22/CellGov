//! The mmapper map and search calls held to the `lv2_mmapper_map` (an
//! area of 1 MiB pages) and `lv2_mmapper_map_64k` (an area of 64 KiB
//! pages) captures from the reference console, which a game process
//! took. Each map or search the probe made on a clean area runs on a
//! fresh host after the probe's setup, and its return code is compared
//! with the captured word; a search's address is compared as an offset
//! from the area base.
//!
//! The console unmapped each window with 335 before its next step.
//! CellGov does not route 335, so a window it maps stays mapped; the
//! fresh host per step stands in for that unmap. The answers that
//! depend on 335, or on a mapping still live, are the named divergences
//! at the end of this file.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::syscall;

use crate::host::test_support::{console_payload, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::classify;
use crate::Lv2Dispatch;

const KEYLESS: u64 = 0xFFFF_0000_0000_0000;
const FLAG_64K: u64 = 0x200;
const FLAG_1M: u64 = 0x400;
const PROT_RW: u64 = 0x40000;
const PROT_RO: u64 = 0x80000;
const AREA: u64 = 0x1000_0000;
const MIB: u64 = 0x10_0000;
const K64: u64 = 0x1_0000;
const BAD_ID: u64 = 0xDEAD_BEEF;
const OUT: u64 = 0x100;
const SKIP: u64 = 0xDEAD_D00D;
/// The value the probe stores in its out word before each call.
const OUT_PRESET: u64 = 0xFFFF_FFFF;
/// The slots that hold a 335 answer. The replay does not compare them.
const UNMAP_SLOTS: [usize; 12] = [11, 13, 17, 19, 21, 26, 29, 31, 37, 38, 41, 42];

const MAP: u64 = syscall::MMAPPER_MAP_SHARED_MEMORY;
const SEARCH: u64 = syscall::MMAPPER_SEARCH_AND_MAP;
const FREE_SHM: u64 = syscall::MMAPPER_FREE_SHARED_MEMORY;
const FREE_ADDR: u64 = syscall::MMAPPER_FREE_ADDRESS;
/// `sys_mmapper_unmap_shared_memory`, which the probe sends and CellGov
/// does not route.
const UNMAP: u64 = 335;

fn console(name: &str) -> Vec<u64> {
    console_payload(name)
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// The probe's `sc`: the return code and the u32 of the first write.
fn sc(host: &mut Lv2Host, rt: &FakeRuntime, number: u64, args: [u64; 4]) -> (u64, Option<u64>) {
    let [a, b, c, d] = args;
    let request = classify(number, &[a, b, c, d, 0, 0, 0, 0]);
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

/// The probe's setup, slots 0-5: area A under `area_flags`, a 64 KiB
/// segment M and a 1 MiB segment M1.
struct Setup {
    host: Lv2Host,
    a: u64,
    m: u64,
    m1: u64,
}

fn setup(words: &[u64], rt: &FakeRuntime, area_flags: u64) -> Setup {
    let mut host = Lv2Host::new();
    let mut step = |slot: usize, number: u64, args: [u64; 4]| {
        let (code, written) = sc(&mut host, rt, number, args);
        assert_eq!(code, words[slot], "setup slot {slot}");
        written.expect("setup writes its out word")
    };
    let a = step(
        0,
        syscall::MMAPPER_ALLOCATE_ADDRESS,
        [AREA, area_flags, AREA, OUT],
    );
    let m = step(
        2,
        syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
        [KEYLESS, K64, FLAG_64K, OUT],
    );
    let m1 = step(
        4,
        syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
        [KEYLESS, MIB, FLAG_1M, OUT],
    );
    Setup { host, a, m, m1 }
}

impl Setup {
    fn rc(&mut self, words: &[u64], slot: usize, rt: &FakeRuntime, number: u64, args: [u64; 4]) {
        let (code, _) = sc(&mut self.host, rt, number, args);
        assert_eq!(code, words[slot], "slot {slot}: syscall {number} {args:x?}");
    }

    /// A 337 whose rc is in `slot` and address in `slot + 1`. A refused
    /// search leaves the probe's out word at its 0xFFFFFFFF preset.
    fn search(&mut self, words: &[u64], slot: usize, rt: &FakeRuntime, start: u64, id: u64) {
        let (code, written) = sc(&mut self.host, rt, SEARCH, [start, id, PROT_RW, OUT]);
        assert_eq!(code, words[slot], "slot {slot}: 337 from {start:#x}");
        if code == 0 {
            let found = written.expect("337 writes the address");
            assert_eq!(
                found - self.a,
                words[slot + 1] - words[1],
                "slot {}: offset in the area",
                slot + 1
            );
        } else {
            assert_eq!(words[slot + 1], OUT_PRESET, "slot {}", slot + 1);
            assert_eq!(written, None, "slot {}: a refused 337 writes", slot + 1);
        }
    }
}

fn replay(name: &str, area_flags: u64) {
    let words = console(name);
    let rt = FakeRuntime::new(0x1000);
    let fresh = || setup(&words, &rt, area_flags);

    // Slots 6-8 run on one area, in order.
    let mut s = fresh();
    let (a, m, m1) = (s.a, s.m, s.m1);
    s.rc(&words, 6, &rt, MAP, [a, m, PROT_RW, 0]);
    s.rc(&words, 7, &rt, MAP, [a, m, PROT_RW, 0]);
    s.rc(&words, 8, &rt, MAP, [a + K64, m1, PROT_RW, 0]);

    fresh().rc(&words, 14, &rt, MAP, [a + 0x100, m, PROT_RW, 0]);
    fresh().rc(&words, 15, &rt, MAP, [a, BAD_ID, PROT_RW, 0]);
    fresh().rc(&words, 16, &rt, MAP, [a, m, 0x999, 0]);
    fresh().rc(&words, 18, &rt, MAP, [a, m, PROT_RO, 0]);
    assert_eq!(words[46], words[1] + AREA, "slot 46");
    fresh().rc(&words, 20, &rt, MAP, [a + AREA, m, PROT_RW, 0]);

    fresh().search(&words, 24, &rt, a, m);
    fresh().search(&words, 27, &rt, a + K64, m);
    fresh().rc(&words, 30, &rt, SEARCH, [a, m, PROT_RW, 0]);
    fresh().rc(&words, 32, &rt, SEARCH, [a, BAD_ID, PROT_RW, OUT]);
    let mut s = fresh();
    s.search(&words, 33, &rt, a, m1);
    s.search(&words, 35, &rt, a, m1);

    fresh().rc(&words, 40, &rt, MAP, [a, m, PROT_RW, 0]);

    // The closing frees, on an area that holds no mapping. The 64 KiB
    // capture's slot 43 is a named divergence below.
    let mut s = fresh();
    if name != "lv2_mmapper_map_64k" {
        s.rc(&words, 43, &rt, FREE_SHM, [m, 0, 0, 0]);
    }
    s.rc(&words, 44, &rt, FREE_SHM, [m1, 0, 0, 0]);
    s.rc(&words, 45, &rt, FREE_ADDR, [a, 0, 0, 0]);
}

#[test]
fn mmapper_map_answers_as_on_the_console_in_an_area_of_1m_pages() {
    replay("lv2_mmapper_map", FLAG_1M);
}

#[test]
fn mmapper_map_answers_as_on_the_console_in_an_area_of_64k_pages() {
    replay("lv2_mmapper_map_64k", FLAG_64K);
}

/// The `lv2_mmapper_map_edges` capture, in an area of 64 KiB pages: a
/// map past the area's end, a search behind a 64 KiB map, the area free
/// at a non-base address and twice, and an area reserved after a free.
#[test]
fn mmapper_edges_answer_as_on_the_console() {
    let words = console("lv2_mmapper_map_edges");
    let rt = FakeRuntime::new(0x1000);
    let fresh = || setup(&words, &rt, FLAG_64K);

    let mut s = fresh();
    let (a, m, m1) = (s.a, s.m, s.m1);
    s.rc(&words, 6, &rt, MAP, [a + AREA - K64, m1, PROT_RW, 0]);
    assert_eq!(words[7], SKIP, "slot 7: nothing to unmap");

    let mut s = fresh();
    s.rc(&words, 8, &rt, MAP, [a, m, PROT_RW, 0]);
    s.search(&words, 9, &rt, a, m1);

    // The console unmapped both windows (slots 11-12); a fresh host
    // stands in for that.
    let mut s = fresh();
    s.rc(&words, 13, &rt, FREE_ADDR, [a + K64, 0, 0, 0]);
    s.rc(&words, 14, &rt, FREE_SHM, [m, 0, 0, 0]);
    s.rc(&words, 15, &rt, FREE_SHM, [m1, 0, 0, 0]);
    s.rc(&words, 16, &rt, FREE_ADDR, [a, 0, 0, 0]);
    s.rc(&words, 17, &rt, FREE_ADDR, [a, 0, 0, 0]);
    let einval = u64::from(errno::CELL_EINVAL);
    assert_eq!(
        sc(&mut s.host, &rt, MAP, [a, m, PROT_RW, 0]).0,
        einval,
        "a freed area holds no map"
    );

    let (code, base) = sc(
        &mut s.host,
        &rt,
        syscall::MMAPPER_ALLOCATE_ADDRESS,
        [AREA, FLAG_64K, AREA, OUT],
    );
    assert_eq!(code, words[18], "slot 18");
    let base = base.expect("330 writes the base");
    assert_eq!(
        base - a,
        words[19] - words[1],
        "slot 19: the freed base again"
    );
    s.rc(&words, 20, &rt, FREE_ADDR, [base, 0, 0, 0]);
}

#[test]
fn the_area_lookup_stops_at_the_area_end() {
    let words = console("lv2_mmapper_map_64k");
    let rt = FakeRuntime::new(0x1000);
    let s = setup(&words, &rt, FLAG_64K);
    assert_eq!(
        s.host
            .mmapper_area_of(s.a + AREA - 1)
            .map(|(b, _)| u64::from(b)),
        Some(s.a)
    );
    assert_eq!(s.host.mmapper_area_of(s.a + AREA), None);
    assert_eq!(s.host.mmapper_area_of(s.a - 1), None);
}

/// CellGov's choice: neither probe fills an area. A search stays inside
/// the area it starts at and answers CELL_ENOMEM when that area is full.
#[test]
fn a_search_stays_inside_its_area() {
    let words = console("lv2_mmapper_map_64k");
    let rt = FakeRuntime::new(0x1000);
    let mut s = setup(&words, &rt, FLAG_64K);
    let a = s.a;
    s.host.mmapper_ledger_insert(
        u32::try_from(a).expect("u32 area base"),
        u32::try_from(AREA).expect("u32 area size"),
    );
    assert_eq!(
        sc(&mut s.host, &rt, SEARCH, [a, s.m, PROT_RW, OUT]).0,
        u64::from(errno::CELL_ENOMEM)
    );
}

/// Divergence: the console unmaps with 335, which CellGov does not
/// route; its 335 answers CELL_ENOSYS. No 335 answer in either capture
/// is CELL_ENOSYS.
#[test]
fn divergence_unmap_is_not_routed() {
    let words = console("lv2_mmapper_map_64k");
    let rt = FakeRuntime::new(0x1000);
    let mut s = setup(&words, &rt, FLAG_64K);
    let a = s.a;
    assert_eq!(sc(&mut s.host, &rt, MAP, [a, s.m, PROT_RW, 0]).0, words[6]);
    assert_eq!(words[11], 0, "the console unmaps the window");
    assert_eq!(words[12], words[3], "the console's 335 returns the id of M");
    assert_eq!(
        sc(&mut s.host, &rt, UNMAP, [a, OUT, 0, 0]).0,
        u64::from(errno::CELL_ENOSYS),
        "CellGov does not route 335"
    );
    for name in ["lv2_mmapper_map", "lv2_mmapper_map_64k"] {
        let words = console(name);
        for slot in UNMAP_SLOTS {
            assert_ne!(
                words[slot],
                u64::from(errno::CELL_ENOSYS),
                "{name} slot {slot}"
            );
        }
    }
}

/// Divergence: the console refuses to free a segment or an area that
/// still holds a mapping; CellGov frees both.
#[test]
fn divergence_a_free_while_mapped_is_not_refused() {
    let words = console("lv2_mmapper_map_64k");
    let rt = FakeRuntime::new(0x1000);
    let mut s = setup(&words, &rt, FLAG_64K);
    let (a, m) = (s.a, s.m);
    assert_eq!(sc(&mut s.host, &rt, MAP, [a, m, PROT_RW, 0]).0, words[6]);
    let ebusy = u64::from(errno::CELL_EBUSY);
    assert_eq!(words[9], ebusy, "the console keeps a mapped segment");
    assert_eq!(words[10], ebusy, "the console keeps a mapped area");
    assert_eq!(sc(&mut s.host, &rt, FREE_SHM, [m, 0, 0, 0]).0, 0);
    assert_eq!(sc(&mut s.host, &rt, FREE_ADDR, [a, 0, 0, 0]).0, 0);
}

/// Divergence: at the end of the 64 KiB probe the console refuses to
/// free M although every map of it was unmapped. The one map attempt
/// with no unmap after it is the 337 of slot 30, which answered
/// CELL_EFAULT. That search left no window at A: the 337 of slot 33 and
/// the 334 of slot 40 both map at A. CellGov frees M.
#[test]
fn divergence_a_segment_after_a_faulted_search_still_frees() {
    let words = console("lv2_mmapper_map_64k");
    let rt = FakeRuntime::new(0x1000);
    let mut s = setup(&words, &rt, FLAG_64K);
    let (a, m) = (s.a, s.m);
    let efault = u64::from(errno::CELL_EFAULT);
    assert_eq!(words[30], efault);
    assert_eq!(words[31], SKIP);
    assert_eq!((words[33], words[34]), (0, words[1]), "slot 33 maps at A");
    assert_eq!(words[40], 0, "slot 40 maps at A");
    assert_eq!(sc(&mut s.host, &rt, SEARCH, [a, m, PROT_RW, 0]).0, efault);
    assert_eq!(words[43], u64::from(errno::CELL_EBUSY));
    assert_eq!(sc(&mut s.host, &rt, FREE_SHM, [m, 0, 0, 0]).0, 0);
}
