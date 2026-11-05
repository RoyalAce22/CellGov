use std::path::{Path, PathBuf};

use cellgov_ps3_abi::hw::ppc_mnemonics::{
    PPE_BOOK_III_INSTRUCTIONS, PPE_UNIMPLEMENTED_INSTRUCTIONS,
};

use super::capture::{file_name, render};
use super::system::*;
use crate::ppu_reference::{
    parse_reference_file, replay_reference, word_unit, PpuReferenceFile, PpuReferenceSet,
    PpuReferenceVector, PpuReferenceYieldReason, ReferenceField,
};
use crate::regenerate::write_if_asked;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ppu_reference")
}

fn committed(file: &str) -> PpuReferenceSet {
    let path = fixtures().join(file);
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    match parse_reference_file(&json).unwrap_or_else(|e| panic!("{}: {e}", path.display())) {
        PpuReferenceFile::Set(set) => set,
        PpuReferenceFile::Single(_) => panic!("{}: a single-vector file", path.display()),
    }
}

/// Every set the system model writes, with its file name.
fn all_sets() -> Vec<(String, PpuReferenceSet)> {
    let mut out: Vec<(String, PpuReferenceSet)> = SYSTEM_INSTRUCTIONS
        .iter()
        .map(|unit| (file_name(unit), instruction_set(unit)))
        .collect();
    out.extend(other_sets().into_iter().map(|(f, s)| (f.to_owned(), s)));
    out
}

#[test]
fn each_system_file_is_its_model_rendered() {
    let mut stale = Vec::new();
    for (file, set) in all_sets() {
        let want = render(&set);
        let path = fixtures().join(&file);
        if write_if_asked(&path, &want) {
            continue;
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        if have.replace("\r\n", "\n") != want {
            stale.push(path.display().to_string());
        }
    }
    assert!(
        stale.is_empty(),
        "stale; rerun with CELLGOV_REGENERATE=1:\n{}",
        stale.join("\n")
    );
}

fn vector<'a>(set: &'a PpuReferenceSet, name: &str) -> &'a PpuReferenceVector {
    set.vectors
        .iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("no vector {name}"))
}

fn value_of<T: Clone + std::fmt::Debug>(field: &ReferenceField<T>) -> T {
    match field {
        ReferenceField::Value { value } => value.clone(),
        other => panic!("not a value: {other:?}"),
    }
}

const BASE: u64 = 0x1000_0000;

#[test]
fn the_reservation_model_follows_the_cited_rules_on_fixed_vectors() {
    // [PPC-Book2 p:24 s:3.3.2] lwarx at the line's last word holds line 0;
    // at 128 it holds line 1.
    let lwarx = committed("lwarx.json");
    for (name, line) in [("lwarx-off124", BASE), ("lwarx-off128", BASE + 128)] {
        let v = vector(&lwarx, name);
        assert_eq!(
            value_of(&v.expected.state.reservation),
            Some(line),
            "{name}"
        );
        assert_eq!(value_of(&v.expected.reservations), [(0, line)], "{name}");
    }
    // [PPC-Book2 p:25 s:3.3.2] stwcx. under line 0 to line 1 drops the
    // reservation and leaves the store and CR0 EQ undefined; to line 0 it
    // stores r3's low word and sets EQ.
    let stwcx = committed("stwcx_dot.json");
    let miss = vector(&stwcx, "stwcx.-other-line");
    assert!(matches!(
        miss.expected.memory,
        ReferenceField::Undefined { .. }
    ));
    assert!(matches!(
        miss.expected.state.cr,
        ReferenceField::Undefined { .. }
    ));
    assert_eq!(value_of(&miss.expected.state.reservation), None);
    let hit = vector(&stwcx, "stwcx.-held-off0");
    let data = value_of(&hit.expected.memory);
    assert_eq!(data[0..4], (hit.initial_state.gpr[&3] as u32).to_be_bytes());
    assert_eq!(data[4..], hit.initial_memory[4..]);
    assert_eq!(value_of(&hit.expected.state.cr) >> 28, 0b0010);
    let so = vector(&stwcx, "stwcx.-held-so");
    assert_eq!(value_of(&so.expected.state.cr) >> 28, 0b0011);
    // With no reservation it stores nothing and clears EQ.
    let none = vector(&stwcx, "stwcx.-none");
    assert_eq!(value_of(&none.expected.memory), none.initial_memory);
    assert_eq!(value_of(&none.expected.state.cr) >> 28, 0b0000);
    assert_eq!(value_of(&none.expected.state.reservation), None);
}

#[test]
fn each_reservation_edges_vector_stores_and_sets_eq_as_the_console_did() {
    use cellgov_testkit::console_cases::{probe_dir, PROFILE};
    let path = probe_dir("ppu_reservation_edges")
        .join("ps3")
        .join(PROFILE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let payload = &frame[8..];
    assert_eq!(payload.len(), 4 * (3 + 3 * 18), "payload size");
    let word = |n: usize| u32::from_be_bytes(payload[4 * n..4 * n + 4].try_into().expect("4"));
    assert_eq!(word(0), 0, "the probe failed a step");
    let trials = word(2);
    let stwcx = instruction_set("stwcx.");
    let stdcx = instruction_set("stdcx.");
    let facility = other_sets()
        .into_iter()
        .find(|(file, _)| *file == "facility_reservation.json")
        .expect("the reservation facility set")
        .1;
    // The vector, the probe's case, the store's offset and width.
    let cases: [(&PpuReferenceSet, &str, usize, usize, usize); 16] = [
        (&stwcx, "stwcx.-after-reserve-off0-console", 0, 0, 4),
        (&stwcx, "stwcx.-after-reserve-off4-console", 1, 4, 4),
        (&stwcx, "stwcx.-after-reserve-off64-console", 2, 64, 4),
        (&stwcx, "stwcx.-after-reserve-off124-console", 3, 124, 4),
        (&stwcx, "stwcx.-after-reserve-off128-console", 4, 128, 4),
        (&stdcx, "stdcx.-after-reserve-off0-console", 5, 0, 8),
        (&stdcx, "stdcx.-after-reserve-off8-console", 6, 8, 8),
        (&stdcx, "stdcx.-after-reserve-off64-console", 7, 64, 8),
        (&stdcx, "stdcx.-after-reserve-off120-console", 8, 120, 8),
        (&stdcx, "stdcx.-after-reserve-off128-console", 9, 128, 8),
        (&stdcx, "stdcx.-after-other-width-console", 10, 0, 8),
        (&stwcx, "stwcx.-after-other-width-console", 11, 0, 4),
        (&stwcx, "stwcx.-after-dcbf-console", 12, 0, 4),
        (&stwcx, "stwcx.-after-dcbst-console", 13, 0, 4),
        (&facility, "granule-off124-console", 3, 124, 4),
        (&facility, "granule-off128-console", 4, 128, 4),
    ];
    let mut stored_cases = 0;
    for (set, name, case, off, width) in cases {
        let v = vector(set, name);
        let (eq, held) = (word(3 + 3 * case), word(4 + 3 * case));
        assert!(
            eq == 0 || eq == trials,
            "{name}: EQ in {eq} of {trials} trials"
        );
        assert!(
            held == 0 || held == trials,
            "{name}: stored in {held} of {trials}"
        );
        let cr = value_of(&v.expected.state.cr);
        assert_eq!((cr >> 29) & 1 == 1, eq == trials, "{name}: CR0 EQ");
        let after = value_of(&v.expected.memory);
        let rs = v.initial_state.gpr[&3].to_be_bytes();
        let stored = after[off..off + width] == rs[8 - width..];
        assert_eq!(stored, held == trials, "{name}: the store");
        if stored {
            stored_cases += 1;
        } else {
            assert_eq!(after, v.initial_memory, "{name}: a failed store wrote");
        }
    }
    assert_eq!(stored_cases, 13);
}

#[test]
fn each_dcbz_vector_zeroes_one_aligned_block_of_128_bytes() {
    let mut checked = 0;
    let sets = all_sets();
    for file in ["dcbz.json", "facility_dcbz_width.json"] {
        let (_, set) = sets
            .iter()
            .find(|(f, _)| f == file)
            .unwrap_or_else(|| panic!("no set {file}"));
        for v in set.vectors.clone() {
            let after = value_of(&v.expected.memory);
            let zeroed: Vec<usize> = (0..after.len())
                .filter(|&i| after[i] != v.initial_memory[i])
                .collect();
            assert_eq!(zeroed.len(), 128, "{}", v.name);
            // [PPC-Book2 p:20 s:3.2.2] The block is the one that holds
            // (RA|0) + (RB).
            let w = v.words[0];
            let (ra, rb) = (((w >> 16) & 31) as u8, ((w >> 11) & 31) as u8);
            let base = if ra == 0 { 0 } else { v.initial_state.gpr[&ra] };
            let ea = base.wrapping_add(v.initial_state.gpr[&rb]);
            assert_eq!(zeroed[0] as u64, (ea - BASE) & !127, "{}", v.name);
            assert!(zeroed.iter().all(|&i| after[i] == 0), "{}", v.name);
            checked += 1;
        }
    }
    assert_eq!(checked, 12);
}

/// A probe's committed frame payload, `len` bytes after the header.
fn payload(probe: &str, len: usize) -> Vec<u8> {
    use cellgov_testkit::console_cases::{probe_dir, PROFILE};
    let path = probe_dir(probe)
        .join("ps3")
        .join(PROFILE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "{probe}: frame magic");
    assert_eq!(frame.len() - 8, len, "{probe}: payload size");
    frame[8..].to_vec()
}

fn start_gprs(v: &PpuReferenceVector) -> Vec<u64> {
    (0..32u8).map(|n| v.initial_state.gpr[&n]).collect()
}

#[test]
fn each_console_vector_agrees_with_what_its_capture_recorded() {
    let sets = all_sets();
    let set = |file: &str| {
        &sets
            .iter()
            .find(|(f, _)| f == file)
            .unwrap_or_else(|| panic!("no set {file}"))
            .1
    };
    // ppu_cache_block: no failed step, both lines read back as filled
    // (PATTERN ^ word index), and the dcbt and dcbtst markers set.
    let p = payload("ppu_cache_block", 4 * 18);
    let word = |n: usize| u32::from_be_bytes(p[4 * n..4 * n + 4].try_into().expect("4"));
    assert_eq!(word(0), 0, "ppu_cache_block failed a step");
    for (slot, index) in [0u32, 1, 30, 31, 32, 33, 62, 63].into_iter().enumerate() {
        assert_eq!(word(2 + slot), 0x5a5a_c3c3 ^ index, "line word {index}");
    }
    assert_eq!((word(10), word(11)), (1, 2), "the touch markers");
    for file in [
        "dcbf.json",
        "dcbst.json",
        "icbi.json",
        "dcbt.json",
        "dcbtst.json",
    ] {
        let s = set(file);
        let name = format!("{}-console", file.trim_end_matches(".json"));
        let v = vector(s, &name);
        assert_eq!(value_of(&v.expected.memory), v.initial_memory, "{name}");
        assert_eq!(value_of(&v.expected.state.gpr), start_gprs(v), "{name}");
    }
    // ppu_granule_nan: the first zeroed byte, the count, and none outside
    // the block, against the dcbz console vectors over the same buffer.
    let p = payload("ppu_granule_nan", 4 * 45);
    let word = |n: usize| u32::from_be_bytes(p[4 * n..4 * n + 4].try_into().expect("4"));
    assert_eq!(word(0), 0, "ppu_granule_nan failed a step");
    assert_eq!(word(34), 0, "dcbz zeroed bytes outside its block");
    for (file, name) in [
        ("dcbz.json", "dcbz-ea168-console"),
        ("facility_dcbz_width.json", "dcbz-width-ea168-console"),
    ] {
        let v = vector(set(file), name);
        let after = value_of(&v.expected.memory);
        let zeroed: Vec<usize> = (0..after.len())
            .filter(|&i| after[i] == 0 && v.initial_memory[i] != 0)
            .collect();
        assert_eq!(zeroed.first().map(|&i| i as u32), Some(word(32)), "{name}");
        assert_eq!(zeroed.len() as u32, word(33), "{name}");
    }
    // ppu_may_trap_sync_l3: the case ran, and both runs recorded r3 and CR
    // as the vector expects them.
    use cellgov_testkit::console_cases::{capture, Capture};
    let Capture::Done(runs) = capture("ppu_may_trap_sync_l3", 1) else {
        panic!("ppu_may_trap_sync_l3 ended inside its case");
    };
    let v = vector(set("sync.json"), "sync-l3-console");
    for run in runs[0] {
        assert_eq!(
            run.slots[0],
            value_of(&v.expected.state.gpr)[3],
            "sync 3 r3"
        );
        assert_eq!(run.cr, value_of(&v.expected.state.cr), "sync 3 CR");
    }
    // ppu_timebase_reads: every word written, no inconsistent triple, and
    // no read below an earlier one on either thread; the vectors leave the
    // read value out for that reason.
    let p = payload("ppu_timebase_reads", 8 * 90);
    let dword = |n: usize| u64::from_be_bytes(p[8 * n..8 * n + 8].try_into().expect("8"));
    assert_eq!(dword(89), 0x5050_5442, "ppu_timebase_reads did not finish");
    assert_eq!(
        (dword(73), dword(80), dword(83)),
        (0, 0, 0),
        "a read went back"
    );
    for name in ["read-tb", "read-tbu"] {
        let v = vector(set("facility_time_base.json"), name);
        assert!(
            matches!(v.expected.state.gpr, ReferenceField::Undefined { .. }),
            "{name}"
        );
    }
}

#[test]
fn each_refused_instruction_the_ppe_lists_has_a_vector() {
    let names = |file: &str| -> Vec<String> {
        committed(file)
            .vectors
            .into_iter()
            .map(|v| v.name)
            .collect()
    };
    let has =
        |names: &[String], m: &str| names.iter().any(|n| n == m || *n == format!("{m}-console"));
    let unimplemented = names("ppe_unimplemented.json");
    for m in PPE_UNIMPLEMENTED_INSTRUCTIONS {
        assert!(has(&unimplemented, m), "ppe_unimplemented: no {m}");
    }
    let privileged = names("privileged.json");
    for m in PPE_BOOK_III_INSTRUCTIONS {
        assert!(has(&privileged, m), "privileged: no {m}");
    }
}

#[test]
fn no_unassigned_vector_names_a_word_the_decoder_assigns() {
    let set = committed("unassigned_opcodes.json");
    // The zero word, six primaries, eight extended opcodes under each of
    // four primaries, and the console's primary 1.
    assert_eq!(set.vectors.len(), 1 + 6 + 4 * 8 + 1);
    for v in &set.vectors {
        let word = v.words[0];
        assert_eq!(word_unit(word), None, "{} {word:#010x}", v.name);
        assert_eq!(
            v.expected.stop.reason,
            ReferenceField::Value {
                value: PpuReferenceYieldReason::Fault
            },
            "{}",
            v.name
        );
    }
}

#[test]
fn a_one_bit_change_to_expected_data_or_cr_fails_each_system_file() {
    let mut broken = 0;
    for (file, _) in all_sets() {
        let mut set = committed(&file);
        let Some(index) = set.vectors.iter().position(|v| {
            matches!(v.expected.memory, ReferenceField::Value { .. })
                && matches!(v.expected.state.cr, ReferenceField::Value { .. })
        }) else {
            continue;
        };
        let vector = &mut set.vectors[index];
        assert!(
            replay_reference(&vector.to_artifact())
                .expect("replays")
                .is_match(),
            "{file}: the committed vector does not match"
        );
        if let ReferenceField::Value { value } = &mut vector.expected.memory {
            value[0] ^= 1;
        }
        let replay = replay_reference(&vector.to_artifact()).expect("replays");
        assert!(!replay.is_match(), "{file}: the changed data still matches");
        if let ReferenceField::Value { value } = &mut vector.expected.memory {
            value[0] ^= 1;
        }
        if let ReferenceField::Value { value } = &mut vector.expected.state.cr {
            *value ^= 1;
        }
        let replay = replay_reference(&vector.to_artifact()).expect("replays");
        assert!(!replay.is_match(), "{file}: the changed CR still matches");
        broken += 1;
    }
    assert_eq!(broken, 19);
}
