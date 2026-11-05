use std::collections::BTreeSet;

use super::capture::{captured_probes, convert, file_name, render};
use super::vmx_int::*;
use super::vmx_tests::{committed, fails, fixtures};
use crate::ppu_reference::{replay_reference, ReferenceField};
use crate::regenerate::write_if_asked;

#[test]
fn each_vector_integer_file_is_its_captures_converted() {
    let conversion = convert(&captured_probes());
    let mut stale = Vec::new();
    for unit in VMX_INT_UNITS {
        let set = vmx_int_set(unit, &conversion);
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
fn each_vector_integer_set_meets_the_floor_of_its_unit() {
    let short: Vec<String> = VMX_INT_UNITS
        .into_iter()
        .filter_map(|unit| {
            let missing = int_missing(unit, &committed(&file_name(unit)));
            (!missing.is_empty()).then(|| format!("{unit}: {missing:?}"))
        })
        .collect();
    assert!(short.is_empty(), "{}", short.join("\n"));
}

#[test]
fn a_one_bit_change_to_an_expected_vr_or_vscr_fails_each_vector_integer_file() {
    for unit in VMX_INT_UNITS {
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
fn no_vector_integer_unit_is_pending() {
    let pending =
        std::fs::read_to_string(fixtures().join("pending.txt")).expect("pending.txt reads");
    let listed: BTreeSet<&str> = pending.lines().map(str::trim).collect();
    for unit in VMX_INT_UNITS {
        assert!(
            !listed.contains(format!("instruction:{unit}").as_str()),
            "{unit} is still pending"
        );
    }
}
