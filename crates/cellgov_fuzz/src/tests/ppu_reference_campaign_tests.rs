use std::path::{Path, PathBuf};

use cellgov_ps3_abi::hw::ppc_mnemonics::{
    PPE_BOOK_III_INSTRUCTIONS, PPE_UNIMPLEMENTED_INSTRUCTIONS,
};

use super::*;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ppu_reference")
}

fn addi_set() -> PpuReferenceSet {
    let json = std::fs::read_to_string(fixtures().join("addi.json")).expect("addi.json reads");
    match parse_reference_file(&json).expect("addi.json parses") {
        PpuReferenceFile::Set(set) => set,
        PpuReferenceFile::Single(_) => panic!("addi.json is a vector set"),
    }
}

fn run(file: &str, unit: Option<&str>) -> PpuReferenceFileRun {
    PpuReferenceFileRun {
        file: file.into(),
        unit: unit.map(Into::into),
        outcome: PpuReferenceFileOutcome::Set(Vec::new()),
    }
}

#[test]
fn the_reference_directory_replays_clean_and_names_every_unit() {
    let campaign = run_reference_directory(&fixtures()).expect("the directory reads");
    let failing: Vec<&str> = campaign
        .files
        .iter()
        .filter(|run| !run.outcome.is_match())
        .map(|run| run.file.as_str())
        .collect();
    assert!(failing.is_empty(), "failing files: {failing:?}");
    let c = &campaign.completeness;
    assert!(c.is_complete(), "{c:#?}");
    assert_eq!(c.covered + c.pending, c.units);
    assert!(
        campaign.vectors() > campaign.files.len(),
        "{} vectors in {} files",
        campaign.vectors(),
        campaign.files.len()
    );
}

#[test]
fn every_decoded_kind_belongs_to_a_documented_unit() {
    assert_eq!(uncovered_kinds(), Vec::new());
}

#[test]
fn the_unit_list_names_each_unit_once() {
    let units = all_units();
    let keys: std::collections::BTreeSet<String> = units.iter().map(unit_key).collect();
    assert_eq!(keys.len(), units.len());
    for mnemonic in PPE_UNIMPLEMENTED_INSTRUCTIONS
        .iter()
        .chain(PPE_BOOK_III_INSTRUCTIONS)
    {
        assert!(
            !keys.contains(&format!("instruction:{mnemonic}")),
            "{mnemonic} is in a group unit"
        );
    }
}

#[test]
fn a_one_bit_change_to_each_vectors_expected_pc_or_lr_fails_its_replay() {
    let campaign = run_reference_directory(&fixtures()).expect("the directory reads");
    let mut broken = 0;
    for run in &campaign.files {
        let json = std::fs::read_to_string(fixtures().join(&run.file)).expect("reads");
        let PpuReferenceFile::Set(set) = parse_reference_file(&json).expect("parses") else {
            panic!("{}: the directory holds vector sets", run.file);
        };
        for index in 0..set.vectors.len() {
            let mut mutated = set.clone();
            let state = &mut mutated.vectors[index].expected.state;
            // A vector that stops at a system call leaves its PC open;
            // its LR is a value.
            match (&mut state.pc, &mut state.lr) {
                (ReferenceField::Value { value }, _) => *value ^= 4,
                (_, ReferenceField::Value { value }) => *value ^= 4,
                _ => panic!(
                    "{} {}: neither the expected PC nor LR is a value",
                    run.file, set.vectors[index].name
                ),
            }
            let replay = replay_reference(&mutated.vectors[index].to_artifact()).expect("replays");
            assert!(
                !replay.is_match(),
                "{} {}: the changed PC or LR still matches",
                run.file,
                set.vectors[index].name
            );
            broken += 1;
        }
    }
    assert!(broken > 0, "the directory holds no vector");
    assert_eq!(broken, campaign.vectors());
}

#[test]
fn a_reason_that_names_a_tracker_issue_is_refused() {
    let mut set = addi_set();
    set.vectors[0].expected.state.tb = ReferenceField::Undefined {
        reason: "settled by #123".into(),
    };
    assert!(matches!(
        set.validate(),
        Err(PpuReferenceError::TrackerReason { reason }) if reason == "settled by #123"
    ));
}

#[test]
fn a_set_with_a_repeated_vector_name_or_no_vector_is_refused() {
    let mut repeated = addi_set();
    repeated.vectors.push(repeated.vectors[0].clone());
    assert!(matches!(
        repeated.validate(),
        Err(PpuReferenceError::VectorName { name }) if name == "li-r3-7"
    ));
    let mut empty = addi_set();
    empty.vectors.clear();
    assert!(matches!(
        empty.validate(),
        Err(PpuReferenceError::VectorCount { found: 0, .. })
    ));
}

#[test]
fn completeness_reports_each_kind_of_gap() {
    let files = [
        run("a.json", Some("instruction:add")),
        run("b.json", Some("instruction:add")),
        run("c.json", None),
        run("d.json", Some("instruction:subf")),
    ];
    let report = completeness(
        &files,
        "instruction:subf\nnot_a_unit\ninstruction:or\ninstruction:or\n",
    );
    assert_eq!(
        report.duplicated.get("instruction:add"),
        Some(&vec!["a.json".to_string(), "b.json".to_string()])
    );
    assert_eq!(report.unowned, vec!["c.json".to_string()]);
    assert_eq!(report.stale_pending, vec!["instruction:subf".to_string()]);
    assert_eq!(
        report.unknown_pending,
        vec!["not_a_unit".to_string(), "instruction:or".to_string()]
    );
    assert!(report.missing.contains(&"instruction:and".to_string()));
    assert!(!report.missing.contains(&"instruction:or".to_string()));
    assert_eq!(report.covered, 2);
    assert_eq!(report.pending, 1);
    assert_eq!(
        report.covered + report.pending + report.missing.len(),
        report.units
    );
    assert!(!report.is_complete());
}

#[test]
fn a_facility_set_need_not_hold_a_word_of_its_unit() {
    let mut set = addi_set();
    set.unit = PpuReferenceUnit::Facility {
        name: PpuReferenceFacility::TimeBase,
    };
    assert!(matches!(set.validate(), Ok(())));
}

#[test]
fn a_set_whose_vector_holds_no_word_of_its_unit_is_refused() {
    let mut set = addi_set();
    set.unit = PpuReferenceUnit::Instruction {
        mnemonic: "subf".into(),
    };
    assert!(matches!(
        set.validate(),
        Err(PpuReferenceError::VectorUnit { name, unit })
            if name == "li-r3-7" && unit == "instruction:subf"
    ));
}

#[test]
fn a_single_vector_file_whose_reason_names_a_tracker_issue_is_refused() {
    let json = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ppu_reference_single/li_r3_7_v2.json"
    ));
    let mut artifact = parse_reference_json(json).expect("parses");
    artifact.expected.state.tb = ReferenceField::Undefined {
        reason: "settled by #7".into(),
    };
    let json = serde_json::to_string(&artifact).expect("serializes");
    assert!(matches!(
        parse_reference_file(&json),
        Err(PpuReferenceError::TrackerReason { reason }) if reason == "settled by #7"
    ));
}

#[test]
fn word_bit_6_selects_the_stream_instructions_of_their_own() {
    // [AltiVec-PEM p:A-1 s:A.1] dss/dssall: XO 822; dst/dstt: XO 342;
    // dstst/dststt: XO 374; bit 6 is the A or T field.
    let word = |xo: u32, bit_6: u32, ra: u32, rb: u32| {
        (31 << 26) | (bit_6 << 25) | (ra << 16) | (rb << 11) | (xo << 1)
    };
    let cases = [
        (word(822, 0, 0, 0), "dss"),
        (word(822, 1, 0, 0), "dssall"),
        (word(342, 0, 3, 4), "dst"),
        (word(342, 1, 3, 4), "dstt"),
        (word(374, 0, 3, 4), "dstst"),
        (word(374, 1, 3, 4), "dststt"),
    ];
    for (raw, mnemonic) in cases {
        assert_eq!(
            word_unit(raw),
            Some(PpuReferenceUnit::Instruction {
                mnemonic: mnemonic.into()
            }),
            "{raw:#010x}"
        );
    }
}

#[test]
fn each_word_the_ppe_refuses_sits_in_its_documented_group() {
    use cellgov_ppu::instruction::{ppe_illegal_mnemonic, ppe_privileged_mnemonic};
    // The PPE refuses an SPR move, lq or stq by its operands. tlbiel is
    // outside the listing.
    let by_operand = ["mfspr", "mtspr", "lq", "stq", "tlbiel"];
    let mut words = Vec::new();
    for primary in [19u32, 31] {
        words.extend((0..1024u32).map(|xo| (primary << 26) | (xo << 1)));
    }
    for primary in [59u32, 63] {
        words.extend((0..32u32).map(|xo| (primary << 26) | (xo << 1)));
    }
    let mut checked = 0;
    for raw in words {
        if let Some(mnemonic) = ppe_illegal_mnemonic(raw).filter(|m| !by_operand.contains(m)) {
            assert!(
                PPE_UNIMPLEMENTED_INSTRUCTIONS.contains(&mnemonic),
                "{mnemonic} is refused as illegal"
            );
            assert_eq!(
                word_unit(raw),
                Some(PpuReferenceUnit::PpeUnimplemented),
                "{mnemonic}"
            );
            checked += 1;
        }
        if let Some(mnemonic) = ppe_privileged_mnemonic(raw).filter(|m| !by_operand.contains(m)) {
            assert!(
                PPE_BOOK_III_INSTRUCTIONS.contains(&mnemonic),
                "{mnemonic} is refused as privileged"
            );
            assert_eq!(
                word_unit(raw),
                Some(PpuReferenceUnit::Privileged),
                "{mnemonic}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 9 + 12);
}

#[test]
fn a_single_vector_file_of_one_word_covers_its_kinds_unit() {
    let json = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ppu_reference_single/mtfsb1_ox_v2.json"
    ));
    let file = parse_reference_file(json).expect("parses");
    assert_eq!(
        file_unit(&file),
        Some(PpuReferenceUnit::Instruction {
            mnemonic: "mtfsb1".into()
        })
    );
}
