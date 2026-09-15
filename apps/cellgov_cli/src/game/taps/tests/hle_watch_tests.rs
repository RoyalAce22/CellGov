//! The HLE return watch's record layouts, spec parsing, and the record
//! stream a call to a watched function produces.

use std::collections::BTreeMap;

use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;

use super::wire::*;
use super::{HleWatch, HleWatchSpec};
use crate::game::taps::error::TapError;
use crate::game::taps::record_file::RecordFile;

fn gpr() -> [u64; 32] {
    let mut g = [0u64; 32];
    for (i, r) in g.iter_mut().enumerate() {
        *r = 0x1000 + i as u64;
    }
    g
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn le64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

#[test]
fn an_entry_record_carries_r3_to_r10_after_the_fixed_fields() {
    let rec = entry(
        7,
        0xAABB_CCDD,
        0x0010_0000,
        0x0010_0000,
        0x0020_0004,
        &gpr(),
    );
    assert_eq!(rec.len(), ENTRY_LEN);
    assert_eq!(rec[0], KIND_ENTRY);
    assert_eq!(le64(&rec, 1), 7);
    assert_eq!(le32(&rec, 9), 0xAABB_CCDD);
    assert_eq!(le32(&rec, 13), 0x0010_0000);
    assert_eq!(le32(&rec, 17), 0x0010_0000);
    assert_eq!(le32(&rec, 21), 0x0020_0004);
    for (i, arg) in (3..=10).enumerate() {
        assert_eq!(le64(&rec, 25 + 8 * i), 0x1000 + arg, "r{arg}");
    }
}

#[test]
fn an_exit_record_pairs_to_its_entry_and_carries_r3() {
    let rec = exit(9, 0x1234_5678, 7, 0x0020_0004, 0xCAFE);
    assert_eq!(rec.len(), EXIT_LEN);
    assert_eq!(rec[0], KIND_EXIT);
    assert_eq!(le64(&rec, 1), 9);
    assert_eq!(le32(&rec, 9), 0x1234_5678);
    assert_eq!(le64(&rec, 13), 7);
    assert_eq!(le32(&rec, 21), 0x0020_0004);
    assert_eq!(le64(&rec, 25), 0xCAFE);
}

#[test]
fn body_records_have_their_declared_lengths_and_kinds() {
    let g = gpr();
    let sc = body_syscall(1, 2, 3, 0x81, 0x100, &g);
    assert_eq!((sc.len(), sc[0]), (BODY_SYSCALL_LEN, KIND_BODY_SYSCALL));
    assert_eq!(le32(&sc, 21), 0x81, "syscall number precedes pc");
    assert_eq!(le32(&sc, 25), 0x100);
    assert_eq!(le64(&sc, 29), 0x1003, "first arg is r3");

    let ret = body_syscall_return(4, 2, 3, 0x81, 0x104, 0xFFFF_FFFF_8001_0002);
    assert_eq!(
        (ret.len(), ret[0]),
        (BODY_SYSCALL_RETURN_LEN, KIND_BODY_SYSCALL_RETURN)
    );
    assert_eq!(le64(&ret, 29), 0xFFFF_FFFF_8001_0002);

    let call = body_call(5, 2, 3, 0x108, 0x0030_0000, &g);
    assert_eq!((call.len(), call[0]), (BODY_CALL_LEN, KIND_BODY_CALL));
    assert_eq!(le32(&call, 21), 0x108);
    assert_eq!(le32(&call, 25), 0x0030_0000, "target follows pc");
    assert_eq!(le64(&call, 29), 0x1003, "first arg is r3");
}

#[test]
fn a_resolution_record_caps_the_name_at_255_bytes() {
    let short = resolution(0xA, 0xB, "sys_ppu_thread_create");
    assert_eq!(short[0], KIND_RESOLUTION);
    assert_eq!(
        short.len(),
        RESOLUTION_HEAD_LEN + "sys_ppu_thread_create".len()
    );
    assert_eq!(
        short[RESOLUTION_HEAD_LEN - 1],
        "sys_ppu_thread_create".len() as u8
    );
    assert_eq!(&short[RESOLUTION_HEAD_LEN..], b"sys_ppu_thread_create");

    let long_name = "x".repeat(300);
    let long = resolution(0xA, 0xB, &long_name);
    assert_eq!(long.len(), RESOLUTION_HEAD_LEN + 255);
    assert_eq!(long[RESOLUTION_HEAD_LEN - 1], 255);
}

#[test]
fn nothing_set_is_no_watch() {
    assert_eq!(HleWatchSpec::parse(None, None, None).unwrap(), None);
    assert_eq!(
        HleWatchSpec::parse(Some(""), Some(" , "), Some("")).unwrap(),
        None
    );
}

#[test]
fn nids_and_raw_pcs_parse_with_or_without_the_hex_prefix() {
    let spec = HleWatchSpec::parse(
        Some("0xE6F2C1E7, 9a0e0d6e"),
        Some("10010=entry_a,0X10020=entry_b"),
        Some("watch.bin"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(spec.nids, vec![0xE6F2_C1E7, 0x9A0E_0D6E]);
    assert_eq!(
        spec.raw_pcs,
        vec![
            (0x10010, "entry_a".to_string()),
            (0x10020, "entry_b".to_string())
        ]
    );
}

#[test]
fn a_watch_and_a_path_without_each_other_are_refused() {
    assert!(matches!(
        HleWatchSpec::parse(Some("1234"), None, None),
        Err(TapError::Unpaired { .. })
    ));
    assert!(matches!(
        HleWatchSpec::parse(None, None, Some("watch.bin")),
        Err(TapError::Unpaired { .. })
    ));
}

#[test]
fn a_malformed_token_names_its_variable() {
    let err = HleWatchSpec::parse(Some("zz"), None, Some("w")).unwrap_err();
    assert!(
        err.to_string().starts_with("CELLGOV_HLE_RETURN_WATCH:"),
        "{err}"
    );
    let err = HleWatchSpec::parse(None, Some("10010"), Some("w")).unwrap_err();
    assert!(matches!(err, TapError::BadShape { .. }), "{err}");
    let err = HleWatchSpec::parse(Some("100000000"), None, Some("w")).unwrap_err();
    assert!(matches!(err, TapError::OutOfRange { .. }), "{err}");
}

#[test]
fn a_raw_pc_whose_on_wire_id_is_a_watched_nid_is_refused() {
    let err = HleWatchSpec::parse(Some("80010010"), Some("10010=f"), Some("w")).unwrap_err();
    assert!(matches!(
        err,
        TapError::RawPcCollides {
            pc: 0x10010,
            id: 0x8001_0010
        }
    ));
}

#[test]
fn the_header_lists_nids_then_raw_pc_ids() {
    let spec = HleWatchSpec::parse(Some("AA"), Some("10=f"), Some("w"))
        .unwrap()
        .unwrap();
    let h = spec.header();
    assert_eq!(&h[0..4], b"CGHW");
    assert_eq!(le32(&h, 4), 1);
    assert_eq!(le32(&h, 8), 2);
    assert_eq!(le32(&h, 12), 0xAA);
    assert_eq!(le32(&h, 16), 0x8000_0010);
}

const ENTRY_PC: u64 = 0x1_0000;
const RETURN_PC: u64 = 0x2_0004;

fn watch(spec: &HleWatchSpec) -> HleWatch<Vec<u8>> {
    HleWatch::new(spec, RecordFile::over("test", Vec::new(), &[]).unwrap())
}

fn at(pc: u64, lr: u64) -> PpuState {
    let mut s = PpuState::new();
    s.pc = pc;
    s.set_lr(lr);
    for r in 3..=11 {
        s.set_gpr(r, 0x100 + r as u64);
    }
    s
}

/// Split a record stream into `(kind, bytes)` by each kind's length.
fn records(mut bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    while let Some(&kind) = bytes.first() {
        let len = match kind {
            KIND_ENTRY => ENTRY_LEN,
            KIND_EXIT => EXIT_LEN,
            KIND_BODY_SYSCALL => BODY_SYSCALL_LEN,
            KIND_BODY_SYSCALL_RETURN => BODY_SYSCALL_RETURN_LEN,
            KIND_BODY_CALL => BODY_CALL_LEN,
            KIND_RESOLUTION => RESOLUTION_HEAD_LEN + bytes[RESOLUTION_HEAD_LEN - 1] as usize,
            other => panic!("unknown record kind {other}"),
        };
        out.push((kind, bytes[..len].to_vec()));
        bytes = &bytes[len..];
    }
    out
}

#[test]
fn a_watched_call_records_entry_body_events_and_exit_in_order() {
    let spec = HleWatchSpec::parse(None, Some("10000=f"), Some("w"))
        .unwrap()
        .unwrap();
    let mut w = watch(&spec);
    let sc = PpuInstruction::Sc { lev: 0 };
    let bl = PpuInstruction::B {
        offset: 0x100,
        aa: false,
        link: true,
    };
    // Entry at the watched PC, a bl inside the body, an sc and the
    // instruction after it, then the return to the caller.
    w.dispatch(&PpuInstruction::Consumed, &at(ENTRY_PC, RETURN_PC));
    w.dispatch(&bl, &at(ENTRY_PC + 4, RETURN_PC));
    w.dispatch(&sc, &at(ENTRY_PC + 8, RETURN_PC));
    w.dispatch(&PpuInstruction::Consumed, &at(ENTRY_PC + 12, RETURN_PC));
    w.dispatch(&PpuInstruction::Consumed, &at(RETURN_PC, 0));

    let recs = records(&w.into_inner());
    let kinds: Vec<u8> = recs.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        kinds,
        vec![
            KIND_RESOLUTION,
            KIND_ENTRY,
            KIND_BODY_CALL,
            KIND_BODY_SYSCALL,
            KIND_BODY_SYSCALL_RETURN,
            KIND_EXIT
        ]
    );
    let entry_no = le64(&recs[1].1, 1);
    assert_eq!(
        le32(&recs[2].1, 25),
        (ENTRY_PC + 4 + 0x100) as u32,
        "bl target"
    );
    assert_eq!(le64(&recs[3].1, 13), entry_no, "sc pairs to the entry");
    assert_eq!(
        le32(&recs[4].1, 25),
        (ENTRY_PC + 12) as u32,
        "sc returns at pc + 4"
    );
    assert_eq!(le64(&recs[5].1, 13), entry_no, "exit pairs to the entry");
    assert_eq!(le32(&recs[5].1, 21), RETURN_PC as u32);
}

#[test]
fn body_events_outside_a_watched_call_write_nothing() {
    let spec = HleWatchSpec::parse(None, Some("10000=f"), Some("w"))
        .unwrap()
        .unwrap();
    let mut w = watch(&spec);
    w.dispatch(&PpuInstruction::Sc { lev: 0 }, &at(0x3_0000, 0));
    let recs = records(&w.into_inner());
    assert_eq!(recs.len(), 1, "only the raw-PC resolution record");
}

fn exports(pairs: &[(&str, u32, u32)]) -> BTreeMap<String, BTreeMap<u32, u32>> {
    let mut map: BTreeMap<String, BTreeMap<u32, u32>> = BTreeMap::new();
    for &(lib, nid, opd) in pairs {
        map.entry(lib.to_string()).or_default().insert(nid, opd);
    }
    map
}

#[test]
fn a_nid_one_library_exports_resolves_through_its_opd() {
    let spec = HleWatchSpec::parse(Some("AA"), None, Some("w"))
        .unwrap()
        .unwrap();
    let mut w = watch(&spec);
    let lines = w.bind(&exports(&[("libA", 0xAA, 0x9000)]), |opd| {
        (opd == 0x9000).then_some(ENTRY_PC as u32)
    });
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("resolved NID 0x000000aa"),
        "{}",
        lines[0]
    );

    w.dispatch(&PpuInstruction::Consumed, &at(ENTRY_PC, RETURN_PC));
    let kinds: Vec<u8> = records(&w.into_inner()).iter().map(|(k, _)| *k).collect();
    assert_eq!(kinds, vec![KIND_RESOLUTION, KIND_ENTRY]);
}

#[test]
fn a_nid_several_libraries_export_stays_unwatched() {
    let spec = HleWatchSpec::parse(Some("AA"), None, Some("w"))
        .unwrap()
        .unwrap();
    let mut w = watch(&spec);
    let lines = w.bind(
        &exports(&[("_cellAudio", 0xAA, 0x9000), ("cellAudio", 0xAA, 0x9100)]),
        |_| Some(ENTRY_PC as u32),
    );
    assert!(lines[0].contains("_cellAudio, cellAudio"), "{}", lines[0]);
    w.dispatch(&PpuInstruction::Consumed, &at(ENTRY_PC, RETURN_PC));
    assert!(w.into_inner().is_empty());
}

#[test]
fn a_nid_resolves_once_across_firmware_sets() {
    let spec = HleWatchSpec::parse(Some("AA"), None, Some("w"))
        .unwrap()
        .unwrap();
    let mut w = watch(&spec);
    w.bind(&exports(&[("libA", 0xAA, 0x9000)]), |_| Some(0x1000));
    let same = w.bind(&exports(&[("libA", 0xAA, 0x9000)]), |_| Some(0x1000));
    assert!(same.is_empty(), "{same:?}");
    let moved = w.bind(&exports(&[("libA", 0xAA, 0x9000)]), |_| Some(0x2000));
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert!(
        moved[0].contains("0x00001000") && moved[0].contains("0x00002000 is not watched"),
        "{}",
        moved[0]
    );
    let kinds: Vec<u8> = records(&w.into_inner()).iter().map(|(k, _)| *k).collect();
    assert_eq!(kinds, vec![KIND_RESOLUTION]);
}

#[test]
fn a_nid_listed_twice_binds_and_reports_once() {
    let spec = HleWatchSpec::parse(Some("AA,0xaa"), None, Some("w"))
        .unwrap()
        .unwrap();
    let mut w = watch(&spec);
    let lines = w.bind(&exports(&[("libA", 0xAA, 0x9000)]), |_| Some(0x1000));
    assert_eq!(lines.len(), 1, "{lines:?}");
    let kinds: Vec<u8> = records(&w.into_inner()).iter().map(|(k, _)| *k).collect();
    assert_eq!(kinds, vec![KIND_RESOLUTION]);
}

#[test]
fn raw_pcs_sharing_an_on_wire_id_are_refused() {
    for pcs in ["10010=f,10010=g", "10010=f,80010010=g"] {
        let err = HleWatchSpec::parse(None, Some(pcs), Some("w")).unwrap_err();
        assert!(
            matches!(
                err,
                TapError::RawPcsCollide {
                    first: 0x10010,
                    id: 0x8001_0010,
                    ..
                }
            ),
            "{pcs}: {err}"
        );
    }
}

#[test]
fn a_raw_pc_name_the_resolution_record_cannot_carry_is_refused() {
    let fits = format!("10010={}", "n".repeat(255));
    assert!(HleWatchSpec::parse(None, Some(&fits), Some("w")).is_ok());
    let over = format!("10010={}", "n".repeat(256));
    assert!(matches!(
        HleWatchSpec::parse(None, Some(&over), Some("w")),
        Err(TapError::BadShape { .. })
    ));
}

#[test]
fn a_retried_entry_instruction_records_one_entry_and_one_exit() {
    let spec = HleWatchSpec::parse(None, Some("10000=f"), Some("w"))
        .unwrap()
        .unwrap();
    let mut w = watch(&spec);
    let store = PpuInstruction::Consumed;
    // A full store buffer sends the entry instruction back once.
    w.dispatch(&store, &at(ENTRY_PC, RETURN_PC));
    w.dispatch(&store, &at(ENTRY_PC, RETURN_PC));
    w.dispatch(&store, &at(ENTRY_PC + 4, RETURN_PC));
    w.dispatch(&store, &at(RETURN_PC, 0));
    // A second visit to the return PC finds no stale frame to pop.
    w.dispatch(&store, &at(RETURN_PC, 0));
    let kinds: Vec<u8> = records(&w.into_inner()).iter().map(|(k, _)| *k).collect();
    assert_eq!(kinds, vec![KIND_RESOLUTION, KIND_ENTRY, KIND_EXIT]);
}

#[test]
fn a_recursive_call_after_a_body_event_is_a_second_entry() {
    let spec = HleWatchSpec::parse(None, Some("10000=f"), Some("w"))
        .unwrap()
        .unwrap();
    let mut w = watch(&spec);
    let bl = PpuInstruction::B {
        offset: -4,
        aa: false,
        link: true,
    };
    w.dispatch(&PpuInstruction::Consumed, &at(ENTRY_PC, RETURN_PC));
    w.dispatch(&bl, &at(ENTRY_PC + 4, RETURN_PC));
    w.dispatch(&PpuInstruction::Consumed, &at(ENTRY_PC, ENTRY_PC + 8));
    let kinds: Vec<u8> = records(&w.into_inner()).iter().map(|(k, _)| *k).collect();
    assert_eq!(
        kinds,
        vec![KIND_RESOLUTION, KIND_ENTRY, KIND_BODY_CALL, KIND_ENTRY]
    );
}
