//! The `lv2_mmapper_map`, `lv2_mmapper_map_64k` and `lv2_mmapper_unmap`
//! probes replayed whole on one host, each payload compared word for
//! word with the reference console's capture.
//!
//! Each replay follows its probe's `ppu/main.c` step for step, the
//! branches on earlier answers included, and keeps the probe's out word:
//! preset to 0xFFFFFFFF before a call that takes it, left alone by a
//! refusal. An address is compared as an offset from the area base and
//! an id by the segment it names, since a fresh host's kernel-id slots
//! and area base are not the console process's.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
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
const OUT_PRESET: u64 = 0xFFFF_FFFF;

const ALLOC_ADDR: u64 = syscall::MMAPPER_ALLOCATE_ADDRESS;
const FREE_ADDR: u64 = syscall::MMAPPER_FREE_ADDRESS;
const ALLOC_SHM: u64 = syscall::MMAPPER_ALLOCATE_SHARED_MEMORY;
const FREE_SHM: u64 = syscall::MMAPPER_FREE_SHARED_MEMORY;
const MAP: u64 = syscall::MMAPPER_MAP_SHARED_MEMORY;
const UNMAP: u64 = syscall::MMAPPER_UNMAP_SHARED_MEMORY;
const SEARCH: u64 = syscall::MMAPPER_SEARCH_AND_MAP;

/// One host running a probe's calls in order, with the probe's out word.
struct Probe<'a> {
    host: Lv2Host,
    rt: &'a FakeRuntime,
    out: Vec<u64>,
    w32: u64,
}

impl<'a> Probe<'a> {
    /// The probe's payload before its first call: SKIP in every slot
    /// but the last, which is zero.
    fn new(rt: &'a FakeRuntime, words: usize) -> Self {
        let mut out = vec![SKIP; words];
        out[words - 1] = 0;
        Self {
            host: Lv2Host::new(),
            rt,
            out,
            w32: OUT_PRESET,
        }
    }

    fn put(&mut self, slot: usize, v: u64) {
        self.out[slot] = v;
    }

    /// The probe's `sc`: a call's first write lands in its out word.
    fn sc(&mut self, number: u64, args: [u64; 4]) -> u64 {
        let [a, b, c, d] = args;
        let request = classify(number, &[a, b, c, d, 0, 0, 0, 0]);
        match self.host.dispatch(request, UnitId::new(0), self.rt) {
            Lv2Dispatch::Immediate { code, effects } => {
                if let Some(word) = effects.iter().find_map(|e| match e {
                    Effect::SharedWriteIntent { bytes, .. } => Some(u32::from_be_bytes(
                        bytes.bytes()[..4].try_into().expect("a u32 write"),
                    )),
                    _ => None,
                }) {
                    self.w32 = u64::from(word);
                }
                code
            }
            other => panic!("syscall {number}: expected Immediate, got {other:?}"),
        }
    }

    /// A call that takes the out word, preset first.
    fn sc_out(&mut self, number: u64, args: [u64; 4]) -> u64 {
        self.w32 = OUT_PRESET;
        self.sc(number, args)
    }

    fn unmap(&mut self, addr: u64) -> u64 {
        self.sc_out(UNMAP, [addr, OUT, 0, 0])
    }
}

/// The capture's payload words.
fn console(name: &str) -> Vec<u64> {
    console_payload(name)
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// `payload` with each address slot as an offset from the area base in
/// slot 1 and each id slot as the setup slot that minted the id. A slot
/// that holds no address or id (a refusal's preset, SKIP) is left as is.
fn normalized(
    payload: &[u64],
    addr_slots: &[usize],
    id_slots: &[usize],
    setup_ids: &[usize],
) -> Vec<u64> {
    let base = payload[1];
    let mut out = payload.to_vec();
    for &slot in addr_slots {
        if out[slot] != SKIP && out[slot] != OUT_PRESET {
            out[slot] = out[slot].wrapping_sub(base);
        }
    }
    for &slot in id_slots.iter().chain(setup_ids) {
        if let Some(&minted) = setup_ids.iter().find(|&&s| payload[s] == payload[slot]) {
            out[slot] = 0x1D_0000 + minted as u64;
        }
    }
    out
}

fn assert_payloads_agree(
    name: &str,
    cellgov: &[u64],
    addr_slots: &[usize],
    id_slots: &[usize],
    setup_ids: &[usize],
) {
    let console = console(name);
    assert_eq!(cellgov.len(), console.len(), "{name}: payload length");
    let cellgov = normalized(cellgov, addr_slots, id_slots, setup_ids);
    let console = normalized(&console, addr_slots, id_slots, setup_ids);
    for (slot, (ours, theirs)) in cellgov.iter().zip(&console).enumerate() {
        assert_eq!(
            ours, theirs,
            "{name} slot {slot}: CellGov {ours:#x}, console {theirs:#x}"
        );
    }
}

/// The `lv2_mmapper_map` / `lv2_mmapper_map_64k` probe's `main`, in an
/// area of `area_flags` pages.
fn map_probe(rt: &FakeRuntime, area_flags: u64) -> Vec<u64> {
    let mut p = Probe::new(rt, 48);
    let rc = p.sc_out(ALLOC_ADDR, [AREA, area_flags, AREA, OUT]);
    let a = p.w32;
    p.put(0, rc);
    p.put(1, a);
    if rc != 0 {
        return p.out;
    }
    let rc = p.sc_out(ALLOC_SHM, [KEYLESS, K64, FLAG_64K, OUT]);
    let m = p.w32;
    p.put(2, rc);
    p.put(3, m);
    if rc != 0 {
        let v = p.sc(FREE_ADDR, [a, 0, 0, 0]);
        p.put(45, v);
        return p.out;
    }
    let rc = p.sc_out(ALLOC_SHM, [KEYLESS, MIB, FLAG_1M, OUT]);
    let m1 = p.w32;
    p.put(4, rc);
    p.put(5, m1);
    if rc != 0 {
        let v = p.sc(FREE_SHM, [m, 0, 0, 0]);
        p.put(43, v);
        let v = p.sc(FREE_ADDR, [a, 0, 0, 0]);
        p.put(45, v);
        return p.out;
    }

    let rc = p.sc(MAP, [a, m, PROT_RW, 0]);
    p.put(6, rc);
    let rc2 = p.sc(MAP, [a, m, PROT_RW, 0]);
    p.put(7, rc2);
    let x1 = p.sc(MAP, [a + K64, m1, PROT_RW, 0]);
    p.put(8, x1);
    if x1 == 0 {
        p.unmap(a + K64);
    }
    if rc == 0 {
        let v = p.sc(FREE_SHM, [m, 0, 0, 0]);
        p.put(9, v);
        let v = p.sc(FREE_ADDR, [a, 0, 0, 0]);
        p.put(10, v);
        let v = p.unmap(a);
        p.put(11, v);
        p.put(12, p.w32);
        let v = p.unmap(a);
        p.put(13, v);
    }
    for (slot, addr, id) in [(14, a + 0x100, m), (15, a, BAD_ID)] {
        let x1 = p.sc(MAP, [addr, id, PROT_RW, 0]);
        p.put(slot, x1);
        if x1 == 0 {
            p.unmap(addr);
        }
    }
    let other = if a + AREA < 0xC000_0000 {
        a + AREA
    } else {
        a - AREA
    };
    p.put(46, other);
    for (slot, addr, flags) in [(16, a, 0x999), (18, a, PROT_RO), (20, other, PROT_RW)] {
        let rc = p.sc(MAP, [addr, m, flags, 0]);
        p.put(slot, rc);
        let v = if rc == 0 { p.unmap(addr) } else { SKIP };
        p.put(slot + 1, v);
    }

    for (slot, start) in [(24, a), (27, a + K64)] {
        let rc = p.sc_out(SEARCH, [start, m, PROT_RW, OUT]);
        let addr = p.w32;
        p.put(slot, rc);
        p.put(slot + 1, addr);
        let v = if rc == 0 { p.unmap(addr) } else { SKIP };
        p.put(slot + 2, v);
    }
    let rc = p.sc(SEARCH, [a, m, PROT_RW, 0]);
    p.put(30, rc);
    let v = if rc == 0 { p.unmap(a) } else { SKIP };
    p.put(31, v);
    let x1 = p.sc_out(SEARCH, [a, BAD_ID, PROT_RW, OUT]);
    p.put(32, x1);
    if x1 == 0 {
        let addr = p.w32;
        p.unmap(addr);
    }
    let rc = p.sc_out(SEARCH, [a, m1, PROT_RW, OUT]);
    let x1 = p.w32;
    p.put(33, rc);
    p.put(34, x1);
    let rc2 = p.sc_out(SEARCH, [a, m1, PROT_RW, OUT]);
    let x2 = p.w32;
    p.put(35, rc2);
    p.put(36, x2);
    let v = if rc == 0 { p.unmap(x1) } else { SKIP };
    p.put(37, v);
    let v = if rc2 == 0 { p.unmap(x2) } else { SKIP };
    p.put(38, v);

    let rc = p.sc(MAP, [a, m, PROT_RW, 0]);
    p.put(40, rc);
    if rc == 0 {
        let v = p.sc(UNMAP, [a, 0, 0, 0]);
        p.put(41, v);
        let v = p.unmap(a);
        p.put(42, v);
    }

    let v = p.sc(FREE_SHM, [m, 0, 0, 0]);
    p.put(43, v);
    let v = p.sc(FREE_SHM, [m1, 0, 0, 0]);
    p.put(44, v);
    let v = p.sc(FREE_ADDR, [a, 0, 0, 0]);
    p.put(45, v);
    p.out
}

const MAP_ADDR_SLOTS: [usize; 6] = [1, 25, 28, 34, 36, 46];
const MAP_ID_SLOTS: [usize; 1] = [12];
const MAP_SETUP_IDS: [usize; 2] = [3, 5];

#[test]
fn the_map_probe_in_an_area_of_1m_pages_reads_as_on_the_console() {
    let rt = FakeRuntime::new(0x1000);
    assert_payloads_agree(
        "lv2_mmapper_map",
        &map_probe(&rt, FLAG_1M),
        &MAP_ADDR_SLOTS,
        &MAP_ID_SLOTS,
        &MAP_SETUP_IDS,
    );
}

#[test]
fn the_map_probe_in_an_area_of_64k_pages_reads_as_on_the_console() {
    let rt = FakeRuntime::new(0x1000);
    assert_payloads_agree(
        "lv2_mmapper_map_64k",
        &map_probe(&rt, FLAG_64K),
        &MAP_ADDR_SLOTS,
        &MAP_ID_SLOTS,
        &MAP_SETUP_IDS,
    );
}

/// The `lv2_mmapper_unmap` probe's `main`.
fn unmap_probe(rt: &FakeRuntime) -> Vec<u64> {
    let mut p = Probe::new(rt, 34);
    let rc = p.sc_out(ALLOC_ADDR, [AREA, FLAG_64K, AREA, OUT]);
    let a = p.w32;
    p.put(0, rc);
    p.put(1, a);
    if rc != 0 {
        return p.out;
    }
    let mut ids = [0u64; 3];
    for (n, (slot, size, flags)) in [(2, K64, FLAG_64K), (4, MIB, FLAG_1M), (6, K64, FLAG_64K)]
        .into_iter()
        .enumerate()
    {
        let rc = p.sc_out(ALLOC_SHM, [KEYLESS, size, flags, OUT]);
        ids[n] = p.w32;
        p.put(slot, rc);
        p.put(slot + 1, ids[n]);
        if rc != 0 {
            // The probe's setup failures; none happens on a fresh host.
            return p.out;
        }
    }
    let [m, m1, m2] = ids;

    let rc = p.sc(MAP, [a, m1, PROT_RW, 0]);
    p.put(8, rc);
    if rc == 0 {
        let v = p.unmap(a + K64);
        p.put(9, v);
        p.put(10, p.w32);
        let v = p.unmap(a);
        p.put(11, v);
        p.put(12, p.w32);
    }

    let other = if a + AREA < 0xC000_0000 {
        a + AREA
    } else {
        a - AREA
    };
    let v = p.unmap(other);
    p.put(13, v);
    p.put(32, p.w32);
    let v = p.sc(UNMAP, [a, 0, 0, 0]);
    p.put(14, v);

    let v = p.sc(MAP, [a, m, PROT_RW, 0]);
    p.put(15, v);
    let v = p.sc(MAP, [a + 2 * K64, m, PROT_RW, 0]);
    p.put(16, v);
    let v = p.unmap(a);
    p.put(17, v);
    p.put(18, p.w32);
    let v = p.sc(FREE_SHM, [m, 0, 0, 0]);
    p.put(19, v);
    let v = p.unmap(a + 2 * K64);
    p.put(20, v);
    p.put(21, p.w32);
    let v = p.sc(FREE_SHM, [m, 0, 0, 0]);
    p.put(22, v);

    let v = p.sc(SEARCH, [a, m2, PROT_RW, 0]);
    p.put(23, v);
    let v = p.unmap(a);
    p.put(24, v);
    let v = p.sc(FREE_SHM, [m2, 0, 0, 0]);
    p.put(25, v);
    let rc = p.sc(MAP, [a, m2, PROT_RW, 0]);
    p.put(26, rc);
    if rc == 0 {
        let v = p.unmap(a);
        p.put(27, v);
    }
    let v = p.sc(FREE_SHM, [m2, 0, 0, 0]);
    p.put(28, v);

    let v = p.sc(FREE_ADDR, [a, 0, 0, 0]);
    p.put(29, v);
    let v = p.sc(FREE_SHM, [m1, 0, 0, 0]);
    p.put(30, v);
    let v = p.sc(FREE_SHM, [m2, 0, 0, 0]);
    p.put(31, v);
    p.out
}

#[test]
fn the_unmap_probe_reads_as_on_the_console() {
    let rt = FakeRuntime::new(0x1000);
    assert_payloads_agree(
        "lv2_mmapper_unmap",
        &unmap_probe(&rt),
        &[1],
        &[10, 12, 18, 21],
        &[3, 5, 7],
    );
}
