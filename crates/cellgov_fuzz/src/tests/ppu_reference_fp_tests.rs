use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::capture::{captured_probes, convert, file_name, render};
use super::fp::*;
use crate::ppu_reference::{
    parse_reference_file, replay_reference, PpuReferenceFile, PpuReferenceSet, PpuReferenceState,
    PpuReferenceVector, ReferenceField,
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
fn each_floating_point_file_is_its_captures_converted() {
    let conversion = convert(&captured_probes());
    let mut stale = Vec::new();
    for unit in FP_UNITS {
        let set = fp_set(unit, &conversion);
        assert!(!set.vectors.is_empty(), "{unit}: no captured case");
        let want = render(&set);
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
fn each_floating_point_set_meets_the_floor_of_its_unit() {
    let mut short = Vec::new();
    for unit in FP_UNITS {
        let floor = fp_floor(unit);
        let shown: BTreeSet<String> = committed_set(unit)
            .vectors
            .iter()
            .flat_map(|v| fp_features(&floor, unit, v))
            .collect();
        let missing: Vec<String> = fp_required(&floor).difference(&shown).cloned().collect();
        if !missing.is_empty() {
            short.push(format!("{unit}: {missing:?}"));
        }
    }
    assert!(short.is_empty(), "{}", short.join("\n"));
}

#[test]
fn a_status_bit_another_word_sets_is_not_credited_to_the_unit() {
    let conversion = convert(&captured_probes());
    let name = "ppu_mcrfs_ox-overflow-vxsnan-then-mcrfs0";
    let vector = conversion.candidates["fadd"]
        .iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("{name} is a fadd candidate"));
    assert!(vector.words.len() > 1, "{name} has several words");
    let shown = fp_features(&fp_floor("fadd"), "fadd", vector);
    assert!(
        !shown.contains("sets:vxsnan"),
        "{name} credits fadd with the VXSNAN that mtfsb1 set: {shown:?}"
    );
}

#[test]
fn a_one_bit_change_to_an_expected_fpr_cr_or_fpscr_fails_each_floating_point_file() {
    for unit in FP_UNITS {
        let set = committed_set(unit);
        let vector = &set.vectors[0];
        assert!(
            replay_reference(&vector.to_artifact())
                .expect("replays")
                .is_match(),
            "{unit}: the unchanged vector does not match"
        );
        let frt = ((vector.words[0] >> 21) & 31) as usize;
        fails(vector, unit, "FRT", |s| {
            let ReferenceField::Value { value } = &mut s.fpr else {
                panic!("the expected FPRs are a value");
            };
            value[frt] ^= 1;
        });
        fails(vector, unit, "CR", |s| {
            let ReferenceField::Value { value } = &mut s.cr else {
                panic!("the expected CR is a value");
            };
            *value ^= 1;
        });
        fails(vector, unit, "FPSCR", |s| {
            let ReferenceField::Value { value } = &mut s.fpscr else {
                panic!("the expected FPSCR is a value");
            };
            *value ^= 0x0000_0008;
        });
    }
}

/// Asserts that `vector` with `change` made to its expected state no
/// longer matches.
fn fails(
    vector: &PpuReferenceVector,
    unit: &str,
    name: &str,
    change: impl FnOnce(&mut PpuReferenceState),
) {
    let mut changed = vector.clone();
    change(&mut changed.expected.state);
    let replay = replay_reference(&changed.to_artifact()).expect("replays");
    assert!(
        !replay.is_match(),
        "{unit}: the changed {name} still matches"
    );
}

#[test]
fn no_floating_point_unit_is_pending() {
    let pending =
        std::fs::read_to_string(fixtures().join("pending.txt")).expect("pending.txt reads");
    let listed: BTreeSet<&str> = pending.lines().map(str::trim).collect();
    for unit in FP_UNITS {
        assert!(
            !listed.contains(format!("instruction:{unit}").as_str()),
            "{unit} is still pending"
        );
    }
}
