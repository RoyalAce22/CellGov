use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::capture::{file_name, render};
use super::multiple::*;
use crate::ppu_reference::{
    parse_reference_file, replay_reference, PpuReferenceFile, PpuReferenceSet, PpuReferenceVector,
    PpuReferenceYieldReason, ReferenceField,
};
use crate::regenerate::write_if_asked;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ppu_reference")
}

fn committed_set(unit: &str) -> PpuReferenceSet {
    let path = fixtures().join(file_name(unit));
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    match parse_reference_file(&json).unwrap_or_else(|e| panic!("{}: {e}", path.display())) {
        PpuReferenceFile::Set(set) => set,
        PpuReferenceFile::Single(_) => panic!("{}: a single-vector file", path.display()),
    }
}

#[test]
fn each_multiple_and_string_file_is_its_model_rendered() {
    let mut stale = Vec::new();
    for unit in MULTIPLE_UNITS {
        let want = render(&multiple_set(unit));
        let path = fixtures().join(file_name(unit));
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

fn expected_gprs(v: &PpuReferenceVector) -> Vec<u64> {
    match &v.expected.state.gpr {
        ReferenceField::Value { value } => value.clone(),
        other => panic!("{}: GPRs {other:?}", v.name),
    }
}

fn vector<'a>(set: &'a PpuReferenceSet, name: &str) -> &'a PpuReferenceVector {
    set.vectors
        .iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("no vector {name}"))
}

#[test]
fn the_model_follows_the_cited_rules_on_fixed_vectors() {
    // [PPC-Book1 p:48 s:3.3.6] Five bytes fill r5 and the top byte of
    // r6's low half; the rest of r6 is cleared.
    let lswi = multiple_set("lswi");
    let v = vector(&lswi, "lswi-r5-n5-off1");
    let m = &v.initial_memory;
    let g = expected_gprs(v);
    assert_eq!(
        g[5],
        u64::from(u32::from_be_bytes([m[1], m[2], m[3], m[4]]))
    );
    assert_eq!(g[6], u64::from(m[5]) << 24);
    // Twelve bytes from r30 wrap into r0.
    let v = vector(&lswi, "lswi-r30-n12-off2");
    let m = &v.initial_memory;
    assert_eq!(
        expected_gprs(v)[0],
        u64::from(u32::from_be_bytes([m[10], m[11], m[12], m[13]]))
    );
    // [PPC-Book1 p:46 s:3.3.5] lmw clears each high half.
    let lmw = multiple_set("lmw");
    let v = vector(&lmw, "lmw-r31-base0-d0");
    let m = &v.initial_memory;
    assert_eq!(
        expected_gprs(v)[31],
        u64::from(u32::from_be_bytes([m[0], m[1], m[2], m[3]]))
    );
    // stmw stores only the low halves.
    let stmw = multiple_set("stmw");
    let v = vector(&stmw, "stmw-r31-base0-d0");
    let ReferenceField::Value { value: data } = &v.expected.memory else {
        panic!("stmw data is a value");
    };
    let r31 = v.initial_state.gpr[&31];
    assert_eq!(data[0..4], (r31 as u32).to_be_bytes());
    assert_eq!(data[4..], v.initial_memory[4..]);
    // [PPC-Book1 p:49 s:3.3.6] Five stored bytes take r5's low half and
    // the top byte of r6's low half; the bytes past them keep their
    // start values.
    for (unit, name) in [("stswi", "stswi-r5-n5-off1"), ("stswx", "stswx-r5-n5-off1")] {
        let set = multiple_set(unit);
        let v = vector(&set, name);
        let ReferenceField::Value { value: data } = &v.expected.memory else {
            panic!("{name} data is a value");
        };
        let g = &v.initial_state.gpr;
        assert_eq!(data[1..5], (g[&5] as u32).to_be_bytes(), "{name}");
        assert_eq!(data[5], (g[&6] >> 24) as u8, "{name}");
        assert_eq!(data[..1], v.initial_memory[..1], "{name}");
        assert_eq!(data[6..], v.initial_memory[6..], "{name}");
    }
    // Twelve bytes from r30 wrap into r0.
    let stswi = multiple_set("stswi");
    let v = vector(&stswi, "stswi-r30-n12-off2");
    let ReferenceField::Value { value: data } = &v.expected.memory else {
        panic!("stswi data is a value");
    };
    let g = &v.initial_state.gpr;
    assert_eq!(data[2..6], (g[&30] as u32).to_be_bytes());
    assert_eq!(data[6..10], (g[&31] as u32).to_be_bytes());
    assert_eq!(data[10..14], (g[&0] as u32).to_be_bytes());
    assert_eq!(data[14..], v.initial_memory[14..]);
}

/// What a multiple or string vector shows:
///
/// - whether its console run ended;
/// - its displacement's sign and whether RT is r31, for `lmw` and `stmw`;
/// - its byte count, for a string form;
/// - whether its register range wraps past r31;
/// - whether it is the RA = 0 form.
fn shown(unit: &str, v: &PpuReferenceVector) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if matches!(
        v.expected.stop.reason,
        ReferenceField::Value {
            value: PpuReferenceYieldReason::Fault
        }
    ) {
        out.insert("ended".to_owned());
        return out;
    }
    let word = *v.words.last().expect("a vector has words");
    let rt = ((word >> 21) & 31) as u64;
    let ra = (word >> 16) & 31;
    let rb = ((word >> 11) & 31) as u64;
    let n = match unit {
        "lmw" | "stmw" => {
            let d = word as u16 as i16;
            out.insert(format!("d:{}", d.signum()));
            out.insert(format!("rt:{}", if rt == 31 { "31" } else { "range" }));
            return out;
        }
        "lswi" | "stswi" => {
            if rb == 0 {
                32
            } else {
                rb
            }
        }
        _ => v.initial_state.xer & 0x7f,
    };
    out.insert(format!("n:{n}"));
    if n > 0 && rt + n.div_ceil(4) - 1 > 31 {
        out.insert("wrap".to_owned());
    }
    if ra == 0 {
        out.insert("ra0".to_owned());
    }
    out
}

fn floor(unit: &str) -> BTreeSet<String> {
    let names: &[&str] = match unit {
        "lmw" | "stmw" => &["d:0", "d:1", "d:-1", "rt:31", "rt:range", "ended"],
        "lswi" | "stswi" => &["n:1", "n:4", "n:5", "n:32", "wrap"],
        "lswx" => &["n:0", "n:1", "n:4", "n:5", "n:32", "wrap", "ra0", "ended"],
        "stswx" => &["n:0", "n:1", "n:4", "n:5", "n:32", "wrap", "ra0"],
        other => panic!("no floor for {other}"),
    };
    let mut out: BTreeSet<String> = names.iter().map(|n| (*n).to_owned()).collect();
    if unit == "lswi" {
        out.insert("ended".to_owned());
    }
    out
}

#[test]
fn each_multiple_and_string_set_meets_the_floor_of_its_unit() {
    for unit in MULTIPLE_UNITS {
        let have: BTreeSet<String> = committed_set(unit)
            .vectors
            .iter()
            .flat_map(|v| shown(unit, v))
            .collect();
        let missing: Vec<_> = floor(unit).difference(&have).cloned().collect();
        assert!(missing.is_empty(), "{unit}: {missing:?}");
    }
}

#[test]
fn a_one_bit_change_to_an_expected_value_fails_each_multiple_and_string_file() {
    for unit in MULTIPLE_UNITS {
        let mut set = committed_set(unit);
        let vector = &mut set.vectors[0];
        assert!(
            replay_reference(&vector.to_artifact())
                .expect("replays")
                .is_match(),
            "{unit}: the committed vector does not match"
        );
        let store = unit.starts_with("st");
        if store {
            let ReferenceField::Value { value } = &mut vector.expected.memory else {
                panic!("{unit}: the expected data is a value");
            };
            value[0] ^= 1;
        } else {
            let ReferenceField::Value { value } = &mut vector.expected.state.gpr else {
                panic!("{unit}: the expected GPRs are a value");
            };
            let rt = ((vector.words[0] >> 21) & 31) as usize;
            value[rt] ^= 1;
        }
        let replay = replay_reference(&vector.to_artifact()).expect("replays");
        assert!(
            !replay.is_match(),
            "{unit}: the changed value still matches"
        );
    }
}
