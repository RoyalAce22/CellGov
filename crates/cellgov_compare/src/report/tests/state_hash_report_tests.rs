//! Both renderers surface a same-runner state-hash divergence.

use crate::compare::{
    Classification, CompareMode, CompareResult, MultiCompareResult, StateHashDivergence,
};
use crate::observation::ObservedHashes;
use crate::report::{format_human, format_json, format_multi_human, format_multi_json};
use crate::test_support::sample_observation;
use cellgov_trace::StateHash;

fn hashes(memory: u64, unit_status: u64, sync: u64) -> ObservedHashes {
    ObservedHashes {
        memory: StateHash::new(memory),
        unit_status: StateHash::new(unit_status),
        sync: StateHash::new(sync),
    }
}

fn divergent_result() -> CompareResult {
    CompareResult {
        classification: Classification::Divergence,
        mode: CompareMode::Strict,
        outcome_mismatch: None,
        memory_divergence: None,
        event_divergence: None,
        state_hash_divergence: Some(StateHashDivergence {
            expected: hashes(0x1, 0x2, 0x3),
            actual: hashes(0x1, 0x2, 0xabcd),
        }),
    }
}

#[test]
fn human_report_names_every_hash_pair() {
    let out = format_human(&divergent_result());
    assert!(out.contains("classification: DIVERGENCE"), "{out}");
    assert!(
        out.contains(
            "state_hashes: memory expected=0x0000000000000001 actual=0x0000000000000001 \
             unit_status expected=0x0000000000000002 actual=0x0000000000000002 \
             sync expected=0x0000000000000003 actual=0x000000000000abcd"
        ),
        "{out}"
    );
}

#[test]
fn json_report_carries_the_hash_pair_as_raw_u64s() {
    let obs = sample_observation();
    let json = format_json(&divergent_result(), &obs, &obs).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let d = &v["state_hash_divergence"];
    assert_eq!(d["expected"]["sync"], 3);
    assert_eq!(d["actual"]["sync"], 0xabcd);
    assert_eq!(d["expected"]["memory"], 1);
}

#[test]
fn multi_reports_carry_the_hash_pair_on_both_sub_results() {
    let obs = sample_observation();
    let settled = MultiCompareResult {
        classification: Classification::Divergence,
        mode: CompareMode::Strict,
        oracle_divergence: None,
        cellgov_result: Some(divergent_result()),
    };
    let unsettled = MultiCompareResult {
        classification: Classification::UnsettledOracle,
        mode: CompareMode::Strict,
        oracle_divergence: Some(divergent_result()),
        cellgov_result: None,
    };
    for (r, key) in [
        (&settled, "cellgov_result"),
        (&unsettled, "oracle_divergence"),
    ] {
        let text = format_multi_human(r, 1);
        assert!(
            text.contains("state_hashes: memory expected="),
            "{key}: {text}"
        );
        let json = format_multi_json(r, std::slice::from_ref(&obs), &obs).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            v[key]["state_hash_divergence"]["actual"]["sync"], 0xabcd,
            "{key}"
        );
    }
}

#[test]
fn json_report_omits_the_field_when_hashes_agree() {
    let obs = sample_observation();
    let mut r = divergent_result();
    r.state_hash_divergence = None;
    r.classification = Classification::Match;
    let json = format_json(&r, &obs, &obs).unwrap();
    assert!(!json.contains("\"state_hash_divergence\""));
}
