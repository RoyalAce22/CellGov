use super::*;
use cellgov_lv2_archive::CallerRow;
use cellgov_ps3_abi::lv2::census::PupCensusClass;

#[test]
fn the_committed_archives_the_report_reads_all_parse() {
    assert!(parse(&NAME, NAME_TSV).is_ok());
    assert!(parse(&CAPABILITY_GATE, GATE_TSV).is_ok());
    assert!(parse(&CALLER, CALLER_TSV).is_ok());
}

#[test]
fn caller_evidence_carries_every_site_of_the_module() {
    let caller = CallerRow {
        pup_sha256: "ab".repeat(32),
        module: "sys/a.sprx".to_string(),
        ordinal: 9,
        sites: vec![16, 32, 48],
    };
    let row = ReportRow::from(UnmodelledSyscall {
        ordinal: 9,
        hits: 2,
        names: Vec::new(),
        census: PupCensusClass::NotExtracted,
        gate: None,
        callers: vec![&caller],
    });
    assert_eq!(
        serde_json::to_string(&row).unwrap(),
        r#"{"ordinal":9,"hits":2,"names":[],"census":"not_extracted","gate":null,"caller_evidence":[{"module":"sys/a.sprx","sites":[16,32,48]}]}"#
    );
}
