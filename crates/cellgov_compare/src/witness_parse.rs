//! Parse the `BENCH_*` witness lines a boot writes to stderr.
//!
//! The recording path and the asserting path both call
//! [`parse_witness_lines`], so a value cannot be recorded under one
//! reading and checked under another.

use std::collections::{BTreeMap, BTreeSet};

/// One `BENCH_*` line: `(prefix, tracked (key, witness) pairs, ignored keys)`.
type LineSpec = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static [&'static str],
);

/// Every `BENCH_*` line the boot path emits: its tracked witness
/// fields and the keys it may carry that no witness tracks.
///
/// A line's tokens are `name=value` pairs. Single-value lines spell
/// their token `count`, so the table renames it to the witness name;
/// multi-value lines already name each field and map to themselves.
const LINE_TABLE: &[LineSpec] = &[
    (
        "BENCH_VRSAVE_WITNESS:",
        &[
            ("mfvrsave_executed", "mfvrsave_executed"),
            ("vrsave_written", "vrsave_written"),
        ],
        &[],
    ),
    (
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS:",
        &[("count", "host_invariant_breaks")],
        &[],
    ),
    (
        "BENCH_ATOMIC_WITNESS:",
        &[
            ("ldarx", "ldarx"),
            ("stdcx", "stdcx"),
            ("lwarx", "lwarx"),
            ("stwcx", "stwcx"),
        ],
        &[],
    ),
    (
        "BENCH_MEM_FAULT_WITNESS:",
        &[
            ("arm_entries", "mem_fault_arm_entries"),
            ("unmapped_routed", "mem_fault_unmapped_routed"),
        ],
        &[],
    ),
    (
        // Timer sleeps bypass Lv2Host::dispatch (the TIMER_USLEEP /
        // TIMER_SLEEP fast path), so no dispatch-side witness counts
        // them.
        "BENCH_TIMER_SLEEP_WITNESS:",
        &[("count", "timer_sleeps")],
        &[],
    ),
    (
        "BENCH_RSX_LABEL_WRITES_WITNESS:",
        &[("count", "rsx_label_writes")],
        &[],
    ),
    (
        "BENCH_RSX_SET_REFERENCE_WITNESS:",
        &[("count", "rsx_set_reference")],
        &[],
    ),
    ("BENCH_DCBZ_WITNESS:", &[("count", "dcbz")], &[]),
    (
        "BENCH_SPU_IMAGE_REGISTER_WITNESS:",
        &[("count", "spu_image_register")],
        &[],
    ),
    (
        "BENCH_SPU_THREAD_INIT_WITNESS:",
        &[("count", "spu_thread_init")],
        &[],
    ),
    (
        "BENCH_LWMUTEX_COND_WITNESS:",
        &[
            ("lwmutex_acquires", "lwmutex_acquires"),
            ("lwmutex_releases", "lwmutex_releases"),
            ("cond_reacquires", "cond_reacquires"),
        ],
        &[],
    ),
    (
        "BENCH_AUTHORITY_ID_WITNESS:",
        &[("lwmutex_unknown_locks", "lwmutex_unknown_locks")],
        // Consumed by the authority-id suite directly, not tracked as
        // anchored witnesses.
        &["program_authority_id", "authid_source"],
    ),
    (
        "BENCH_PRX_LOAD_WITNESS:",
        &[
            ("hle_stubs", "prx_load_hle_stubs"),
            ("not_found", "prx_load_not_found"),
        ],
        &[],
    ),
];

/// A token that did not parse as a witness value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{line_prefix} token {token:?}: {reason}")]
pub struct WitnessParseError {
    /// `BENCH_*` prefix the bad token appeared under.
    pub line_prefix: String,
    /// The offending `name=value` token.
    pub token: String,
    /// Why it was rejected.
    pub reason: String,
}

/// Everything one boot's stderr said through its witness lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedWitnesses {
    /// Witness values keyed by witness name.
    pub values: BTreeMap<String, u64>,
    /// `BENCH_*` prefixes that appeared. Lets a checker distinguish
    /// "this line reported 0" from "this line was never emitted".
    pub seen_lines: BTreeSet<&'static str>,
}

/// Extract every witness value from a boot's stderr.
///
/// Booleans are recorded as 0 or 1 so one numeric type covers the set.
///
/// # Errors
///
/// Returns every malformed token found -- including keys the table
/// neither tracks nor lists as ignored, and a recognized line emitted
/// more than once. A run with errors must not be recorded or asserted
/// against.
pub fn parse_witness_lines(stderr: &str) -> Result<ParsedWitnesses, Vec<WitnessParseError>> {
    let mut out = ParsedWitnesses {
        values: BTreeMap::new(),
        seen_lines: BTreeSet::new(),
    };
    let mut errors = Vec::new();

    for line in stderr.lines() {
        let line = line.trim();
        let Some((prefix, fields, ignored)) = LINE_TABLE
            .iter()
            .find(|(p, _, _)| line.starts_with(p))
            .map(|(p, f, i)| (*p, *f, *i))
        else {
            continue;
        };
        if !out.seen_lines.insert(prefix) {
            errors.push(WitnessParseError {
                line_prefix: prefix.to_string(),
                token: String::new(),
                reason: "line emitted more than once; a duplicate silently \
                         overwriting the first would hide which boot the \
                         values came from"
                    .to_string(),
            });
            continue;
        }
        for token in line[prefix.len()..].split_whitespace() {
            let Some((key, raw)) = token.split_once('=') else {
                errors.push(WitnessParseError {
                    line_prefix: prefix.to_string(),
                    token: token.to_string(),
                    reason: "expected name=value".to_string(),
                });
                continue;
            };
            let Some((_, witness)) = fields.iter().find(|(k, _)| *k == key) else {
                if !ignored.contains(&key) {
                    errors.push(WitnessParseError {
                        line_prefix: prefix.to_string(),
                        token: token.to_string(),
                        reason: format!(
                            "key {key:?} is neither tracked nor listed as \
                             ignored; if the emitter grew a field, add it to \
                             the line table"
                        ),
                    });
                }
                continue;
            };
            let value = match raw {
                "true" => Some(1),
                "false" => Some(0),
                _ => raw.parse::<u64>().ok(),
            };
            match value {
                Some(v) => {
                    out.values.insert((*witness).to_string(), v);
                }
                None => errors.push(WitnessParseError {
                    line_prefix: prefix.to_string(),
                    token: token.to_string(),
                    reason: format!("value {raw:?} is neither a u64 nor a bool"),
                }),
            }
        }
    }

    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// Witness names the boot path can emit.
///
/// Lets a caller report which recorded witnesses this build no longer
/// produces.
pub fn known_witness_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = LINE_TABLE
        .iter()
        .flat_map(|(_, fields, _)| fields.iter().map(|(_, w)| *w))
        .collect();
    names.sort_unstable();
    names
}

/// The `BENCH_*` line that emits `witness`, or `None` for a name this
/// build does not know.
pub fn line_of(witness: &str) -> Option<&'static str> {
    LINE_TABLE
        .iter()
        .find(|(_, fields, _)| fields.iter().any(|(_, w)| *w == witness))
        .map(|(p, _, _)| *p)
}
