use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::capture::*;
use super::floor::{features, floor, required};
use crate::ppu_reference::{
    parse_reference_file, replay_reference, PpuReferenceFile, PpuReferenceSet, ReferenceField,
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
fn each_converted_file_is_its_captures_converted() {
    let conversion = convert(&captured_probes());
    let mut stale = Vec::new();
    for unit in UNITS {
        let candidates = conversion.candidates.get(unit).cloned().unwrap_or_default();
        assert!(!candidates.is_empty(), "{unit}: no captured case");
        let want = render(&unit_set(unit, &candidates));
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

#[test]
fn the_converter_skips_only_other_units_and_the_traps_that_ended_the_console_run() {
    let conversion = convert(&captured_probes());
    let skipped: Vec<_> = conversion.skipped.keys().copied().collect();
    assert_eq!(skipped, [Skip::OtherUnit, Skip::Ended]);
    assert_eq!(
        conversion.skipped[&Skip::Ended],
        2,
        "the tdi and twi cases that trapped on the console"
    );
}

#[test]
fn each_converted_set_meets_the_floor_of_its_unit() {
    let mut short = Vec::new();
    for unit in UNITS {
        let f = floor(unit);
        let set = committed_set(unit);
        let shown: BTreeSet<String> = set.vectors.iter().flat_map(|v| features(&f, v)).collect();
        let missing: Vec<String> = required(&f).difference(&shown).cloned().collect();
        if !missing.is_empty() {
            short.push(format!("{unit}: {missing:?}"));
        }
    }
    assert!(short.is_empty(), "{}", short.join("\n"));
}

#[test]
fn a_one_bit_change_to_an_expected_gpr_fails_each_converted_file() {
    for unit in UNITS {
        let mut set = committed_set(unit);
        let clean = replay_reference(&set.vectors[0].to_artifact()).expect("replays");
        assert!(
            clean.is_match(),
            "{unit}: the unchanged vector does not match"
        );
        let ReferenceField::Value { value } = &mut set.vectors[0].expected.state.gpr else {
            panic!("{unit}: the expected GPRs are a value");
        };
        value[3] ^= 1;
        let replay = replay_reference(&set.vectors[0].to_artifact()).expect("replays");
        assert!(!replay.is_match(), "{unit}: the changed GPR still matches");
    }
}

#[test]
fn no_converted_unit_is_pending() {
    let pending =
        std::fs::read_to_string(fixtures().join("pending.txt")).expect("pending.txt reads");
    let listed: BTreeSet<&str> = pending.lines().map(str::trim).collect();
    for unit in UNITS {
        assert!(
            !listed.contains(format!("instruction:{unit}").as_str()),
            "{unit} is still pending"
        );
    }
}
