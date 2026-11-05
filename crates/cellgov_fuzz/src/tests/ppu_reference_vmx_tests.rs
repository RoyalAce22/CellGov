use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::capture::{captured_probes, convert, file_name, render};
use super::vmx::*;
use crate::ppu_reference::{
    parse_reference_file, replay_reference, PpuReferenceFile, PpuReferenceSet, PpuReferenceState,
    PpuReferenceVector, ReferenceField,
};
use crate::regenerate::write_if_asked;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ppu_reference")
}

fn units() -> impl Iterator<Item = &'static str> {
    VMX_UNITS.into_iter().chain(VMX_MEMORY_UNITS)
}

fn committed(name: &str) -> PpuReferenceSet {
    let path = fixtures().join(name);
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    match parse_reference_file(&json).unwrap_or_else(|e| panic!("{}: {e}", path.display())) {
        PpuReferenceFile::Set(set) => set,
        PpuReferenceFile::Single(_) => panic!("{}: a single-vector file", path.display()),
    }
}

#[test]
fn each_vector_file_is_its_captures_converted() {
    let conversion = convert(&captured_probes());
    let mut stale = Vec::new();
    let sets = units()
        .map(|unit| (file_name(unit), vmx_set(unit, &conversion)))
        .chain([
            ("facility_vscr.json".to_owned(), vscr_facility_set()),
            ("facility_vrsave.json".to_owned(), vrsave_facility_set()),
        ]);
    for (name, set) in sets {
        assert!(!set.vectors.is_empty(), "{name}: no captured case");
        let want = render(&set);
        let path = fixtures().join(&name);
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
fn each_vector_set_meets_the_floor_of_its_unit() {
    let mut short = Vec::new();
    for unit in units() {
        let floor = vmx_floor(unit);
        let shown: BTreeSet<String> = committed(&file_name(unit))
            .vectors
            .iter()
            .flat_map(|v| vmx_features(floor, unit, v))
            .collect();
        let missing: Vec<String> = vmx_required(floor).difference(&shown).cloned().collect();
        if !missing.is_empty() {
            short.push(format!("{unit}: {missing:?}"));
        }
    }
    assert!(short.is_empty(), "{}", short.join("\n"));
}

#[test]
fn a_one_bit_change_to_an_expected_vr_or_vscr_fails_each_vector_file() {
    let names = units()
        .map(file_name)
        .chain(["facility_vscr.json".to_owned()]);
    for name in names {
        let set = committed(&name);
        let vector = &set.vectors[0];
        assert!(
            replay_reference(&vector.to_artifact())
                .expect("replays")
                .is_match(),
            "{name}: the unchanged vector does not match"
        );
        fails(vector, &name, "VR", |s| {
            let ReferenceField::Value { value } = &mut s.vr_hex else {
                panic!("the expected VRs are a value");
            };
            let v = &mut value[3];
            let last = v.pop().expect("a hex digit");
            v.push(if last == '0' { '1' } else { '0' });
        });
        fails(vector, &name, "VSCR", |s| {
            let ReferenceField::Value { value } = &mut s.vscr else {
                panic!("the expected VSCR is a value");
            };
            *value ^= 1;
        });
    }
}

#[test]
fn a_changed_expected_vrsave_or_gpr_fails_the_vrsave_facility_file() {
    let set = committed("facility_vrsave.json");
    assert_eq!(set.vectors.len(), 3, "facility_vrsave.json: vector count");
    for vector in &set.vectors {
        assert!(
            replay_reference(&vector.to_artifact())
                .expect("replays")
                .is_match(),
            "{}: the unchanged vector does not match",
            vector.name
        );
        fails(vector, &vector.name, "VRSAVE", |s| {
            let ReferenceField::Value { value } = &mut s.vrsave else {
                panic!("the expected VRSAVE is a value");
            };
            *value ^= 1;
        });
        fails(vector, &vector.name, "r3", |s| {
            let ReferenceField::Value { value } = &mut s.gpr else {
                panic!("the expected GPRs are a value");
            };
            value[3] ^= 1 << 40;
        });
    }
}

/// Asserts that `vector` with `change` made to its expected state no
/// longer matches.
fn fails(
    vector: &PpuReferenceVector,
    name: &str,
    what: &str,
    change: impl FnOnce(&mut PpuReferenceState),
) {
    let mut changed = vector.clone();
    change(&mut changed.expected.state);
    let replay = replay_reference(&changed.to_artifact()).expect("replays");
    assert!(
        !replay.is_match(),
        "{name}: the changed {what} still matches"
    );
}

#[test]
fn a_case_whose_end_rests_on_an_unloaded_vscr_or_whose_vscr_write_is_unread_is_skipped() {
    use super::capture::{skip_reason, Skip, ENTRY_VSCR, MEMORY_BASE};
    use cellgov_testkit::console_cases::{parse_table, Run};
    let run = Run {
        slots: [0; 4],
        xer: 0,
        cr: 0,
        fpscr: 0,
    };
    const SATURATES: &str = "v1=0x01000100_01000100_01000100_01000100";
    const FITS: &str = "v1=0x00010002_00030004_00050006_00070008";
    // mfvscr v4; vpkshss v3,v1,v2; mtvscr v5 then vpkshss then mfvscr v4.
    let rows = [
        ("0x10800604", FITS, "v4", Err(Skip::UnloadedVscr)),
        ("0x1061118e", SATURATES, "v3", Err(Skip::UnloadedVscr)),
        ("0x1061118e", FITS, "v3", Ok(())),
        (
            "0x10002e44,0x1061118e",
            SATURATES,
            "v3",
            Err(Skip::WritesUnrecorded),
        ),
        (
            "0x10002e44,0x1061118e,0x10800604",
            SATURATES,
            "v3,v4",
            Ok(()),
        ),
    ];
    for (words, input, record, want) in rows {
        let text = format!(
            "id\tasm\twords\tinputs\trecord\tmay_end_process\n\
             c\t-\t{words}\t{input},v5=0x10000\t{record}\t0\n"
        );
        let cases = parse_table(&text).expect("the row parses");
        let mut start = cases[0].initial(0, MEMORY_BASE);
        start.vscr = ENTRY_VSCR;
        let got = skip_reason(&cases[0], &start, &[run, run]);
        assert_eq!(got, want, "{words} with {input} recording {record}");
    }
}

#[test]
fn no_vector_unit_of_this_module_is_pending() {
    let pending =
        std::fs::read_to_string(fixtures().join("pending.txt")).expect("pending.txt reads");
    let listed: BTreeSet<&str> = pending.lines().map(str::trim).collect();
    for unit in units() {
        assert!(
            !listed.contains(format!("instruction:{unit}").as_str()),
            "{unit} is still pending"
        );
    }
}
