use std::collections::BTreeSet;
use std::path::Path;

use super::*;

const ROTATION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/spu_reference/rotqbyi_12_v1.json"
));

/// A one-vector set for `unit` that stops at once and expects nothing.
fn set_for(unit: serde_json::Value) -> String {
    let unsupported = serde_json::json!({"status": "unsupported", "reason": "not under test"});
    let expected: serde_json::Map<String, serde_json::Value> = [
        "end",
        "regs_hex",
        "local_store",
        "pc",
        "lslr",
        "fpscr",
        "stop",
        "interrupts_enabled",
        "srr0",
        "signals",
        "channels",
        "reservation",
        "effects",
        "main_memory",
        "peer",
        "ppu_results",
        "mfc_exceptions",
    ]
    .into_iter()
    .map(|name| (name.to_string(), unsupported.clone()))
    .collect();
    serde_json::json!({
        "schema_version": 6,
        "unit": unit,
        "vectors": [{
            "name": "v",
            "provenance": {
                "kind": "documented_vector",
                "citation": "CBEA p:95 s:8.5.3",
                "vector_id": "v"
            },
            "words": [0],
            "initial_state": {"pc": 0},
            "step_limit": 4,
            "expected": expected
        }]
    })
    .to_string()
}

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).expect("writes");
}

/// Every unit key except `covered`, one per line.
fn pending_except(covered: &[&str]) -> String {
    all_units()
        .iter()
        .map(unit_key)
        .filter(|key| !covered.contains(&key.as_str()))
        .map(|key| key + "\n")
        .collect()
}

/// The units come from their sources, one key each: 199 opcode-map rows
/// and the unassigned words, 28 defined channels and the reserved
/// numbers, 33 SPU-queue commands and the rest, and 13 facilities.
#[test]
fn the_units_are_every_row_channel_command_and_facility_once() {
    let units = all_units();
    assert_eq!(units.len(), 200 + 29 + 34 + 13);
    let keys: BTreeSet<String> = units.iter().map(unit_key).collect();
    assert_eq!(keys.len(), units.len());
    for key in [
        "instruction:rotqbyi",
        "unassigned_opcodes",
        "channel:29",
        "reserved_channels",
        "mfc_command:0x0040",
        "outside_spu_queue",
        "facility:dma_local_store_window",
    ] {
        assert!(keys.contains(key), "{key}");
    }
    // Reserved channels and proxy-only commands have no unit of their own.
    assert!(!keys.contains("channel:5"));
    assert!(!keys.contains("mfc_command:0x0028"));
}

#[test]
fn facility_names_are_the_names_the_schema_writes() {
    for facility in SpuReferenceFacility::ALL {
        assert_eq!(
            serde_json::to_value(facility).expect("serializes"),
            serde_json::Value::from(facility.name())
        );
    }
}

#[test]
fn a_one_word_single_vector_file_covers_the_row_its_word_selects() {
    let file = parse_reference_file(ROTATION).expect("parses");
    assert_eq!(
        file_unit(&file).map(|unit| unit_key(&unit)).as_deref(),
        Some("instruction:rotqbyi")
    );
    let mut two = serde_json::from_str::<serde_json::Value>(ROTATION).expect("JSON");
    two["words"] = serde_json::json!([0, 0]);
    let file = parse_reference_file(&two.to_string()).expect("parses");
    assert_eq!(file_unit(&file), None);
}

#[test]
fn a_complete_directory_replays_every_file_and_reports_no_gap() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("spu_reference_complete");
    let dir: &Path = &scratch;
    write(dir, "rotqbyi.json", ROTATION);
    write(
        dir,
        "start_state.json",
        &set_for(serde_json::json!({"kind": "facility", "name": "start_state"})),
    );
    write(dir, "README.md", "not a fixture");
    write(
        dir,
        SPU_REFERENCE_PENDING_FILE,
        &pending_except(&["instruction:rotqbyi", "facility:start_state"]),
    );
    let campaign = run_reference_directory(dir).expect("reads");
    assert_eq!(campaign.files.len(), 2);
    assert!(campaign.is_clean(), "{:?}", campaign.completeness);
    let completeness = &campaign.completeness;
    assert_eq!(completeness.covered, 2);
    assert_eq!(completeness.pending, completeness.units - 2);
}

/// Each way a directory can disagree with the units is named.
#[test]
fn each_gap_in_a_directory_is_named() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("spu_reference_gaps");
    let dir: &Path = &scratch;
    // Two files for one unit.
    write(dir, "a.json", ROTATION);
    write(dir, "b.json", ROTATION);
    // A file for no unit.
    let mut two = serde_json::from_str::<serde_json::Value>(ROTATION).expect("JSON");
    two["words"] = serde_json::json!([0, 0]);
    write(dir, "c.json", &two.to_string());
    // A file that does not parse covers no unit and does not match.
    write(dir, "d.json", "{}");
    // Pending: a stale entry, an unknown one, a repeat, and every other
    // unit but one.
    let mut pending = pending_except(&["instruction:rotqbyi", "facility:events"]);
    pending.push_str("instruction:rotqbyi\nnot_a_unit\nfacility:interrupts\n");
    write(dir, SPU_REFERENCE_PENDING_FILE, &pending);
    let campaign = run_reference_directory(dir).expect("reads");
    let completeness = &campaign.completeness;
    assert_eq!(completeness.missing, ["facility:events"]);
    assert_eq!(
        completeness
            .duplicated
            .get("instruction:rotqbyi")
            .map(Vec::as_slice),
        Some(["a.json".to_string(), "b.json".to_string()].as_slice())
    );
    assert_eq!(completeness.unowned, ["c.json", "d.json"]);
    assert_eq!(completeness.stale_pending, ["instruction:rotqbyi"]);
    assert_eq!(
        completeness.unknown_pending,
        ["not_a_unit", "facility:interrupts"]
    );
    assert!(!completeness.is_complete());
    assert!(matches!(
        campaign.files[3].outcome,
        SpuReferenceFileOutcome::Refused(_)
    ));
    assert!(!campaign.is_clean());
}

#[test]
fn a_vector_that_disagrees_fails_the_campaign() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("spu_reference_mismatch");
    let dir: &Path = &scratch;
    let mut wrong = serde_json::from_str::<serde_json::Value>(ROTATION).expect("JSON");
    wrong["expected"]["pc"]["value"] = 8.into();
    write(dir, "rotqbyi.json", &wrong.to_string());
    write(
        dir,
        SPU_REFERENCE_PENDING_FILE,
        &pending_except(&["instruction:rotqbyi"]),
    );
    let campaign = run_reference_directory(dir).expect("reads");
    assert!(campaign.completeness.is_complete());
    assert!(!campaign.files[0].outcome.is_match());
    assert!(!campaign.is_clean());
}

#[test]
fn a_set_vector_may_cite_any_spu_document_by_printed_page() {
    for (citation, valid) in [
        ("SPU-ISA p:132 s:6", true),
        ("CBEA p:95 s:8.5.3", true),
        ("CBE-Handbook p:445 s:17.1", true),
        ("PPC-Book1 p:10 s:1", false),
        ("CBEA p:095 s:8", false),
        ("CBEA p:0 s:8", false),
        ("CBEA p:95 s: ", false),
        ("CBEAp:95 s:8", false),
    ] {
        assert_eq!(
            super::set_validate::valid_reference_citation(citation),
            valid,
            "{citation}"
        );
    }
}
