//! A committed result measured under a boot override answers for no
//! cell.

use cellgov_compare::{AppVersion, BootOutcome, BootOverrides};

use super::super::test_fixtures::*;
use super::*;

fn boot_under(overrides: BootOverrides) -> BootSummary {
    let mut b = boot(BootOutcome::ProcessExit, 1_000);
    b.identity = RunIdentity {
        firmware: Some(firmware(REFERENCE_FW)),
        game: Some(GameIdentity {
            title_id: "NPAA61100".to_string(),
            version: BASE.to_string(),
            app_version: Some(AppVersion::AppVer("01.00".to_string())),
        }),
        overrides,
    };
    b
}

#[test]
fn an_anchor_under_an_override_does_not_answer_for_its_cell() {
    let fixtures = Fixtures::new("override-anchor");
    let t = title("NPAA61100", "Overridden", 2008, "Studio");
    fixtures.write_anchor(
        "NPAA61100",
        &reference_key(),
        &boot_under(BootOverrides {
            force_system_authid: true,
            ..BootOverrides::default()
        }),
    );
    match load_title(&t, fixtures.path()) {
        Err(SummaryLoadError::CellOverridden { overrides, .. }) => {
            assert_eq!(overrides, "force_system_authid");
        }
        other => panic!("expected an override refusal, got {other:?}"),
    }
}

#[test]
fn a_cross_runner_summary_under_an_override_does_not_answer_for_its_cell() {
    let fixtures = Fixtures::new("override-cross");
    let t = title("NPAA61100", "Overridden", 2008, "Studio");
    let mut summary = stamped(converged(0), Some(REFERENCE_FW), Some(REFERENCE_FW));
    summary.identity.overrides = BootOverrides {
        prx_base: Some(0x3000_0000),
        ..BootOverrides::default()
    };
    fixtures.write_cross("NPAA61100", &reference_key(), &summary);
    match load_title(&t, fixtures.path()) {
        Err(SummaryLoadError::CellOverridden { path, overrides }) => {
            assert_eq!(
                path,
                cross_path(fixtures.path(), "NPAA61100", &reference_key())
            );
            assert_eq!(overrides, "prx_base=0x30000000");
        }
        other => panic!("expected an override refusal, got {other:?}"),
    }
}

#[test]
fn an_anchor_under_no_override_loads() {
    let fixtures = Fixtures::new("override-none");
    let t = title("NPAA61100", "Overridden", 2008, "Studio");
    fixtures.write_anchor(
        "NPAA61100",
        &reference_key(),
        &boot_under(BootOverrides::default()),
    );
    let docs = load_title(&t, fixtures.path()).unwrap_or_else(|e| panic!("{e}"));
    assert!(docs
        .reference()
        .expect("the reference cell is declared")
        .artifacts
        .boot
        .is_some());
}
