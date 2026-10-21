//! Child-thread stacks held to the `lv2_ppu_thread_stack_span` capture
//! from the reference console. The probe creates twenty joinable threads
//! with 1 MiB stacks before any starts, then starts and joins each in
//! turn; each records its own `sys_ppu_thread_get_stack_information`
//! answer. A last 1 MiB thread follows once the joins collect all twenty.

use super::*;
use crate::ppu_thread::{PpuThreadAttrs, PpuThreadId, ThreadStack};
use cellgov_event::UnitId;

const THREADS: usize = 20;
const STACK: u64 = 0x10_0000;
/// Base and end of the PPU stack segment.
const SEGMENT: (u64, u64) = (0xD000_0000, 0xE000_0000);
/// End of the 15 MiB region CellGov backed before child stacks filled
/// the segment.
const OLD_REGION_END: u64 = 0xD100_0000;

/// The probe's payload, as big-endian u32 words.
fn words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/lv2_ppu_thread_stack_span/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// Thread n's (create rc, start rc, join rc, stack address, stack size)
/// on the console.
fn console_thread(w: &[u32], n: usize) -> [u64; 5] {
    let at = 2 + 5 * n;
    std::array::from_fn(|i| u64::from(w[at + i]))
}

/// Place a block and create an unstarted joinable thread holding it, as
/// one `sys_ppu_thread_create` does; `None` when the host refuses.
fn create(host: &mut Lv2Host, unit: u64) -> Option<(PpuThreadId, ThreadStack)> {
    let stack = host.allocate_child_stack(STACK, 0x10)?;
    let attrs = PpuThreadAttrs {
        entry: 0x1_0000,
        arg: 0,
        stack_base: stack.base() as u32,
        stack_size: stack.size() as u32,
        priority: 1000,
        tls_base: 0,
    };
    let id = host
        .ppu_threads_mut()
        .create_unstarted(UnitId::new(unit), attrs, true)?;
    Some((id, stack))
}

/// Every stack lies in the segment and no two overlap.
fn assert_disjoint_in_segment(who: &str, stacks: &[(u64, u64)]) {
    for (i, &(base, size)) in stacks.iter().enumerate() {
        assert!(
            base >= SEGMENT.0 && base + size <= SEGMENT.1,
            "{who}: stack {i} at {base:#x}+{size:#x} leaves the stack segment"
        );
        for &(other, other_size) in &stacks[i + 1..] {
            assert!(
                base + size <= other || other + other_size <= base,
                "{who}: stacks at {base:#x} and {other:#x} overlap"
            );
        }
    }
}

#[test]
fn twenty_live_1_mib_stacks_are_all_created_past_the_old_15_mib_region() {
    let w = words();
    assert_eq!(w[0], 0, "the probe's failed step");
    let console: Vec<[u64; 5]> = (0..THREADS).map(|n| console_thread(&w, n)).collect();

    let mut host = Lv2Host::new();
    let mut ids = Vec::new();
    let mut cellgov = Vec::new();
    for (n, c) in console.iter().enumerate() {
        let placed = create(&mut host, 10 + n as u64);
        assert_eq!(
            c[0] == 0,
            placed.is_some(),
            "thread {n}: the console's create rc is {:#x}",
            c[0]
        );
        let (id, stack) = placed.expect("a stack");
        assert_eq!(stack.size(), c[4], "thread {n}: stack size");
        ids.push(id);
        cellgov.push((stack.base(), stack.size()));
    }
    for (n, c) in console.iter().enumerate() {
        assert_eq!(
            (c[1], c[2]),
            (0, 0),
            "thread {n}: the console's start and join"
        );
    }

    let console_stacks: Vec<(u64, u64)> = console.iter().map(|c| (c[3], c[4])).collect();
    assert_disjoint_in_segment("the console", &console_stacks);
    assert_disjoint_in_segment("CellGov", &cellgov);
    let top = |s: &[(u64, u64)]| s.iter().map(|&(b, z)| b + z).max().expect("20 stacks");
    assert!(
        top(&console_stacks) > OLD_REGION_END,
        "the console's stacks"
    );
    assert!(top(&cellgov) > OLD_REGION_END, "CellGov's stacks");
    // The console places them 0x2000 apart from 0x2000 above the primary
    // stack; CellGov packs them from the base of its own region.
    assert_ne!(
        console_stacks[0].0, cellgov[0].0,
        "CellGov's region is its own"
    );
}

#[test]
fn once_all_twenty_are_joined_the_next_stack_takes_the_lowest_slot() {
    let w = words();
    assert_eq!(w[0], 0, "the probe's failed step");
    let console_first = u64::from(w[5]);
    let (console_rc, console_last, console_size) =
        (u64::from(w[102]), u64::from(w[103]), u64::from(w[104]));
    assert_eq!(console_rc, 0, "the console's last create");
    assert_eq!(
        console_last, console_first,
        "the console reuses its lowest slot"
    );

    let mut host = Lv2Host::new();
    let threads: Vec<(PpuThreadId, ThreadStack)> = (0..THREADS as u64)
        .map(|n| create(&mut host, 10 + n).expect("a stack"))
        .collect();
    for &(id, _) in &threads {
        host.ppu_threads_mut().mark_finished(id, 0);
        assert!(host.ppu_threads_mut().reap(id), "the join collects {id:?}");
    }
    let (_, last) = create(&mut host, 100).expect("a stack");
    assert_eq!(last.size(), console_size);
    assert_eq!(
        last.base(),
        threads[0].1.base(),
        "CellGov reuses its lowest slot"
    );
}
