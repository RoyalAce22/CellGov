//! The `[ps3]` table: the defaults with and without the table, the
//! derived result file, and the refusals for a non-portable test with no
//! reason and for a volatile range outside its region.

use super::*;

/// A manifest with one 16-byte region named `result`, plus `ps3` verbatim.
fn manifest_with(ps3: &str) -> Result<Manifest, ManifestError> {
    parse(&format!(
        r#"
[test]
name = "widget"

[observe]
memory_regions = [
  {{ name = "result", addr = 256, size = 16 }},
]

[expect]
outcome = "completed"

{ps3}
"#
    ))
}

fn assert_defaults(m: &Manifest) {
    assert_eq!(m.ps3.appid, "CGOV00001");
    assert_eq!(m.ps3.timeout_ms, 30_000);
    assert_eq!(m.ps3.result_file, None);
    assert_eq!(m.result_file_name(), "cgov_widget.bin");
    assert!(m.ps3.portable);
    assert_eq!(m.ps3.not_portable_reason, None);
    assert!(m.ps3.files.is_empty());
    assert!(m.ps3.volatile.is_empty());
}

#[test]
fn a_manifest_without_the_table_takes_every_default() {
    let m = manifest_with("").expect("parse");
    assert_defaults(&m);
}

#[test]
fn an_empty_table_takes_the_same_defaults() {
    let m = manifest_with("[ps3]").expect("parse");
    assert_defaults(&m);
}

#[test]
fn a_full_table_parses() {
    let m = manifest_with(
        r#"
[ps3]
appid = "CGOV00002"
timeout_ms = 5000
result_file = "widget_out.bin"
portable = false
not_portable_reason = "needs a second SPU thread group"
files = ["spu_main.elf", "child.self"]
volatile = [
  { region = "result", offset = 12, size = 4, reason = "retry counter" },
]
"#,
    )
    .expect("parse");
    assert_eq!(m.ps3.appid, "CGOV00002");
    assert_eq!(m.ps3.timeout_ms, 5000);
    assert_eq!(m.ps3.result_file.as_deref(), Some("widget_out.bin"));
    assert_eq!(m.result_file_name(), "widget_out.bin");
    assert!(!m.ps3.portable);
    assert_eq!(
        m.ps3.not_portable_reason.as_deref(),
        Some("needs a second SPU thread group")
    );
    assert_eq!(m.ps3.files, vec!["spu_main.elf", "child.self"]);
    assert_eq!(
        m.ps3.volatile,
        vec![VolatileRange {
            region: "result".to_string(),
            offset: 12,
            size: 4,
            reason: "retry counter".to_string(),
        }]
    );
}

#[test]
fn portable_false_needs_a_reason() {
    let err = manifest_with("[ps3]\nportable = false").expect_err("refused");
    assert!(
        matches!(err, ManifestError::NotPortableWithoutReason),
        "{err:?}"
    );
    let m = manifest_with("[ps3]\nportable = false\nnot_portable_reason = \"no RSX on the bench\"")
        .expect("a reason makes it whole");
    assert!(!m.ps3.portable);
}

#[test]
fn a_volatile_range_must_name_a_declared_region() {
    let err = manifest_with(
        r#"[ps3]
volatile = [{ region = "header", offset = 0, size = 4, reason = "x" }]"#,
    )
    .expect_err("refused");
    match err {
        ManifestError::VolatileRegionUnknown { index, region } => {
            assert_eq!(index, 0);
            assert_eq!(region, "header");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_volatile_range_ending_at_the_region_end_is_accepted_and_one_past_it_is_refused() {
    let m = manifest_with(
        r#"[ps3]
volatile = [{ region = "result", offset = 12, size = 4, reason = "x" }]"#,
    )
    .expect("offset 12 + 4 bytes ends at byte 16");
    assert_eq!(m.ps3.volatile.len(), 1);

    let err = manifest_with(
        r#"[ps3]
volatile = [
  { region = "result", offset = 0, size = 4, reason = "x" },
  { region = "result", offset = 12, size = 5, reason = "y" },
]"#,
    )
    .expect_err("refused");
    match err {
        ManifestError::VolatileRangeOutsideRegion {
            index,
            region,
            offset,
            size,
            region_size,
        } => {
            assert_eq!(index, 1);
            assert_eq!(region, "result");
            assert_eq!(offset, 12);
            assert_eq!(size, 5);
            assert_eq!(region_size, 16);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_volatile_range_whose_end_overflows_is_refused_not_wrapped() {
    let err = manifest_with(
        r#"[ps3]
volatile = [{ region = "result", offset = 18446744073709551615, size = 1, reason = "x" }]"#,
    )
    .expect_err("refused");
    assert!(
        matches!(
            err,
            ManifestError::VolatileRangeOutsideRegion {
                offset: u64::MAX,
                size: 1,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn an_empty_volatile_range_is_refused() {
    let err = manifest_with(
        r#"[ps3]
volatile = [{ region = "result", offset = 4, size = 0, reason = "x" }]"#,
    )
    .expect_err("refused");
    assert!(
        matches!(err, ManifestError::VolatileRangeEmpty { index: 0, .. }),
        "{err:?}"
    );
}

#[test]
fn a_zero_budget_is_refused() {
    let err = manifest_with("[ps3]\ntimeout_ms = 0").expect_err("refused");
    assert!(matches!(err, ManifestError::ZeroTimeout), "{err:?}");
    let m = manifest_with("[ps3]\ntimeout_ms = 1").expect("one millisecond is a budget");
    assert_eq!(m.ps3.timeout_ms, 1);
}

#[test]
fn a_file_name_that_could_leave_its_directory_is_refused() {
    for (table, field, value) in [
        ("[ps3]\nresult_file = \"\"", "result_file", ""),
        (
            "[ps3]\nresult_file = \"../out.bin\"",
            "result_file",
            "../out.bin",
        ),
        (
            "[ps3]\nfiles = [\"spu_main.elf\", \"sub\\\\x.elf\"]",
            "files",
            "sub\\x.elf",
        ),
        ("[ps3]\nfiles = [\"..\"]", "files", ".."),
        ("[ps3]\nappid = \"CGOV00001/x\"", "appid", "CGOV00001/x"),
        ("[ps3]\nappid = \"\"", "appid", ""),
    ] {
        let err = manifest_with(table).expect_err(table);
        match err {
            ManifestError::NotABareName {
                field: got_field,
                value: got_value,
            } => {
                assert_eq!(got_field, field, "{table}");
                assert_eq!(got_value, value, "{table}");
            }
            other => panic!("{table}: {other:?}"),
        }
    }
    let m = manifest_with("[ps3]\nresult_file = \"out.bin\"\nfiles = [\"a.elf\", \"b.self\"]")
        .expect("bare names pass");
    assert_eq!(m.ps3.files, vec!["a.elf", "b.self"]);
}

#[test]
fn an_unknown_field_in_the_table_is_a_parse_error() {
    let err = manifest_with("[ps3]\ntimeout = 5000").expect_err("refused");
    assert!(matches!(err, ManifestError::Parse(_)), "{err:?}");
    let err = manifest_with(
        r#"[ps3]
volatile = [{ region = "result", offset = 0, size = 4, why = "x" }]"#,
    )
    .expect_err("refused");
    assert!(matches!(err, ManifestError::Parse(_)), "{err:?}");
}

/// The boot-run shape of `[cellgov]`: a title table, no `scenario`.
const TITLE_BOOT_MANIFEST: &str = r#"
[test]
name = "widget"

[observe]
memory_regions = [
  { name = "result", addr = 256, size = 16 },
]

[expect]
outcome = "completed"

[cellgov.title]
short_name = "widget"
display_name = "widget microtest"
eboot_candidates = ["widget.elf"]

[cellgov.source]
kind = "manifest-relative"
path = "build"

[ps3]
files = ["spu_main.elf"]
"#;

#[test]
fn the_console_view_reads_a_title_boot_manifest_the_full_parser_refuses() {
    let err = parse(TITLE_BOOT_MANIFEST).expect_err("no scenario");
    assert!(matches!(err, ManifestError::Parse(_)), "{err:?}");
    let m = parse_console(TITLE_BOOT_MANIFEST).expect("the console view skips [cellgov]");
    assert_eq!(m.test.name, "widget");
    assert_eq!(m.observe.memory_regions.len(), 1);
    assert!(matches!(m.expect.outcome, OutcomeField::Completed));
    assert_eq!(m.ps3.files, vec!["spu_main.elf"]);
    assert_eq!(m.result_file_name(), "cgov_widget.bin");
}

#[test]
fn the_console_view_applies_the_same_refusals_and_defaults() {
    let m = parse_console(&TITLE_BOOT_MANIFEST.replace("files = [\"spu_main.elf\"]", ""))
        .expect("parse");
    assert_eq!(m.ps3.appid, "CGOV00001");
    assert_eq!(m.ps3.timeout_ms, 30_000);
    assert!(m.ps3.files.is_empty());

    let err = parse_console(&TITLE_BOOT_MANIFEST.replace(
        "files = [\"spu_main.elf\"]",
        "volatile = [{ region = \"result\", offset = 16, size = 1, reason = \"x\" }]",
    ))
    .expect_err("refused");
    assert!(
        matches!(
            err,
            ManifestError::VolatileRangeOutsideRegion { offset: 16, .. }
        ),
        "{err:?}"
    );
    let err = parse_console(
        &TITLE_BOOT_MANIFEST.replace("files = [\"spu_main.elf\"]", "portable = false"),
    )
    .expect_err("refused");
    assert!(
        matches!(err, ManifestError::NotPortableWithoutReason),
        "{err:?}"
    );
}

#[test]
fn the_refusals_name_the_range_and_the_region() {
    let err = manifest_with(
        r#"[ps3]
volatile = [{ region = "result", offset = 12, size = 5, reason = "x" }]"#,
    )
    .expect_err("refused");
    assert_eq!(
        err.to_string(),
        "manifest [ps3]: volatile range 0 at offset 12 of 5 bytes runs past region \"result\" of 16 bytes"
    );
}
