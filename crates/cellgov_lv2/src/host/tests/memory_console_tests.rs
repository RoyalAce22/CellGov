//! The memory allocate, free and container calls held to the
//! `lv2_memory_free_container` capture from the reference console. The
//! probe's calls run through the classifier and dispatch in its order,
//! and every return code is compared with the captured word. The
//! available user memory is compared as a change from the first read:
//! the console's total and its first read count the loaded image, which
//! CellGov's do not. A container id is compared by its base and by the
//! step between two of the probe's creates: its serial comes from a
//! counter the console shares across processes. Block bases are
//! CellGov's own.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::kernel_id::MEMORY_CONTAINER_SERIAL_MASK;
use cellgov_ps3_abi::lv2::syscall;

use super::*;
use crate::host::test_support::{console_payload, FakeRuntime};
use crate::request::classify;
use crate::Lv2Dispatch;

const CID_PTR: u64 = 0x100;
const ADDR_PTR: u64 = 0x104;
const INFO_PTR: u64 = 0x108;
const MIB: u64 = 0x10_0000;
const PAGE_1M: u64 = 0x400;

fn console() -> Vec<u64> {
    console_payload("lv2_memory_free_container")
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// The probe's `sc`: the return code and the bytes of the first write.
fn sc(host: &mut Lv2Host, rt: &FakeRuntime, number: u64, args: [u64; 4]) -> (u64, Vec<u8>) {
    let [a, b, c, d] = args;
    let request = classify(number, &[a, b, c, d, 0, 0, 0, 0]);
    match host.dispatch(request, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code, effects } => {
            let written = effects
                .iter()
                .find_map(|e| match e {
                    Effect::SharedWriteIntent { bytes, .. } => Some(bytes.bytes().to_vec()),
                    _ => None,
                })
                .unwrap_or_default();
            (code, written)
        }
        other => panic!("syscall {number}: expected Immediate, got {other:?}"),
    }
}

/// A container id without its serial.
fn base(cid: u64) -> u64 {
    cid & !u64::from(MEMORY_CONTAINER_SERIAL_MASK)
}

fn word(bytes: &[u8]) -> u64 {
    u64::from(u32::from_be_bytes(
        bytes[..4].try_into().expect("a u32 write"),
    ))
}

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
    words: Vec<u64>,
    first_avail: u64,
}

impl Probe {
    fn rc(&mut self, slot: usize, number: u64, args: [u64; 4]) -> Vec<u8> {
        let (code, written) = sc(&mut self.host, &self.rt, number, args);
        assert_eq!(
            code, self.words[slot],
            "slot {slot}: syscall {number} {args:x?}"
        );
        written
    }

    /// A create or allocate whose out word the probe records only when
    /// it succeeded; returns that word.
    fn minted(&mut self, slot: usize, number: u64, args: [u64; 4]) -> Option<u64> {
        let written = self.rc(slot, number, args);
        (self.words[slot] == 0).then(|| word(&written))
    }

    fn avail(&mut self) -> u64 {
        let (_, written) = sc(
            &mut self.host,
            &self.rt,
            syscall::MEMORY_GET_USER_MEMORY_SIZE,
            [INFO_PTR, 0, 0, 0],
        );
        u64::from(u32::from_be_bytes(written[4..8].try_into().expect("info")))
    }

    /// The available user memory at `slot` holds the console's change
    /// from its first read.
    fn avail_at(&mut self, slot: usize) {
        let here = self.avail();
        assert_eq!(
            self.first_avail.wrapping_sub(here),
            self.words[2].wrapping_sub(self.words[slot]),
            "slot {slot}: available user memory taken since the first read"
        );
    }
}

#[test]
fn memory_allocate_free_and_containers_answer_as_on_the_console() {
    let words = console();
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let (code, _) = sc(
        &mut host,
        &rt,
        syscall::MEMORY_GET_USER_MEMORY_SIZE,
        [INFO_PTR, 0, 0, 0],
    );
    assert_eq!(code, words[0], "slot 0");
    let mut p = Probe {
        host,
        rt,
        words,
        first_avail: 0,
    };
    p.first_avail = p.avail();
    const CREATE_324: u64 = syscall::MEMORY_CONTAINER_CREATE_324;
    const CREATE: u64 = syscall::MEMORY_CONTAINER_CREATE;
    const DESTROY: u64 = syscall::MEMORY_CONTAINER_DESTROY;
    const ALLOCATE: u64 = syscall::MEMORY_ALLOCATE;
    const FREE: u64 = syscall::MEMORY_FREE;
    const FROM: u64 = syscall::MEMORY_ALLOCATE_FROM_CONTAINER;

    // B. 324.
    p.rc(3, CREATE_324, [0, 0x8_0000, 0, 0]);
    p.rc(4, CREATE_324, [0, MIB, 0, 0]);
    if let Some(c) = p.minted(5, CREATE_324, [CID_PTR, 0x8_0000, 0, 0]) {
        sc(&mut p.host, &p.rt, DESTROY, [c, 0, 0, 0]);
    }
    let c = p
        .minted(6, CREATE_324, [CID_PTR, MIB, 0, 0])
        .expect("slot 6");
    assert_eq!(base(c), base(p.words[7]), "slot 7: a 324 container's base");
    p.avail_at(8);
    p.rc(9, DESTROY, [c, 0, 0, 0]);
    p.avail_at(10);

    // B. 341.
    p.rc(11, CREATE, [0, 0x8_0000, 0, 0]);
    p.rc(12, CREATE, [0, MIB, 0, 0]);
    if let Some(c) = p.minted(13, CREATE, [CID_PTR, 0x8_0000, 0, 0]) {
        sc(&mut p.host, &p.rt, DESTROY, [c, 0, 0, 0]);
    }
    let c = p
        .minted(14, CREATE, [CID_PTR, 0x18_0000, 0, 0])
        .expect("slot 14");
    assert_eq!(
        base(c),
        base(p.words[15]),
        "slot 15: a 341 container's base"
    );
    let before = c;
    p.avail_at(16);
    p.rc(17, DESTROY, [c, 0, 0, 0]);
    p.avail_at(18);
    p.rc(19, DESTROY, [c, 0, 0, 0]);
    p.rc(20, DESTROY, [0xDEAD_BEEF, 0, 0, 0]);

    // C. 348.
    for (slot, size, flags) in [(21, 0, PAGE_1M), (23, 0x1000, PAGE_1M), (25, MIB, 0x999)] {
        if let Some(a) = p.minted(slot, ALLOCATE, [size, flags, ADDR_PTR, 0]) {
            sc(&mut p.host, &p.rt, FREE, [a, 0, 0, 0]);
        }
    }
    p.avail_at(27);
    p.rc(28, ALLOCATE, [MIB, PAGE_1M, 0, 0]);
    p.avail_at(29);

    // D. 349.
    let a1 = p
        .minted(30, ALLOCATE, [MIB, PAGE_1M, ADDR_PTR, 0])
        .expect("slot 30");
    p.avail_at(32);
    p.rc(33, FREE, [a1, 0, 0, 0]);
    p.avail_at(34);
    p.rc(35, FREE, [a1, 0, 0, 0]);
    p.rc(36, FREE, [0xDEAD_0000, 0, 0, 0]);
    let a2 = p
        .minted(37, ALLOCATE, [MIB, PAGE_1M, ADDR_PTR, 0])
        .expect("slot 37");
    p.rc(39, FREE, [a2 + 0x1_0000, 0, 0, 0]);
    p.rc(40, FREE, [a2, 0, 0, 0]);

    // E. 350.
    let c = p.minted(41, CREATE, [CID_PTR, MIB, 0, 0]).expect("slot 41");
    assert_eq!(
        c.wrapping_sub(before),
        p.words[42].wrapping_sub(p.words[15]),
        "slot 42: the serial step since slot 15"
    );
    p.rc(43, FROM, [0, c, PAGE_1M, ADDR_PTR]);
    p.rc(44, FROM, [0x1000, c, 0x999, ADDR_PTR]);
    p.rc(45, FROM, [MIB, 0xDEAD_BEEF, 0x999, ADDR_PTR]);
    p.rc(46, FROM, [MIB + 1, 0xDEAD_BEEF, PAGE_1M, ADDR_PTR]);
    p.rc(47, FROM, [2 * MIB, c, PAGE_1M, ADDR_PTR]);
    p.rc(48, FROM, [2 * MIB, c, PAGE_1M, 0]);
    p.rc(49, FROM, [MIB, c, PAGE_1M, 0]);
    let a3 = p
        .minted(50, FROM, [MIB, c, PAGE_1M, ADDR_PTR])
        .expect("slot 50");
    p.rc(52, DESTROY, [c, 0, 0, 0]);
    p.rc(53, FREE, [a3, 0, 0, 0]);
    p.rc(54, DESTROY, [c, 0, 0, 0]);
    p.avail_at(55);
}
