use super::*;
use crate::archive::{parse, render};

fn row(fw: &str, order: u64, date: Option<&str>, priority: u64, role: FirmwareRole) -> FirmwareRow {
    FirmwareRow {
        fw: fw.to_string(),
        order,
        release_date: date.map(str::to_string),
        priority,
        role,
    }
}

fn spine() -> Vec<FirmwareRow> {
    vec![
        row("1.00", 100, Some("2006-11-11"), 3, FirmwareRole::Baseline),
        row("1.50", 150, Some("2007-01-24"), 1, FirmwareRole::None),
        row(
            "3.55",
            355,
            Some("2010-12-07"),
            1,
            FirmwareRole::CensusReference,
        ),
        row("3.60", 360, Some("2011-03-09"), 2, FirmwareRole::None),
        row("4.00", 400, Some("2011-11-30"), 2, FirmwareRole::None),
        row("4.93", 493, None, 1, FirmwareRole::Final),
    ]
}

#[test]
fn every_role_has_a_distinct_label_that_round_trips() {
    let labels: Vec<&str> = FirmwareRole::ALL.iter().map(|r| r.label()).collect();
    assert_eq!(labels, ["baseline", "census_reference", "final", "none"]);
    for role in FirmwareRole::ALL {
        assert_eq!(FirmwareRole::from_label(role.label()), Some(*role));
    }
    assert_eq!(FirmwareRole::from_label("reference"), None);
}

#[test]
fn a_version_key_is_one_or_two_digits_a_dot_and_two_digits() {
    for good in ["1.00", "4.93", "10.00", "3.55"] {
        assert!(is_version_key(good), "{good}");
    }
    for bad in [
        "1", "1.0", "1.000", "01.5000", "01.50", "x.55", "1.5a", "", ".55", "1.",
    ] {
        assert!(!is_version_key(bad), "{bad}");
    }
}

#[test]
fn a_priority_outside_one_to_three_is_refused() {
    for priority in [0, 4, u64::MAX] {
        let mut rows = spine();
        rows[1].priority = priority;
        assert_eq!(
            check_firmware_rows(&rows),
            Err(FirmwareTableError::Priority {
                fw: "1.50".to_string(),
                priority,
                expected: "1, 2 or 3",
            })
        );
    }
}

#[test]
fn the_census_reference_carries_priority_one() {
    let mut rows = spine();
    rows[2].priority = 2;
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::Priority {
            fw: "3.55".to_string(),
            priority: 2,
            expected: "1, which the census reference carries",
        })
    );
}

#[test]
fn priority_two_is_reserved_for_the_two_era_openers() {
    let mut rows = spine();
    rows[1].priority = 2;
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::Priority {
            fw: "1.50".to_string(),
            priority: 2,
            expected: "1 or 3; priority 2 belongs to 3.60 and 4.00",
        })
    );

    for index in [3, 4] {
        let mut rows = spine();
        rows[index].priority = 3;
        let fw = rows[index].fw.clone();
        assert_eq!(
            check_firmware_rows(&rows),
            Err(FirmwareTableError::Priority {
                fw,
                priority: 3,
                expected: "1 or 2; 3.60 and 4.00 have raised priority",
            })
        );
    }

    let mut rows = spine();
    rows[3].priority = 1;
    assert_eq!(check_firmware_rows(&rows), Ok(()));

    for (index, fw) in [(3, "3.60"), (4, "4.00")] {
        let mut rows = spine();
        rows.remove(index);
        assert_eq!(
            check_firmware_rows(&rows),
            Err(FirmwareTableError::RequiredRow {
                fw,
                purpose: "priority 2 era opener",
            })
        );
    }
}

#[test]
fn a_well_formed_spine_passes() {
    assert_eq!(check_firmware_rows(&spine()), Ok(()));
}

#[test]
fn an_order_that_does_not_rise_is_refused_naming_both_rows() {
    let mut rows = spine();
    rows[2].order = 150;
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::OrderNotRising {
            fw: "3.55".to_string(),
            order: 150,
            previous: 150,
        })
    );
    rows[2].order = 149;
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::OrderNotRising {
            fw: "3.55".to_string(),
            order: 149,
            previous: 150,
        })
    );
}

#[test]
fn a_cell_that_is_not_a_version_key_or_a_date_is_refused() {
    let mut rows = spine();
    rows[1].fw = "01.5000".to_string();
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::NotAVersionKey {
            fw: "01.5000".to_string()
        })
    );
    let mut rows = spine();
    rows[1].release_date = Some("2006-13-01".to_string());
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::BadDate {
            fw: "1.50".to_string(),
            date: "2006-13-01".to_string()
        })
    );
    for bad in [
        "2006-1-17",
        "20061117",
        "2006-00-17",
        "2006-11-00",
        "2006-11-32",
        "2006-02-29",
        "2006-04-31",
        "2006-11",
        "2006-11-17-1",
    ] {
        let mut rows = spine();
        rows[1].release_date = Some(bad.to_string());
        assert!(
            matches!(
                check_firmware_rows(&rows),
                Err(FirmwareTableError::BadDate { .. })
            ),
            "{bad}"
        );
    }
    let mut rows = spine();
    rows[1].release_date = Some("2008-02-29".to_string());
    assert_eq!(check_firmware_rows(&rows), Ok(()));
}

#[test]
fn the_three_single_roles_sit_on_exactly_one_row_each() {
    let mut rows = spine();
    rows[1].role = FirmwareRole::Final;
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::RoleCount {
            role: "final",
            count: 2
        })
    );
    let mut rows = spine();
    rows[0].role = FirmwareRole::None;
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::RoleCount {
            role: "baseline",
            count: 0
        })
    );
}

#[test]
fn each_single_role_sits_on_the_row_its_definition_names() {
    let mut rows = spine();
    rows[0].role = FirmwareRole::None;
    rows[1].role = FirmwareRole::Baseline;
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::RoleRow {
            role: "baseline",
            fw: "1.50".to_string(),
            expected: "1.00".to_string(),
        })
    );

    let mut rows = spine();
    rows[1].role = FirmwareRole::CensusReference;
    rows[2].role = FirmwareRole::None;
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::RoleRow {
            role: "census_reference",
            fw: "1.50".to_string(),
            expected: "3.55".to_string(),
        })
    );

    let mut rows = spine();
    rows[1].role = FirmwareRole::Final;
    rows[5].role = FirmwareRole::None;
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::RoleRow {
            role: "final",
            fw: "1.50".to_string(),
            expected: "4.93".to_string(),
        })
    );

    let mut rows = spine();
    rows[5].fw = "5.00".to_string();
    rows[5].order = 500;
    assert_eq!(
        check_firmware_rows(&rows),
        Err(FirmwareTableError::RoleRow {
            role: "final",
            fw: "5.00".to_string(),
            expected: "the last 4.9x row".to_string(),
        })
    );
}

#[test]
fn the_rendered_table_loads_and_reads_back() {
    let text = "fw\torder\trelease_date\tpriority\trole\n\
                1.00\t100\t2006-11-11\t3\tbaseline\n\
                1.50\t150\t2007-01-24\t1\tnone\n\
                3.55\t355\t2010-12-07\t1\tcensus_reference\n\
                3.60\t360\t2011-03-09\t2\tnone\n\
                4.00\t400\t2011-11-30\t2\tnone\n\
                4.93\t493\tnone\t1\tfinal\n";
    let table = parse(&FIRMWARE, text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(firmware_rows(&table), spine());
    assert_eq!(render(&FIRMWARE, &table.rows), Ok(text.to_string()));
    let twice = format!("{text}4.93\t494\tnone\t1\tnone\n");
    assert!(parse(&FIRMWARE, &twice).is_err());
    let unsorted = format!("{text}2.00\t200\tnone\t2\tnone\n");
    assert!(parse(&FIRMWARE, &unsorted).is_err());
}
