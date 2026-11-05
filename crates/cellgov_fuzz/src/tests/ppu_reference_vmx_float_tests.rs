use super::capture::{captured_probes, convert, file_name, render};
use super::vmx::{mnemonic, vscr_before};
use super::vmx_float::*;
use super::vmx_tests::{committed, fails, fixtures};
use crate::ppu_reference::{replay_reference, ReferenceField};
use crate::regenerate::write_if_asked;

#[test]
fn each_vector_float_file_is_its_captures_converted() {
    let conversion = convert(&captured_probes());
    let mut stale = Vec::new();
    for unit in VMX_FLOAT_UNITS {
        let set = vmx_float_set(unit, &conversion);
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
fn each_vector_float_set_meets_the_floor_of_its_unit() {
    let short: Vec<String> = VMX_FLOAT_UNITS
        .into_iter()
        .filter_map(|unit| {
            let missing = float_missing(unit, &committed(&file_name(unit)));
            (!missing.is_empty()).then(|| format!("{unit}: {missing:?}"))
        })
        .collect();
    assert!(short.is_empty(), "{}", short.join("\n"));
}

#[test]
fn a_one_bit_change_to_an_expected_vr_or_vscr_fails_each_vector_float_file() {
    for unit in VMX_FLOAT_UNITS {
        let name = file_name(unit);
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
fn the_vscr_an_mtvscr_loads_is_the_splat_an_earlier_vspltisw_wrote() {
    let set = committed(&file_name("vnmsubfp"));
    let v = set
        .vectors
        .iter()
        .find(|v| v.name == "ppu_vmx_gap_edges-vnmsubfp_keeps_sat")
        .expect("the vector that loads the VSCR from a vspltisw");
    let at = v
        .words
        .iter()
        .position(|&w| mnemonic(w).as_deref() == Some("vnmsubfp"))
        .expect("a vnmsubfp word");
    // vnmsubfp leaves the VSCR alone, so the console's read-back is the
    // VSCR the mtvscr loaded: SAT set, NJ clear.
    let ReferenceField::Value { value } = &v.expected.state.vscr else {
        panic!("the expected VSCR is a value");
    };
    assert_eq!(*value, 1);
    assert_eq!(vscr_before(v, at), *value);
}

#[test]
fn a_record_compare_that_leaves_the_seeded_cr6_shows_no_cr6_outcome() {
    let mut set = committed(&file_name("vcmpeqfp"));
    assert!(
        !float_missing("vcmpeqfp", &set).contains(&"cr6:mixed".to_owned()),
        "vcmpeqfp: the committed set shows no mixed outcome"
    );
    let mut changed = 0;
    for v in &mut set.vectors {
        let seed = v.initial_state.cr;
        let ReferenceField::Value { value } = &mut v.expected.state.cr else {
            panic!("the expected CR is a value");
        };
        // Only a record compare with a mixed outcome ends with CR6 zero.
        if (*value >> 4) & 0xf == 0 {
            *value |= seed & 0xf0;
            changed += 1;
        }
    }
    assert!(
        changed > 0,
        "vcmpeqfp: no record compare ends with CR6 zero"
    );
    assert!(float_missing("vcmpeqfp", &set).contains(&"cr6:mixed".to_owned()));
}

#[test]
fn the_pending_list_is_empty() {
    let pending =
        std::fs::read_to_string(fixtures().join("pending.txt")).expect("pending.txt reads");
    let listed: Vec<&str> = pending
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert!(listed.is_empty(), "still pending: {listed:?}");
}
