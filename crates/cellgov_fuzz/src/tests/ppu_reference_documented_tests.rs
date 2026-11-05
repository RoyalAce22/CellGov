use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::capture::{file_name, render};
use super::documented::*;
use crate::ppu_reference::{
    parse_reference_file, replay_reference, PpuReferenceFile, PpuReferenceSet, PpuReferenceVector,
    ReferenceField,
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
fn each_documented_file_is_its_model_rendered() {
    let mut stale = Vec::new();
    for unit in DOCUMENTED_UNITS {
        let want = render(&documented_set(unit));
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
fn the_branch_model_follows_the_cited_rules_on_fixed_words() {
    // b 0x100 from 4: relative; ba 0x100: absolute; bl sets LR.
    assert_eq!(branch_end(0x4800_0100, 4, 0, 0, 0).pc, 0x104);
    assert_eq!(branch_end(0x4800_0102, 4, 0, 0, 0).pc, 0x100);
    assert_eq!(branch_end(0x4800_0101, 4, 0, 0, 0).lr, 8);
    // bdnz 0x40 (BO 16) falls through when CTR 1 -> 0 and branches when
    // CTR 2 -> 1.
    let bdnz = 0x4200_0040;
    assert_eq!(
        branch_end(bdnz, 0, 0, 0, 1),
        End {
            pc: 4,
            lr: 0,
            ctr: 0
        }
    );
    assert_eq!(branch_end(bdnz, 0, 0, 0, 2).pc, 0x40);
    // beq 0x40 (BO 12, BI 2): taken on CR bit 2 set, CTR left alone.
    assert_eq!(branch_end(0x4182_0040, 0, 0x2000_0000, 0, 7).pc, 0x40);
    assert_eq!(branch_end(0x4182_0040, 0, 0, 0, 7).ctr, 7);
    // blr to LR 0x1003 goes to 0x1000; bctrl puts 4 in LR.
    assert_eq!(branch_end(0x4e80_0020, 0, 0, 0x1003, 0).pc, 0x1000);
    assert_eq!(
        branch_end(0x4e80_0421, 0, 0, 0x55, 0x2004),
        End {
            pc: 0x2004,
            lr: 4,
            ctr: 0x2004
        }
    );
}

/// What a branch vector shows: its BO class taken or not, its LK and its
/// AA.
fn branch_features(v: &PpuReferenceVector) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let word = *v.words.last().expect("a vector has words");
    let ReferenceField::Value { value: pc } = v.expected.state.pc else {
        return out;
    };
    let taken = pc != v.initial_state.pc + 4;
    let primary = word >> 26;
    if primary != 18 {
        let bo = (word >> 21) & 31;
        out.insert(format!("bo{bo}:{}", if taken { "taken" } else { "not" }));
    }
    out.insert(format!("lk{}", word & 1));
    if primary != 19 {
        out.insert(format!("aa{}", (word >> 1) & 1));
    }
    out
}

/// The features a branch unit's set must show.
fn branch_floor(unit: &str) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = ["lk0", "lk1"].map(String::from).into();
    // Branch always is taken every time. Each BO2 = 0 bcctr form has one
    // capture: BO 8 taken and BO 10 not taken.
    let outcomes: &[&str] = match unit {
        "b" => &[],
        "bcctr" => &[
            "bo20:taken",
            "bo12:taken",
            "bo12:not",
            "bo4:taken",
            "bo4:not",
            "bo8:taken",
            "bo10:not",
        ],
        _ => &[
            "bo20:taken",
            "bo12:taken",
            "bo12:not",
            "bo4:taken",
            "bo4:not",
            "bo16:taken",
            "bo16:not",
            "bo18:taken",
            "bo18:not",
            "bo8:taken",
            "bo8:not",
            "bo0:taken",
            "bo0:not",
            "bo10:taken",
            "bo10:not",
            "bo2:taken",
            "bo2:not",
        ],
    };
    out.extend(outcomes.iter().map(|o| (*o).to_owned()));
    if matches!(unit, "b" | "bc") {
        out.extend(["aa0", "aa1"].map(String::from));
    }
    out
}

#[test]
fn each_branch_set_meets_the_floor_of_its_unit() {
    for unit in ["b", "bc", "bclr", "bcctr"] {
        let shown: BTreeSet<String> = committed_set(unit)
            .vectors
            .iter()
            .flat_map(branch_features)
            .collect();
        let missing: Vec<_> = branch_floor(unit).difference(&shown).cloned().collect();
        assert!(missing.is_empty(), "{unit}: {missing:?}");
    }
}

#[test]
fn a_one_bit_change_to_an_expected_lr_fails_each_documented_file() {
    for unit in DOCUMENTED_UNITS {
        let mut set = committed_set(unit);
        let vector = &mut set.vectors[0];
        assert!(
            replay_reference(&vector.to_artifact())
                .expect("replays")
                .is_match(),
            "{unit}: the committed vector does not match"
        );
        let ReferenceField::Value { value } = &mut vector.expected.state.lr else {
            panic!("{unit}: the expected LR is a value");
        };
        *value ^= 1;
        let replay = replay_reference(&vector.to_artifact()).expect("replays");
        assert!(!replay.is_match(), "{unit}: the changed LR still matches");
    }
}

#[test]
fn the_sc_set_stops_at_the_call_with_its_lev() {
    let vectors = committed_set("sc").vectors;
    assert_eq!(vectors.len(), 2, "one vector per start CR");
    for vector in vectors {
        // [PPC-Book1 p:26 s:2.4.2] LEV is bits 20:26 of the sc word.
        let word = *vector.words.last().expect("a vector has words");
        let lev = ((word >> 5) & 0x7f) as u8;
        let stop = &vector.expected.stop;
        assert!(
            matches!(
                stop.reason,
                ReferenceField::Value {
                    value: crate::ppu_reference::PpuReferenceYieldReason::Syscall
                }
            ),
            "{}",
            vector.name
        );
        assert_eq!(
            stop.syscall_lev,
            ReferenceField::Value { value: Some(lev) },
            "{}",
            vector.name
        );
        assert_eq!(
            stop.pc,
            ReferenceField::Value {
                value: Some(vector.initial_state.pc)
            },
            "{}: the stop is at the sc word",
            vector.name
        );
    }
}
