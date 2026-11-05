//! System-call buffers across the edge between two 64 KiB shared-memory
//! segments mapped end to end, checked against the `lv2_span_shm_buffers`
//! console capture.
//!
//! On the console the kernel reads a mutex attribute, and an SPU image
//! path, across the edge, and writes a Raw SPU interrupt mask across it.

use cellgov_mem::{ByteRange, GuestAddr, PageSize, Region, RegionAccess};
use cellgov_ps3_abi::lv2::errno::{CELL_EINVAL, CELL_ENOENT};
use cellgov_ps3_abi::lv2::spu::raw_window;
use cellgov_ps3_abi::lv2::sync::{
    SYS_SYNC_NOT_ADAPTIVE, SYS_SYNC_NOT_PROCESS_SHARED, SYS_SYNC_NOT_RECURSIVE, SYS_SYNC_PRIORITY,
};
use cellgov_ps3_abi::lv2::syscall as sc;

use crate::span_harness::{
    addi, addis, call, exit, li, lwz, oris, payload, run, sldi32, stw, word, OUT,
};

/// The done marker, "SEAM".
const DONE: u32 = 0x5345_414D;
/// The value the probe writes for a step it did not run.
const SKIP: u32 = 0xDEAD_D00D;
/// A pshared word no mutex has.
const PSHARED_UNKNOWN: u32 = 0x12345;
/// The probe's fill of an out word before a call.
const FILL: u32 = 0xA5A5_A5A5;
/// The absent file the probe opens as an SPU image.
const ABSENT: &[u8] = b"/dev_hdd0/tmp/cgov_span_absent.elf";

/// Where the program stores the payload word `i`.
fn slot(i: usize) -> i16 {
    (OUT + 0x100 + 4 * i as u64) as i16
}

/// The call `num` with `args`, its return code stored as payload word `i`.
fn call_to(p: &mut Vec<u32>, num: u64, args: &[u32], i: usize) {
    call(p, num, args, 3);
    p.push(stw(3, slot(i), 0));
}

/// Stores `bytes`, zero-padded to whole words, at `d` from `base`.
fn put(p: &mut Vec<u32>, bytes: &[u8], base: u32, d: i16) {
    for (k, chunk) in bytes.chunks(4).enumerate() {
        let mut w = [0u8; 4];
        w[..chunk.len()].copy_from_slice(chunk);
        word(p, 6, u32::from_be_bytes(w));
        p.push(stw(6, d + 4 * k as i16, base));
    }
}

/// The mutex attribute block with `pshared`, as bytes.
fn mutex_attr(pshared: u32) -> Vec<u8> {
    let mut words = [0u32; 10];
    words[0] = SYS_SYNC_PRIORITY;
    words[1] = SYS_SYNC_NOT_RECURSIVE;
    words[2] = pshared;
    words[3] = SYS_SYNC_NOT_ADAPTIVE;
    words[7] = u32::from_be_bytes(*b"seam");
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// The probe's steps as one thread. r20 holds the area base, r21 and r22
/// the two segments, r23 the edge, r24 the Raw SPU id, r25 an out
/// pointer. Every return code lands in the payload slot the probe gives
/// it.
fn program() -> Vec<u32> {
    let mut p = Vec::new();
    let mut args = Vec::new();
    word(&mut args, 3, 0x1000_0000);
    args.push(li(4, 0x200));
    word(&mut args, 5, 0x1000_0000);
    args.push(li(6, OUT as i16));
    call_to(&mut p, sc::MMAPPER_ALLOCATE_ADDRESS, &args, 2);
    for (out, i) in [(4i16, 4), (8, 5)] {
        // A keyless 64 KiB segment of 64 KiB granularity.
        let args = [
            addis(3, 0, 0xFFFF),
            sldi32(3, 3),
            li(4, 0),
            oris(4, 4, 1),
            li(5, 0x200),
            li(6, OUT as i16 + out),
        ];
        call_to(&mut p, sc::MMAPPER_ALLOCATE_SHARED_MEMORY, &args, i);
    }
    p.push(lwz(20, OUT as i16, 0));
    p.push(lwz(21, OUT as i16 + 4, 0));
    p.push(lwz(22, OUT as i16 + 8, 0));
    p.push(stw(20, slot(3), 0));
    let map = |at: u32, seg: u32| [at, addi(4, seg, 0), addis(5, 0, 4), li(6, 0)];
    call_to(
        &mut p,
        sc::MMAPPER_MAP_SHARED_MEMORY,
        &map(addi(3, 20, 0), 21),
        6,
    );
    call_to(
        &mut p,
        sc::MMAPPER_MAP_SHARED_MEMORY,
        &map(addis(3, 20, 1), 22),
        7,
    );
    p.push(addis(23, 20, 1));

    // The attribute at the edge - 8, then a destroy of what was created.
    let mutex_id = OUT as i16 + 0x10;
    put(&mut p, &mutex_attr(SYS_SYNC_NOT_PROCESS_SHARED), 23, -8);
    call_to(
        &mut p,
        sc::MUTEX_CREATE,
        &[li(3, mutex_id), addi(4, 23, -8)],
        8,
    );
    call_to(&mut p, sc::MUTEX_DESTROY, &[lwz(3, mutex_id, 0)], 9);
    put(&mut p, &mutex_attr(PSHARED_UNKNOWN), 23, -8);
    call_to(
        &mut p,
        sc::MUTEX_CREATE,
        &[li(3, mutex_id), addi(4, 23, -8)],
        10,
    );

    // The path in main memory, then at the edge - 12.
    let image = OUT as i16 + 0x40;
    let here = OUT as i16 + 0x200;
    put(&mut p, ABSENT, 0, here);
    call_to(&mut p, sc::SPU_IMAGE_OPEN, &[li(3, image), li(4, here)], 12);
    put(&mut p, ABSENT, 23, -12);
    call_to(
        &mut p,
        sc::SPU_IMAGE_OPEN,
        &[li(3, image), addi(4, 23, -12)],
        13,
    );

    // The interrupt mask into main memory, the edge - 20 and the edge - 4.
    call_to(&mut p, sc::SPU_INITIALIZE, &[li(3, 6), li(4, 1)], 14);
    let raw_id = OUT as i16 + 0x50;
    call_to(&mut p, sc::RAW_SPU_CREATE, &[li(3, raw_id), li(4, 0)], 15);
    p.push(lwz(24, raw_id, 0));
    for (target, i) in [
        (li(25, OUT as i16 + 0x60), 16),
        (addi(25, 23, -20), 19),
        (addi(25, 23, -4), 22),
    ] {
        p.push(target);
        word(&mut p, 6, FILL);
        p.push(stw(6, 0, 25));
        p.push(stw(6, 4, 25));
        call_to(
            &mut p,
            sc::RAW_SPU_GET_INT_MASK,
            &[addi(3, 24, 0), li(4, 2), addi(5, 25, 0)],
            i,
        );
        for k in 0..2 {
            p.push(lwz(6, 4 * k as i16, 25));
            p.push(stw(6, slot(i + 1 + k), 0));
        }
    }
    call_to(&mut p, sc::RAW_SPU_DESTROY, &[addi(3, 24, 0)], 25);

    let unmap = |at: u32| [at, li(4, OUT as i16 + 0x70)];
    call_to(
        &mut p,
        sc::MMAPPER_UNMAP_SHARED_MEMORY,
        &unmap(addi(3, 23, 0)),
        26,
    );
    call_to(
        &mut p,
        sc::MMAPPER_UNMAP_SHARED_MEMORY,
        &unmap(addi(3, 20, 0)),
        27,
    );
    call_to(
        &mut p,
        sc::MMAPPER_FREE_SHARED_MEMORY,
        &[addi(3, 22, 0)],
        28,
    );
    call_to(
        &mut p,
        sc::MMAPPER_FREE_SHARED_MEMORY,
        &[addi(3, 21, 0)],
        29,
    );
    call_to(&mut p, sc::MMAPPER_FREE_ADDRESS, &[addi(3, 20, 0)], 30);
    exit(&mut p);
    p
}

/// Main memory and the reserved SPU range, as the boot gives them.
fn regions() -> Vec<Region> {
    vec![
        Region::new(0, 0x2_0000, "main", PageSize::Page64K),
        Region::with_access(
            raw_window::BASE,
            raw_window::STRIDE as usize,
            "spu_reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        )
        .with_caching_inhibited(),
    ]
}

#[test]
fn a_mutex_attribute_a_path_and_an_out_word_across_two_maps_end_to_end_read_and_write_as_on_the_console(
) {
    let w = payload("lv2_span_shm_buffers", 32);
    assert_eq!(w[0..2], [DONE, 0], "the main thread finished");
    assert_eq!(w[8..10], [0, 0], "the attribute across the edge is read");
    assert_eq!(
        w[10..12],
        [CELL_EINVAL.code, SKIP],
        "the pshared word past the edge is read"
    );
    assert_eq!(w[12..14], [CELL_ENOENT.code; 2], "both paths are read");
    assert_eq!(w[22], 0, "the mask is written across the edge");
    assert_ne!(w[23..25], [FILL; 2], "the mask bytes landed");
    assert_eq!(w[23..25], w[17..19], "the same mask as the aligned read");

    let ran = run(regions(), &program());
    assert_eq!(ran.first_fault, None);
    let read = |i: usize| {
        let range = ByteRange::new(GuestAddr::new(OUT + 0x100 + 4 * i as u64), 4).expect("range");
        let bytes = ran.rt.memory().read(range).expect("the payload slot");
        u32::from_be_bytes(bytes[..].try_into().expect("4"))
    };
    for (i, &console) in w.iter().enumerate().take(31).skip(2) {
        if i == 11 {
            // The probe destroys no mutex the create refused.
            continue;
        }
        assert_eq!(read(i), console, "payload word {i}");
    }
}
