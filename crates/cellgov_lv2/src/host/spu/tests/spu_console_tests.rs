//! SPU thread ABI values held to the reference console's microtest captures.

use crate::dispatch::{Lv2Dispatch, SpuLoadImage};
use crate::host::test_support::{console_payload, payload_u32, FakeRuntime};
use crate::host::Lv2Host;
use crate::image::LsSegment;
use crate::request::Lv2Request;
use crate::spu_stop::{SpuThreadError, SpuThreadStop};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{SPU_STATUS_P, SPU_STATUS_STOP_CODE_SHIFT};
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::spu::{group_join_cause, image, segment, stop_code, thread_window};

fn commit(mem: &mut GuestMemory, addr: u32, bytes: &[u8]) {
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(u64::from(addr)), bytes.len() as u64).unwrap(),
        bytes,
    )
    .unwrap();
}

fn immediate_code(d: &Lv2Dispatch) -> u32 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code as u32,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

/// The path `spu_image_segments` builds for one open case: `length`
/// bytes under `/dev_hdd0/tmp/`, one long name or names of one byte.
fn probe_path(short_names: bool, length: usize) -> Vec<u8> {
    const PREFIX: &[u8] = b"/dev_hdd0/tmp/";
    if length == 0 {
        return Vec::new();
    }
    let mut path = PREFIX.to_vec();
    for i in PREFIX.len()..length {
        let tail = i - PREFIX.len();
        path.push(if short_names && tail % 2 == 1 {
            b'/'
        } else {
            b'a'
        });
    }
    if !short_names {
        path[PREFIX.len()..].fill(b'p');
    }
    if length > PREFIX.len() {
        if let Some(last @ b'/') = path.last_mut() {
            *last = b'a';
        }
    }
    path
}

#[test]
fn image_open_bounds_the_path_as_the_console_does() {
    const PATH: u32 = 0x100;
    let cases: [(bool, usize); 16] = [
        (false, 255),
        (false, 256),
        (false, 269),
        (false, 270),
        (false, 271),
        (false, 1022),
        (false, 1023),
        (false, 1024),
        (false, 1025),
        (false, 2048),
        (true, 1022),
        (true, 1023),
        (true, 1024),
        (true, 1025),
        (true, 2048),
        (false, 0),
    ];
    let payload = console_payload("spu_image_segments");
    assert_eq!(&payload[0..8], &[0; 8], "the probe ran every step");
    for (i, &(short_names, length)) in cases.iter().enumerate() {
        let mut mem = GuestMemory::new(0x1000);
        let mut path = probe_path(short_names, length);
        path.push(0);
        commit(&mut mem, PATH, &path);
        let rt = FakeRuntime::with_memory(mem);
        let mut host = Lv2Host::new();
        let d = host.dispatch(
            Lv2Request::SpuImageOpen {
                img_ptr: 0x10,
                path_ptr: PATH,
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            immediate_code(&d),
            payload_u32(&payload, 32 + 4 * i),
            "case {i}: {length} bytes, short names {short_names}"
        );
    }
}

const IMG: u32 = 0x200;
const SEGS: u32 = 0x300;
const CODE: u32 = 0x1000;
const COPY_SOURCE: u32 = 0x1100;
const DECOY_C: u32 = 0x1200;
const DECOY_14: u32 = 0x1300;
const COPY_LS: u32 = 0x30000;
const FILL_LS: u32 = 0x31000;
const FILL2_LS: u32 = 0x31010;

/// The probe's five segment records, each `(type, ls, size, +0x0C,
/// +0x10, +0x14)`; a stand-in code segment takes the blob's place.
fn probe_segments(test_copy_type: u32) -> [[u32; 6]; 5] {
    [
        [segment::TYPE_COPY, 0, 0x10, 0, CODE, 0],
        [test_copy_type, COPY_LS, 16, DECOY_C, COPY_SOURCE, DECOY_14],
        [
            segment::TYPE_FILL,
            FILL_LS,
            16,
            0xDECA_FBAD,
            0x1122_3344,
            0x5566_7788,
        ],
        [
            segment::TYPE_FILL,
            FILL2_LS,
            16,
            0x7777_7777,
            0x7777_7777,
            0x7777_7777,
        ],
        [segment::TYPE_INFO, 0, 0x20, 0, COPY_SOURCE, 0],
    ]
}

fn probe_runtime(entry: u32, segs: &[[u32; 6]]) -> FakeRuntime {
    let mut mem = GuestMemory::new(0x4000);
    let record: Vec<u8> = [image::TYPE_USER, entry, SEGS, segs.len() as u32]
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect();
    commit(&mut mem, IMG, &record);
    for (i, seg) in segs.iter().enumerate() {
        let row: Vec<u8> = seg.iter().flat_map(|w| w.to_be_bytes()).collect();
        commit(&mut mem, SEGS + i as u32 * segment::LEN, &row);
    }
    for (at, word) in [
        (COPY_SOURCE, [0xC0, 0xC1, 0xC2, 0xC3]),
        (DECOY_C, [0xDC; 4]),
        (DECOY_14, [0xD0; 4]),
    ] {
        let bytes: Vec<u8> = word.iter().flat_map(|&b| [b; 4]).collect();
        commit(&mut mem, at, &bytes);
    }
    FakeRuntime::with_memory(mem)
}

/// Initialize slot 0 of a fresh one-thread group with the record at
/// `IMG`; return the answer and the host, ready for a group start.
fn initialize(rt: &FakeRuntime) -> (u32, Lv2Host) {
    let mut host = Lv2Host::new();
    let created = host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads: 1,
            priority: 100,
            attr_ptr: 0,
        },
        UnitId::new(0),
        rt,
    );
    assert_eq!(immediate_code(&created), 0);
    let d = host.dispatch(
        Lv2Request::SpuThreadInitialize {
            thread_ptr: 0x180,
            group_id: 1,
            thread_num: 0,
            img_ptr: IMG,
            attr_ptr: 0,
            arg_ptr: 0,
        },
        UnitId::new(0),
        rt,
    );
    (immediate_code(&d), host)
}

#[test]
fn a_hand_built_user_image_loads_as_the_console_loads_it() {
    let payload = console_payload("spu_image_segments");
    let einval = errno::CELL_EINVAL.code;
    assert_eq!(payload_u32(&payload, 12), einval, "type 3 on the console");
    let (type3, _) = initialize(&probe_runtime(0xd0, &probe_segments(3)));
    assert_eq!(type3, payload_u32(&payload, 12), "a segment type of 3");
    let code_only = &probe_segments(segment::TYPE_COPY)[..1];
    for (entry, at) in [(image::ENTRY_MAX, 16), (image::ENTRY_MAX + 4, 20)] {
        let (answer, _) = initialize(&probe_runtime(entry, code_only));
        assert_eq!(answer, payload_u32(&payload, at), "entry {entry:#x}");
    }

    let rt = probe_runtime(0xd0, &probe_segments(segment::TYPE_COPY));
    let (answer, mut host) = initialize(&rt);
    assert_eq!(answer, payload_u32(&payload, 8), "the five segments");
    let d = host.dispatch(
        Lv2Request::SpuThreadGroupStart { group_id: 1 },
        UnitId::new(0),
        &rt,
    );
    let Lv2Dispatch::RegisterSpu { inits, code: 0, .. } = d else {
        panic!("expected RegisterSpu, got {d:?}");
    };
    let SpuLoadImage::Segments(segments) = &inits[&0].image else {
        panic!("expected a segment image");
    };
    let loaded = |ls: u32| -> &[u8] {
        let seg: &LsSegment = segments
            .iter()
            .find(|s| s.ls_start == ls)
            .unwrap_or_else(|| panic!("no segment at {ls:#x}"));
        &seg.bytes
    };
    assert_eq!(loaded(COPY_LS), &payload[96..112], "the COPY range");
    assert_eq!(loaded(FILL_LS), &payload[112..128], "the first FILL range");
    assert_eq!(
        loaded(FILL2_LS),
        &payload[128..144],
        "the second FILL range"
    );
}

/// The value `#define <name>` sets in a probe's SPU program.
fn spu_define(probe: &str, name: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("spu/main.c");
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let value = source
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("#define") && words.next() == Some(name))
                .then(|| words.next())
                .flatten()
        })
        .unwrap_or_else(|| panic!("{probe}: no #define {name}"));
    let hex = value.trim_end_matches('u').trim_start_matches("0x");
    u64::from_str_radix(hex, 16).unwrap_or_else(|e| panic!("{name} = {value}: {e}"))
}

#[test]
fn the_thread_window_is_where_the_console_maps_each_slot() {
    const PROBE: &str = "spu_thread_window_slots";
    assert_eq!(spu_define(PROBE, "WINDOW_BASE"), thread_window::BASE);
    assert_eq!(spu_define(PROBE, "WINDOW_STRIDE"), thread_window::STRIDE);
    assert_eq!(
        spu_define(PROBE, "PROBLEM_STATE"),
        thread_window::PROBLEM_STATE
    );
    let payload = console_payload(PROBE);
    assert_eq!(&payload[0..8], &[0; 8], "the probe ran every step");
    for slot in 0..4u32 {
        for word in 0..4u32 {
            let at = 32 + 16 * slot as usize + 4 * word as usize;
            assert_eq!(
                payload_u32(&payload, at),
                0x5A00_0000 | slot << 16 | word,
                "slot {slot} word {word} through its local-store alias"
            );
        }
        let release = if slot == 0 { 0xE } else { 0xB0 + slot };
        assert_eq!(
            (
                payload_u32(&payload, 96 + 16 * slot as usize),
                payload_u32(&payload, 100 + 16 * slot as usize)
            ),
            (slot, release),
            "slot {slot}'s signal through the problem-state alias"
        );
    }
}

#[test]
fn the_join_causes_and_stop_services_are_the_ones_the_console_reports() {
    let window = console_payload("spu_thread_window_slots");
    let causes: Vec<u32> = (2..8).map(|w| payload_u32(&window, 4 * w)).collect();
    assert_eq!(
        causes,
        [
            group_join_cause::ALL_THREADS_EXIT,
            0,
            group_join_cause::GROUP_EXIT,
            0x1234,
            group_join_cause::ALL_THREADS_EXIT,
            0
        ],
        "four threads exit; one calls the group exit with 0x1234; one yields, then exits"
    );
    assert_eq!(payload_u32(&window, 160), 0x5949_4C44, "the yield resumed");

    let stops = console_payload("spu_stop_service_codes");
    assert_eq!(&stops[0..8], &[0; 8], "the probe ran every step");
    let words: Vec<u32> = (2..14).map(|w| payload_u32(&stops, 4 * w)).collect();
    assert_eq!(
        words,
        [
            0,
            0,
            group_join_cause::TERMINATED,
            0x7E57,
            group_join_cause::ALL_THREADS_EXIT,
            0,
            0,
            0x1111_1111,
            0x2222_2222,
            0x3333_3333,
            errno::CELL_EBUSY.code,
            errno::CELL_EINVAL.code,
        ],
        "terminate, the two joins, receive, tryreceive empty and unbound"
    );
    // The console serves both event-queue stops; CellGov does not yet,
    // and stops the thread group on either.
    for code in [stop_code::RECEIVE_EVENT, stop_code::TRY_RECEIVE_EVENT] {
        let status = SPU_STATUS_P | u32::from(code) << SPU_STATUS_STOP_CODE_SHIFT;
        assert_eq!(
            SpuThreadStop::from_status(status),
            SpuThreadStop::Error(SpuThreadError::UnservedStopCode(code)),
            "stop {code:#x}"
        );
    }
}
