//! The memory and mmapper calls held to the `lv2_memory_constants`
//! capture from the reference console: the area sizes and alignments
//! `sys_mmapper_allocate_address` takes, the granule
//! `sys_memory_container_create` truncates to, the ext entry stride and
//! types `sys_mmapper_allocate_shared_memory_ext` reads, and the flag
//! words `sys_memory_allocate` takes with the page size each one gives.
//! Every return code and area base is compared; container ids, segment
//! ids and block addresses are CellGov's own.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::memory::page_size;
use cellgov_ps3_abi::lv2::syscall;

use crate::host::test_support::{console_payload, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::classify;
use crate::Lv2Dispatch;

const KEYLESS: u64 = 0xFFFF_0000_0000_0000;
const FLAG_64K: u64 = 0x200;
const FLAG_1M: u64 = 0x400;
const AREA: u64 = 0x1000_0000;
const MIB: u64 = 0x10_0000;
const K64: u64 = 0x1_0000;
const OUT: u64 = 0x100;
const TABLE: u64 = 0x200;
/// The end of the test's guest memory: the end of a mapped page.
const END: u64 = 0x2000;
const SKIP: u64 = 0xDEAD_D00D;

fn console() -> Vec<u64> {
    console_payload("lv2_memory_constants")
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// A runtime holding the 64-bit `words` at `at`.
fn runtime_with(at: u64, words: &[u64]) -> FakeRuntime {
    let mut memory = GuestMemory::new(END as usize);
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    if !bytes.is_empty() {
        memory
            .apply_commit(
                ByteRange::new(GuestAddr::new(at), bytes.len() as u64).expect("range"),
                &bytes,
            )
            .expect("commit");
    }
    FakeRuntime::with_memory(memory)
}

/// The return code and the bytes of the first write.
fn sc(host: &mut Lv2Host, rt: &FakeRuntime, number: u64, args: [u64; 6]) -> (u64, Vec<u8>) {
    let [a, b, c, d, e, f] = args;
    match host.dispatch(
        classify(number, &[a, b, c, d, e, f, 0, 0]),
        UnitId::new(0),
        rt,
    ) {
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

fn u32_at(bytes: &[u8], at: usize) -> u64 {
    u64::from(u32::from_be_bytes(
        bytes[at..at + 4].try_into().expect("a u32"),
    ))
}

/// Note `got` when it differs from the console's word in `slot`.
fn check(differing: &mut Vec<String>, words: &[u64], slot: usize, got: u64, what: &str) {
    if got != words[slot] {
        differing.push(format!(
            "slot {slot} ({what}): cellgov {got:#x}, console {:#x}",
            words[slot]
        ));
    }
}

/// The available user memory sc 352 reports.
fn available(host: &mut Lv2Host, rt: &FakeRuntime) -> u64 {
    let (code, info) = sc(
        host,
        rt,
        syscall::MEMORY_GET_USER_MEMORY_SIZE,
        [OUT, 0, 0, 0, 0, 0],
    );
    assert_eq!(code, 0);
    u32_at(&info, 4)
}

#[test]
fn the_memory_calls_take_the_sizes_and_flags_the_console_takes() {
    let words = console();
    assert_eq!(words.len(), 113);
    let rt = runtime_with(0, &[]);
    let mut host = Lv2Host::new();
    let mut differing = Vec::new();

    // A. 330, each area freed with 331.
    for (i, (size, alignment)) in [
        (0x0FFF_FFFF, AREA),
        (0x1800_0000, AREA),
        (0x2000_0000, AREA),
        (AREA, 0x0800_0000),
        (AREA, 0x2000_0000),
        (AREA, 0x4000_0000),
        (AREA, 0x8000_0000),
        (AREA, 0x3000_0000),
    ]
    .into_iter()
    .enumerate()
    {
        let (code, out) = sc(
            &mut host,
            &rt,
            syscall::MMAPPER_ALLOCATE_ADDRESS,
            [size, FLAG_1M, alignment, OUT, 0, 0],
        );
        check(&mut differing, &words, 3 * i, code, "330");
        if code == 0 {
            let base = u32_at(&out, 0);
            // Each area is freed before the next, so both address
            // spaces are empty at every 330 and the bases compare.
            check(&mut differing, &words, 3 * i + 1, base, "330 base");
            let free = sc(
                &mut host,
                &rt,
                syscall::MMAPPER_FREE_ADDRESS,
                [base, 0, 0, 0, 0, 0],
            );
            check(&mut differing, &words, 3 * i + 2, free.0, "331");
        }
    }

    // B. 341: what each create takes from the available memory.
    let before = available(&mut host, &rt);
    for (i, size) in [0xF_FFFF, 0x10_0000, 0x1F_FFFF].into_iter().enumerate() {
        let slot = 24 + 4 * i;
        let (code, out) = sc(
            &mut host,
            &rt,
            syscall::MEMORY_CONTAINER_CREATE,
            [OUT, size, 0, 0, 0, 0],
        );
        check(&mut differing, &words, slot, code, "341");
        let taken = before - available(&mut host, &rt);
        check(
            &mut differing,
            &words,
            slot + 2,
            words[36] - taken,
            "available after 341",
        );
        if code == 0 {
            let destroy = sc(
                &mut host,
                &rt,
                syscall::MEMORY_CONTAINER_DESTROY,
                [u32_at(&out, 0), 0, 0, 0, 0, 0],
            );
            check(&mut differing, &words, slot + 3, destroy.0, "342");
        }
    }

    // C. 339 against tables of 64-bit words, laid out as the probe's
    // main.c lays them: word 2 is the first entry's type, and words 5
    // and 6 sit at bytes 0x28 and 0x30.
    let table = |types: &[(usize, u64)]| {
        let mut t = vec![0u64; 8];
        for &(at, value) in types {
            t[at] = value;
        }
        t
    };
    // (slot, where the table sits, its words, the pointer passed, count)
    let ext_cases: [(usize, u64, Vec<u64>, u64, u64); 7] = [
        (37, TABLE, table(&[(2, 7)]), TABLE, 1),
        (40, TABLE, table(&[(2, 8)]), TABLE, 1),
        (43, TABLE, table(&[]), TABLE, u64::MAX),
        (46, TABLE, table(&[(5, 2)]), TABLE, 2),
        (49, TABLE, table(&[(6, 2)]), TABLE, 2),
        // One 0x18-byte entry ending at the end of mapped memory, then
        // one byte further on.
        (52, END - 0x18, vec![0; 3], END - 0x18, 1),
        (55, END - 0x18, vec![0; 3], END - 0x17, 1),
    ];
    for (slot, placed, table, at, count) in ext_cases {
        let rt = runtime_with(placed, &table);
        let (code, out) = sc(
            &mut host,
            &rt,
            syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_EXT,
            [KEYLESS, K64, FLAG_64K, at, count, OUT],
        );
        check(&mut differing, &words, slot, code, "339");
        if code == 0 {
            let free = sc(
                &mut host,
                &rt,
                syscall::MMAPPER_FREE_SHARED_MEMORY,
                [u32_at(&out, 0), 0, 0, 0, 0, 0],
            );
            check(&mut differing, &words, slot + 2, free.0, "329");
        } else {
            check(&mut differing, &words, slot + 2, SKIP, "no free");
        }
    }

    // D. 348 with each flags word, and 349. CellGov does not serve 351,
    // so the page size the console read for each block is held to the
    // granule `page_size::accepted_granule` gives the same flags word.
    for (i, flags) in [0, 0x100, FLAG_64K, FLAG_1M, 0x600, 0x800, 0xF00]
        .into_iter()
        .enumerate()
    {
        let slot = 64 + 7 * i;
        let (code, out) = sc(
            &mut host,
            &rt,
            syscall::MEMORY_ALLOCATE,
            [MIB, flags, OUT, 0, 0, 0],
        );
        check(&mut differing, &words, slot, code, "348");
        let console_page = (words[slot] == 0).then(|| words[slot + 5] >> 32);
        let granule = page_size::accepted_granule(flags).map(u64::from);
        if granule != console_page {
            differing.push(format!(
                "flags {flags:#x}: granule {granule:x?}, console page size {console_page:x?}"
            ));
        }
        if code == 0 {
            let free = sc(
                &mut host,
                &rt,
                syscall::MEMORY_FREE,
                [u32_at(&out, 0), 0, 0, 0, 0, 0],
            );
            check(&mut differing, &words, slot + 6, free.0, "349");
        }
    }

    assert!(differing.is_empty(), "differs:\n{}", differing.join("\n"));
}
