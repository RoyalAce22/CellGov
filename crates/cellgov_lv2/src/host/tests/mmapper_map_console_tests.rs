//! The mmapper map and search edges held to the `lv2_mmapper_map_edges`
//! capture, and CellGov's own choices no capture reaches. The whole
//! map and unmap probes replay in `mmapper_unmap_console_tests.rs`.

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
const AREA: u64 = 0x1000_0000;
const MIB: u64 = 0x10_0000;
const K64: u64 = 0x1_0000;
const OUT: u64 = 0x100;
const SKIP: u64 = 0xDEAD_D00D;
/// The value the probe stores in its out word before each call.
const OUT_PRESET: u64 = 0xFFFF_FFFF;

const MAP: u64 = syscall::MMAPPER_MAP_SHARED_MEMORY;
const SEARCH: u64 = syscall::MMAPPER_SEARCH_AND_MAP;
const FREE_SHM: u64 = syscall::MMAPPER_FREE_SHARED_MEMORY;
const FREE_ADDR: u64 = syscall::MMAPPER_FREE_ADDRESS;

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
            .mmapper_area_of(
                cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID,
                s.a + AREA - 1
            )
            .map(|(b, _)| u64::from(b)),
        Some(s.a)
    );
    assert_eq!(
        s.host
            .mmapper_area_of(cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID, s.a + AREA),
        None
    );
    assert_eq!(
        s.host
            .mmapper_area_of(cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID, s.a - 1),
        None
    );
}

/// CellGov's choice: neither probe fills an area. A search stays inside
/// the area it starts at and answers CELL_ENOMEM when that area is full.
#[test]
fn a_search_stays_inside_its_area() {
    let words = console("lv2_mmapper_map_64k");
    let rt = FakeRuntime::new(0x1000);
    let mut s = setup(&words, &rt, FLAG_64K);
    let a = s.a;
    s.host.mmapper_window_insert(
        cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID,
        u32::try_from(a).expect("u32 area base"),
        u32::try_from(AREA).expect("u32 area size"),
        0,
    );
    assert_eq!(
        sc(&mut s.host, &rt, SEARCH, [a, s.m, PROT_RW, OUT]).0,
        u64::from(errno::CELL_ENOMEM)
    );
}
