use super::*;
use crate::test_support::build_param_sfo;

fn sfo(entries: &[(&str, &str)]) -> ParamSfo {
    parse(&build_param_sfo(entries)).expect("synthetic PARAM.SFO parses")
}

#[test]
fn app_ver_wins_over_version() {
    let table = sfo(&[("APP_VER", "01.05"), ("VERSION", "01.02")]);
    assert_eq!(
        table.named_version(),
        Some((SfoVersionKey::AppVer, "01.05"))
    );
}

#[test]
fn version_stands_in_when_app_ver_is_absent() {
    let table = sfo(&[("TITLE_ID", "NPAA00001"), ("VERSION", "01.02")]);
    assert_eq!(
        table.named_version(),
        Some((SfoVersionKey::Version, "01.02"))
    );
}

#[test]
fn an_empty_app_ver_falls_through_to_version() {
    let table = sfo(&[("APP_VER", ""), ("VERSION", "01.02")]);
    assert_eq!(
        table.named_version(),
        Some((SfoVersionKey::Version, "01.02"))
    );
}

#[test]
fn a_table_naming_no_version_reads_as_none() {
    assert_eq!(sfo(&[("TITLE_ID", "NPAA00001")]).named_version(), None);
    assert_eq!(
        sfo(&[("APP_VER", ""), ("VERSION", "")]).named_version(),
        None
    );
}

#[test]
fn the_key_spells_itself_as_the_table_does() {
    assert_eq!(SfoVersionKey::AppVer.as_str(), "APP_VER");
    assert_eq!(SfoVersionKey::Version.as_str(), "VERSION");
}
