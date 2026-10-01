//! The bootable microtests' design table against the console, with no
//! build and no boot: runnable on a fresh clone.
//!
//! - Each committed console frame meets the table: every `Exact` word
//!   holds its value and every `NonZero` word is not zero. The console
//!   confirms the design, independently of CellGov.
//! - Each manifest's `[ps3] volatile` ranges are exactly the table's
//!   `Any` words: an `Any` word outside every range would fail a byte
//!   comparison on a value the design leaves free, and a range over a
//!   fixed word would hide a real difference.

use std::path::{Path, PathBuf};

use cellgov_compare::console_profile::{ConsoleProfiles, CONSOLE_PROFILES_FILE};
use cellgov_compare::hardware_capture::{self, CAPTURE_DIR};
use cellgov_compare::manifest::{self, ConsoleManifest};

#[path = "microtests/cases.rs"]
#[allow(
    dead_code,
    reason = "the step cap is the boot suite's; this target boots nothing"
)]
mod cases;
use cases::{Any, Case, Exact, Expect, NonZero, CASES};

fn micro_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/micro")
}

fn manifest_of(case: &Case) -> ConsoleManifest {
    let path = micro_root().join(case.name).join("manifest.toml");
    manifest::load_console(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Each payload word that misses its expectation, by field name.
fn table_misses(case: &Case, payload: &[u8]) -> Vec<String> {
    if payload.len() != case.fields.len() * 4 {
        return vec![format!(
            "payload is {} bytes, the table describes {}",
            payload.len(),
            case.fields.len() * 4
        )];
    }
    payload
        .chunks_exact(4)
        .zip(case.fields)
        .filter_map(|(bytes, &(field, expect))| {
            let word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            match expect {
                Exact(want) if word != want => {
                    Some(format!("{field}: expected 0x{want:08x}, got 0x{word:08x}"))
                }
                NonZero if word == 0 => Some(format!("{field}: expected non-zero, got 0")),
                _ => None,
            }
        })
        .collect()
}

/// The payload byte offsets the manifest's volatile ranges cover, and
/// any range that names a region the manifest does not declare.
fn volatile_offsets(m: &ConsoleManifest) -> Result<Vec<u64>, String> {
    let mut covered = Vec::new();
    for range in &m.ps3.volatile {
        let region = m
            .observe
            .memory_regions
            .iter()
            .find(|r| r.name == range.region)
            .ok_or_else(|| format!("volatile range names undeclared region {}", range.region))?;
        let start = region.payload_offset() + range.offset;
        covered.extend(start..start + range.size);
    }
    covered.sort_unstable();
    covered.dedup();
    Ok(covered)
}

/// How the table's `Any` words and the manifest's volatile bytes
/// disagree: an `Any` word not wholly covered, or a covered byte in a
/// word that is not `Any`.
fn volatile_mismatches(fields: &[(&str, Expect)], covered: &[u64]) -> Vec<String> {
    let mut out = Vec::new();
    for (index, &(field, expect)) in fields.iter().enumerate() {
        let word: Vec<u64> = (index as u64 * 4..index as u64 * 4 + 4).collect();
        let inside = word.iter().filter(|b| covered.contains(b)).count();
        match (expect, inside) {
            (Any, 4) => {}
            (Any, _) => out.push(format!("{field} is Any but not wholly volatile")),
            (_, 0) => {}
            _ => out.push(format!(
                "{field} is fixed by the design but declared volatile"
            )),
        }
    }
    let end = fields.len() as u64 * 4;
    if let Some(past) = covered.iter().find(|&&b| b >= end) {
        out.push(format!(
            "a volatile byte at {past} lies past the {end}-byte payload"
        ));
    }
    out
}

#[test]
fn a_word_off_the_table_is_named_and_a_payload_of_another_length_is_refused() {
    let case = Case {
        name: "synthetic",
        max_steps: 1,
        fields: &[("status", Exact(0)), ("count", NonZero), ("noise", Any)],
    };
    assert!(table_misses(&case, &[0, 0, 0, 0, 0, 0, 0, 1, 9, 9, 9, 9]).is_empty());
    assert_eq!(
        table_misses(&case, &[0, 0, 0, 1, 0, 0, 0, 0, 9, 9, 9, 9]),
        [
            "status: expected 0x00000000, got 0x00000001",
            "count: expected non-zero, got 0"
        ]
    );
    assert!(table_misses(&case, &[0; 8])[0].contains("payload is 8 bytes"));
}

#[test]
fn volatile_bytes_must_cover_every_any_word_and_nothing_else() {
    let fields: &[(&str, Expect)] = &[("status", Exact(0)), ("noise", Any), ("count", NonZero)];
    assert!(volatile_mismatches(fields, &[4, 5, 6, 7]).is_empty());
    assert_eq!(
        volatile_mismatches(fields, &[4, 5, 6]),
        ["noise is Any but not wholly volatile"]
    );
    assert_eq!(
        volatile_mismatches(fields, &[4, 5, 6, 7, 8]),
        ["count is fixed by the design but declared volatile"]
    );
    assert_eq!(
        volatile_mismatches(fields, &[4, 5, 6, 7, 12]),
        ["a volatile byte at 12 lies past the 12-byte payload"]
    );
}

#[test]
fn every_hardware_capture_satisfies_the_design_table() {
    let profiles = ConsoleProfiles::load(&micro_root().join(CONSOLE_PROFILES_FILE))
        .expect("the tracked profiles load");
    let mut problems = Vec::new();
    for case in CASES {
        let capture = micro_root()
            .join(case.name)
            .join(CAPTURE_DIR)
            .join(&profiles.reference);
        if !capture.is_dir() {
            continue;
        }
        let loaded = hardware_capture::load(&capture, &profiles)
            .unwrap_or_else(|e| panic!("{}: {e}", capture.display()));
        // The frame is the 8-byte CGOV header, then the payload.
        let payload = loaded.frame.get(8..).unwrap_or_default();
        for miss in table_misses(case, payload) {
            problems.push(format!("{}: {miss}", case.name));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_volatile_ranges_agree_with_the_design_table() {
    let mut problems = Vec::new();
    for case in CASES {
        let m = manifest_of(case);
        match volatile_offsets(&m) {
            Ok(covered) => {
                for mismatch in volatile_mismatches(case.fields, &covered) {
                    problems.push(format!("{}: {mismatch}", case.name));
                }
            }
            Err(e) => problems.push(format!("{}: {e}", case.name)),
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
