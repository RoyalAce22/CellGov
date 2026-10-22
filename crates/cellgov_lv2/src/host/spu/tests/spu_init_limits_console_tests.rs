//! SPU thread group, initialize and image import limits held to the
//! `spu_group_init_limits`, `spu_image_nsegs_negative`,
//! `spu_image_import_raw` and `spu_image_close_edges` captures from the
//! reference console, which a game process took. Each call runs on a
//! fresh host, as the probe ran it on a fresh group, and its return code
//! is compared with the captured word.
//!
//! Slots 18-20 of `spu_group_init_limits` are the answers of firmware
//! liblv2's `sys_spu_image_import` wrapper, which checks the buffer
//! before it issues any syscall; they are not replayed here. The raw
//! syscall's answers are `spu_image_import_raw`.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::format::elf::ELF_MAGIC;
use cellgov_ps3_abi::lv2::spu::{image, segment};

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::request::{classify, Lv2Request};

const IMG: u32 = 0x200;
const SEGS: u32 = 0x300;
const CODE: u32 = 0x1000;
const ELF_AT: u32 = 0x2000;
const NOT_ELF_AT: u32 = 0x3000;
const BUFFER: u32 = 0x400;
const FILL_LS: u32 = 0x30000;
const MEMORY: usize = 0x8000;
/// The syscall numbers `spu_image_import_raw` sends for import and
/// close, as its ppu/main.c defines them.
const PROBE_IMPORT: u64 = 157;
const PROBE_CLOSE: u64 = 158;
/// Stands in for the probes' null pointers: the test runtime maps
/// address 0.
const UNMAPPED: u32 = 0x7000_0000;

fn commit(mem: &mut GuestMemory, addr: u32, bytes: &[u8]) {
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(addr)), bytes.len() as u64).unwrap(),
        bytes,
    )
    .unwrap();
}

fn words(ws: &[u32]) -> Vec<u8> {
    ws.iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// The code and the u32 of the first write.
fn answer(d: Lv2Dispatch) -> (u32, Option<u32>) {
    match d {
        Lv2Dispatch::Immediate { code, effects } => {
            let written = effects.iter().find_map(|e| match e {
                Effect::SharedWriteIntent { bytes, .. } => Some(u32::from_be_bytes(
                    bytes.bytes()[..4].try_into().expect("a u32 write"),
                )),
                _ => None,
            });
            (code as u32, written)
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

/// A user record at `IMG` over `segs`, each `(type, ls, size, source)`.
fn user_image(segs: &[[u32; 4]], nsegs: u32) -> FakeRuntime {
    let mut mem = GuestMemory::new(MEMORY);
    commit(
        &mut mem,
        IMG,
        &words(&[image::TYPE_USER, 0xb0, SEGS, nsegs]),
    );
    for (i, &[kind, ls, size, source]) in segs.iter().enumerate() {
        commit(
            &mut mem,
            SEGS + i as u32 * segment::LEN,
            &words(&[kind, ls, size, 0, source, 0]),
        );
    }
    FakeRuntime::with_memory(mem)
}

/// The blob, then `fills` FILLs of 16 bytes.
fn blob_and_fills(fills: u32) -> Vec<[u32; 4]> {
    let mut segs = vec![[segment::TYPE_COPY, 0, 0x10, CODE]];
    segs.extend((0..fills).map(|i| [segment::TYPE_FILL, FILL_LS + 16 * i, 16, 0x5A5A_5A5A]));
    segs
}

fn group_create(host: &mut Lv2Host, rt: &FakeRuntime, num: u32) -> (u32, Option<u32>) {
    answer(host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads: num,
            priority: 100,
            attr_ptr: 0,
        },
        UnitId::new(0),
        rt,
    ))
}

/// Destroys `group` and folds the code into `first_failure` as the
/// probes do: the code of the first destroy that failed, else 0.
fn destroy(host: &mut Lv2Host, rt: &FakeRuntime, group: u32, first_failure: &mut u32) {
    let (code, _) = answer(host.dispatch(
        Lv2Request::SpuThreadGroupDestroy { id: group },
        UnitId::new(0),
        rt,
    ));
    if code != 0 && *first_failure == 0 {
        *first_failure = code;
    }
}

/// 172 into a fresh group of `num` threads at `slot` with the record at
/// `img_ptr`; the group is destroyed again.
fn init(rt: &FakeRuntime, num: u32, slot: u32, img_ptr: u32, destroys: &mut u32) -> u32 {
    let mut host = Lv2Host::new();
    let (code, id) = group_create(&mut host, rt, num);
    assert_eq!(code, 0, "group create of {num}");
    let group = id.expect("the group id");
    let code = init_on(&mut host, rt, group, slot, img_ptr);
    destroy(&mut host, rt, group, destroys);
    code
}

fn init_on(host: &mut Lv2Host, rt: &FakeRuntime, group: u32, slot: u32, img_ptr: u32) -> u32 {
    answer(host.dispatch(
        Lv2Request::SpuThreadInitialize {
            thread_ptr: 0x180,
            group_id: group,
            thread_num: slot,
            img_ptr,
            attr_ptr: 0,
            arg_ptr: 0,
        },
        UnitId::new(0),
        rt,
    ))
    .0
}

#[test]
fn group_create_and_initialize_limits_answer_as_on_the_console() {
    let words = console_payload("spu_group_init_limits");
    let w = |slot: usize| payload_u32(&words, 4 * slot);
    let rt = user_image(&blob_and_fills(0), 1);
    let mut destroys = 0;

    for (slot, num) in [(0, 0), (1, 64), (2, 256), (3, 257)] {
        let mut host = Lv2Host::new();
        let (code, id) = group_create(&mut host, &rt, num);
        assert_eq!(code, w(slot), "slot {slot}: 170 num {num}");
        if code == 0 {
            destroy(&mut host, &rt, id.expect("the group id"), &mut destroys);
        }
    }

    for (slot, num, thread, img) in [
        (4, 1, 1, IMG),
        (5, 2, 2, IMG),
        (6, 1, 255, IMG),
        (7, 1, 256, IMG),
        (8, 1, 300, IMG),
        (9, 1, 300, UNMAPPED),
        (10, 1, 0, UNMAPPED),
        (11, 1, 1, UNMAPPED),
        (17, 1, 0, IMG),
    ] {
        assert_eq!(
            init(&rt, num, thread, img, &mut destroys),
            w(slot),
            "slot {slot}: 172 group {num} slot {thread} image {img:#x}"
        );
    }

    let segment_cases: [(usize, Vec<[u32; 4]>, u32); 3] = [
        (12, blob_and_fills(0), 0),
        (13, blob_and_fills(31), 0x20),
        (14, blob_and_fills(32), 0x21),
    ];
    for (slot, segs, nsegs) in segment_cases {
        assert_eq!(
            init(&user_image(&segs, nsegs), 1, 0, IMG, &mut destroys),
            w(slot),
            "slot {slot}: nsegs {nsegs:#x}"
        );
    }
    for (slot, size) in [(15, 256), (16, 257)] {
        let segs = [
            [segment::TYPE_COPY, 0, 0x10, CODE],
            [segment::TYPE_INFO, 0, size, CODE],
        ];
        assert_eq!(
            init(&user_image(&segs, 2), 1, 0, IMG, &mut destroys),
            w(slot),
            "slot {slot}: INFO of {size} bytes"
        );
    }
    assert_eq!(destroys, w(22), "slot 22: the first destroy that failed");
}

/// Memory holding an "ELF" of `elf_size` bytes (the magic, then zeros)
/// and a buffer that is not one.
fn import_memory(elf_size: u32) -> GuestMemory {
    let mut mem = GuestMemory::new(MEMORY);
    let mut elf = vec![0u8; elf_size as usize];
    elf[..4].copy_from_slice(&ELF_MAGIC);
    commit(&mut mem, ELF_AT, &elf);
    commit(&mut mem, NOT_ELF_AT, &[0x40, 0x80, 0x00, 0x03]);
    mem
}

/// 157 from `src`: the code and the bytes it wrote.
fn import(host: &mut Lv2Host, rt: &FakeRuntime, out: u32, src: u32, size: u32) -> (u32, Vec<u8>) {
    let request = classify(
        PROBE_IMPORT,
        &[
            u64::from(out),
            u64::from(src),
            u64::from(size),
            0,
            0,
            0,
            0,
            0,
        ],
    );
    match host.dispatch(request, UnitId::new(0), rt) {
        Lv2Dispatch::Immediate { code, effects } => {
            let written = effects
                .iter()
                .find_map(|e| match e {
                    Effect::SharedWriteIntent { bytes, .. } => Some(bytes.bytes().to_vec()),
                    _ => None,
                })
                .unwrap_or_default();
            (code as u32, written)
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn close(host: &mut Lv2Host, rt: &FakeRuntime, img: u32) -> u32 {
    answer(host.dispatch(
        classify(PROBE_CLOSE, &[u64::from(img), 0, 0, 0, 0, 0, 0, 0]),
        UnitId::new(0),
        rt,
    ))
    .0
}

/// The raw 157 / 158 run: the first import, its refusals, a thread
/// initialized from the imported record, and the closes.
#[test]
fn raw_image_import_and_close_answer_as_on_the_console() {
    let words = console_payload("spu_image_import_raw");
    let w = |slot: usize| payload_u32(&words, 4 * slot);
    let elf_size = w(18);
    let rt = FakeRuntime::with_memory(import_memory(elf_size));

    let mut host = Lv2Host::new();
    let (code, record) = import(&mut host, &rt, IMG, ELF_AT, elf_size);
    assert_eq!(code, w(0), "slot 0");
    assert_eq!(record.len(), 16, "slot 0 writes a 16-byte record");
    let record_word = |i: usize| u32::from_be_bytes(record[4 * i..4 * i + 4].try_into().unwrap());
    assert_eq!(record_word(0), w(1), "slot 1: a kernel record");
    // Slot 2 is the image id the kernel chose; CellGov mints its own.
    assert_eq!((record_word(2), record_word(3)), (w(3), w(4)), "slots 3-4");

    for (slot, out, src, size) in [
        (5, BUFFER, NOT_ELF_AT, 4),
        (6, UNMAPPED, ELF_AT, elf_size),
        (7, BUFFER, UNMAPPED, elf_size),
        (8, BUFFER, ELF_AT, 0),
    ] {
        let (code, _) = import(&mut Lv2Host::new(), &rt, out, src, size);
        assert_eq!(
            code,
            w(slot),
            "slot {slot}: 157 to {out:#x} from {src:#x}, {size} bytes"
        );
    }

    let mut mem = import_memory(elf_size);
    commit(&mut mem, IMG, &record);
    let rt = FakeRuntime::with_memory(mem);
    let mut destroys = 0;
    let (code, group) = group_create(&mut host, &rt, 1);
    assert_eq!(code, 0);
    let group = group.expect("the group id");
    assert_eq!(init_on(&mut host, &rt, group, 0, IMG), w(9), "slot 9");
    destroy(&mut host, &rt, group, &mut destroys);
    assert_eq!(close(&mut host, &rt, IMG), w(10), "slot 10: close");
    assert_eq!(close(&mut host, &rt, IMG), w(11), "slot 11: close again");
    assert_eq!(close(&mut host, &rt, UNMAPPED), w(12), "slot 12: no record");
    let (code, group) = group_create(&mut host, &rt, 1);
    assert_eq!(code, 0);
    let group = group.expect("the group id");
    assert_eq!(
        init_on(&mut host, &rt, group, 0, IMG),
        w(13),
        "slot 13: initialize after the close"
    );
    destroy(&mut host, &rt, group, &mut destroys);
    assert_eq!(destroys, w(17), "slot 17: the first destroy that failed");
}

#[test]
fn a_negative_segment_count_is_refused_as_on_the_console() {
    let payload = console_payload("spu_image_nsegs_negative");
    let w = |slot: usize| payload_u32(&payload, 4 * slot);
    // The one-row table ends where the memory ends, so an arm that walks
    // a count of -1 faults instead of finding an invalid row.
    let mut mem = GuestMemory::new(MEMORY);
    let last_row = MEMORY as u32 - segment::LEN;
    commit(
        &mut mem,
        IMG,
        &words(&[image::TYPE_USER, 0xb0, last_row, u32::MAX]),
    );
    commit(
        &mut mem,
        last_row,
        &words(&[segment::TYPE_COPY, 0, 0x10, 0, CODE, 0]),
    );
    let rt = FakeRuntime::with_memory(mem);
    let mut destroys = 0;
    assert_eq!(
        init(&rt, 1, 0, IMG, &mut destroys),
        w(0),
        "slot 0: nsegs -1"
    );
    let segs = blob_and_fills(0);
    assert_eq!(
        init(&user_image(&segs, 1), 1, 0, IMG, &mut destroys),
        w(1),
        "slot 1: the control"
    );
    assert_eq!(destroys, w(2), "slot 2: the first destroy that failed");
}

/// Two imports of one ELF, the close of one, and the close of a user
/// record, in the probe's order.
#[test]
fn image_close_edges_answer_as_on_the_console() {
    const REC_B: u32 = 0x240;
    const USER: u32 = 0x280;
    let payload = console_payload("spu_image_close_edges");
    let w = |slot: usize| payload_u32(&payload, 4 * slot);
    let elf_size = payload_u32(&console_payload("spu_image_import_raw"), 4 * 18);
    let rt = FakeRuntime::with_memory(import_memory(elf_size));
    let mut host = Lv2Host::new();
    let (code_a, record_a) = import(&mut host, &rt, IMG, ELF_AT, elf_size);
    assert_eq!(code_a, w(0), "slot 0");
    let (code_b, record_b) = import(&mut host, &rt, REC_B, ELF_AT, elf_size);
    assert_eq!(code_b, w(2), "slot 2");
    assert_ne!(
        record_a[4..8],
        record_b[4..8],
        "slots 1 and 3: two ids, as the console's {:#x} and {:#x}",
        w(1),
        w(3)
    );

    let mut mem = import_memory(elf_size);
    commit(&mut mem, IMG, &record_a);
    commit(&mut mem, REC_B, &record_b);
    commit(&mut mem, USER, &words(&[image::TYPE_USER, 0xb0, SEGS, 1]));
    commit(
        &mut mem,
        SEGS,
        &words(&[segment::TYPE_COPY, 0, 0x10, 0, CODE, 0]),
    );
    let rt = FakeRuntime::with_memory(mem);
    let mut destroys = 0;
    let mut init_from = |host: &mut Lv2Host, img: u32| {
        let (code, group) = group_create(host, &rt, 1);
        assert_eq!(code, 0);
        let group = group.expect("the group id");
        let answer = init_on(host, &rt, group, 0, img);
        destroy(host, &rt, group, &mut destroys);
        answer
    };
    assert_eq!(close(&mut host, &rt, IMG), w(4), "slot 4: close A");
    assert_eq!(init_from(&mut host, REC_B), w(5), "slot 5: B with A closed");
    assert_eq!(init_from(&mut host, IMG), w(6), "slot 6: A after its close");
    assert_eq!(close(&mut host, &rt, REC_B), w(7), "slot 7: close B");
    assert_eq!(
        init_from(&mut host, REC_B),
        w(8),
        "slot 8: B after its close"
    );
    assert_eq!(
        close(&mut host, &rt, USER),
        w(9),
        "slot 9: close of a user record"
    );
    assert_eq!(destroys, w(10), "slot 10");
}
