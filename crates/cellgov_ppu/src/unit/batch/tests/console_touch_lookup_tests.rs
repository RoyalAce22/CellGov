//! The table lookup of a touch on a translating thread, held to three
//! captures from the reference console:
//! - `ppu_cache_block`: a dcbt and a dcbtst of an address that no page
//!   holds run to completion;
//! - `ppu_touch_alias_line`: a touch through one mapping of a page fills
//!   the line that a load through a second mapping finds;
//! - `ppu_touch_inhibited`: a touch of caching-inhibited storage, or of an
//!   address that no page holds, drops no line of a full L1 set, and a
//!   touch of a cacheable line drops one.

use super::{
    dcbt_r7, kernel_unit, memory, read_write, run, word, PAGE, PAGE_RA, SEGMENT, TOUCH_EA,
    TOUCH_RA, UNMAPPED_EA,
};
use crate::instruction::PpuInstruction as I;
use crate::PpuExecutionUnit;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::ppu_mmu::PTE_I;

/// The reference profile's captures.
const PROFILE: &str = "cech20-cex-493";

/// The payload of probe `probe`'s capture under [`PROFILE`].
fn console_payload(probe: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3")
        .join(PROFILE)
        .join("observation.json");
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    observation.memory_regions[0].data.clone()
}

fn be_u32(payload: &[u8]) -> Vec<u32> {
    payload
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

fn be_u64(payload: &[u8]) -> Vec<u64> {
    payload
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// `dcbtst 0,r7`: a touch for a store of the line at r7 with TH = 0.
fn dcbtst_r7() -> u32 {
    word(I::Dcbtst {
        ra: 0,
        rb: 7,
        th: 0,
    })
}

/// Runs batches of at most `budget` instructions until the unit reaches
/// `end`, and asserts that it does with no fault.
fn run_to(unit: &mut PpuExecutionUnit, mem: &GuestMemory, budget: u64, end: u64) {
    for _ in 0..=budget {
        if unit.state().pc == end {
            break;
        }
        let (r, _) = run(unit, mem, budget);
        assert!(r.fault.is_none(), "no fault: {:?}", r.fault);
    }
    assert_eq!(unit.state().pc, end, "every instruction retires");
}

/// The probe's `dcbt 0,r6` and `dcbtst 0,r6`, in its order.
const PROBE_TOUCHES: [u32; 2] = [0x7c00_322c, 0x7c00_31ec];
/// The probe's address in page 0, which no page holds. The kernel unit
/// maps no segment there.
const PROBE_UNHELD: u64 = 0x100;
/// The payload word of the marker after the probe's first touch. The
/// probe writes marker n + 1 after hint n.
const PROBE_TOUCH_MARKERS: usize = 10;

#[test]
fn the_touches_of_an_address_no_page_holds_retire_as_on_the_console() {
    let words = be_u32(&console_payload("ppu_cache_block"));
    assert_eq!(words.len(), 18);
    assert_eq!(words[0], 0, "the probe ran to its end");
    for n in 0..PROBE_TOUCHES.len() {
        assert_eq!(
            words[PROBE_TOUCH_MARKERS + n],
            n as u32 + 1,
            "the console ran touch {n} on"
        );
    }
    for touched in [PROBE_UNHELD, UNMAPPED_EA] {
        let mem = memory(&PROBE_TOUCHES, &[(PAGE, read_write(PAGE_RA))]);
        let mut unit = kernel_unit(0);
        unit.state_mut().set_gpr(6, touched);
        run_to(&mut unit, &mem, 2, SEGMENT + 8);
    }
}

/// The end mark of the `ppu_touch_alias_line` payload.
const ALIAS_END_MARK: u32 = 0x5443_4841;
/// Page 8 of the segment: a second mapping of [`PAGE_RA`], 16 KB above
/// page 4, so the two effective addresses of a line pick one L1 set.
const ALIAS: u64 = 8;
const ALIAS_TOUCH_EA: u64 = SEGMENT + 0x8000 + (TOUCH_EA & 0xFFF);

#[test]
fn a_touch_through_a_second_mapping_fills_the_line_that_a_load_through_the_first_finds() {
    let w = be_u32(&console_payload("ppu_touch_alias_line"));
    assert_eq!(w.len(), 20);
    assert_eq!(w[0], 0, "no step failed");
    assert_eq!(w[18], 1, "every timed load read its word");
    assert_eq!(w[19], ALIAS_END_MARK);
    // Cases: 0 no touch, 1 a touch through X, 2 dcbt and 3 dcbtst through Y.
    let (smallest, largest) = (&w[6..10], &w[10..14]);
    for case in [2, 3] {
        assert!(
            largest[case] < smallest[0],
            "case {case}: after a touch through Y, a load through X misses no cache"
        );
        assert!(
            smallest[case] <= largest[1],
            "case {case}: as fast as after a touch through X"
        );
    }
    for touch in [dcbt_r7(), dcbtst_r7()] {
        let page = read_write(PAGE_RA);
        let mem = memory(&[touch], &[(PAGE, page), (ALIAS, page)]);
        let mut unit = kernel_unit(ALIAS_TOUCH_EA);
        run_to(&mut unit, &mem, 1, SEGMENT + 4);
        assert_eq!(
            unit.timing.caches().holds(TOUCH_EA, TOUCH_RA),
            (true, true),
            "the L1 set of the line through page 4 and the L2 hold the real line"
        );
    }
}

/// The `ppu_touch_inhibited` payload's word 0 when every case ran.
const INHIBITED_DONE: u64 = 0x444F_4E45;
/// The four pages whose lines fill one L1 set: pages 8, 10, 12 and 14,
/// each 8 KB above the one before. r20 to r23 hold their addresses.
const FILL_PAGES: [u64; 4] = [8, 10, 12, 14];
/// Page 16: a fifth cacheable line of the set.
const CACHEABLE: u64 = 16;
/// Page 18: a caching-inhibited line of the set.
const INHIBITED: u64 = 18;
/// Page 20: the table holds no entry for it.
const UNHELD: u64 = 20;
/// The line that each access names, on its page.
const LINE: u64 = 0x80;

fn page_ea(index: u64) -> u64 {
    SEGMENT + (index << 12)
}

fn page_ra(index: u64) -> u64 {
    0x1_0000 + (index << 12)
}

/// Which lines of [`FILL_PAGES`] the L1 holds after four loads fill the
/// set and a touch of the line on page `touched` follows.
fn fill_lines_held_after_touch(touched: u64, touch: u32) -> [bool; 4] {
    let mut code: Vec<u32> = (0..FILL_PAGES.len() as u8)
        .map(|k| {
            word(I::Lwz {
                rt: 8,
                ra: 20 + k,
                imm: LINE as i16,
            })
        })
        .collect();
    code.push(touch);
    let mut pages: Vec<(u64, u64)> = FILL_PAGES
        .iter()
        .chain([&CACHEABLE])
        .map(|&p| (p, read_write(page_ra(p))))
        .collect();
    pages.push((INHIBITED, read_write(page_ra(INHIBITED)) | PTE_I));
    let mem = memory(&code, &pages);
    let mut unit = kernel_unit(page_ea(touched) + LINE);
    for (k, &p) in FILL_PAGES.iter().enumerate() {
        unit.state_mut().set_gpr(20 + k, page_ea(p));
    }
    run_to(
        &mut unit,
        &mem,
        code.len() as u64,
        SEGMENT + 4 * code.len() as u64,
    );
    let caches = unit.timing.caches();
    FILL_PAGES.map(|p| caches.holds(page_ea(p) + LINE, page_ra(p) + LINE).0)
}

#[test]
fn a_touch_of_caching_inhibited_storage_drops_no_line_of_a_full_set_as_on_the_console() {
    let w = be_u64(&console_payload("ppu_touch_inhibited"));
    assert_eq!(w.len(), 20);
    assert_eq!((w[0], w[1]), (INHIBITED_DONE, 0), "every case ran");
    assert_eq!(w[18], 1, "every chase ended where it should");
    // Cases: 0 no touch, 1 dcbt of a cacheable line, 2 dcbt and 3 dcbtst
    // of a caching-inhibited line, 4 dcbt of an address no page holds.
    let (smallest, largest) = (&w[4..9], &w[9..14]);
    assert!(
        smallest[1] > largest[0],
        "a touch of a cacheable line drops a line of the set"
    );
    for case in [2, 3, 4] {
        assert!(
            smallest[case] <= largest[0] && largest[case] < smallest[1],
            "case {case}: the chase runs as fast as with no touch"
        );
    }
    assert!(
        fill_lines_held_after_touch(CACHEABLE, dcbt_r7()).contains(&false),
        "the cacheable touch drops a line of the set"
    );
    for (touched, touch) in [
        (INHIBITED, dcbt_r7()),
        (INHIBITED, dcbtst_r7()),
        (UNHELD, dcbt_r7()),
    ] {
        assert_eq!(
            fill_lines_held_after_touch(touched, touch),
            [true; 4],
            "a touch of page {touched} drops no line"
        );
    }
}
