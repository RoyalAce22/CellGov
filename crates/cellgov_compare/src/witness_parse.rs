//! Parse the `BENCH_*` witness lines a boot writes to stderr.
//!
//! The recording path and the asserting path both call
//! [`parse_witness_lines`], so a value cannot be recorded under one
//! reading and checked under another.

use std::collections::{BTreeMap, BTreeSet};

/// Keys a line may carry beyond its tracked fields.
#[derive(Debug, Clone, Copy)]
enum Extra {
    /// Every key on the line is tracked.
    None,
    /// Named keys another consumer reads.
    Keys(&'static [&'static str]),
    /// Any key that parses as an integer: a per-item inventory whose
    /// key set varies by run, trailing the line's tracked scalars.
    IntegerKeys,
}

impl Extra {
    fn admits(self, key: &str) -> bool {
        match self {
            Self::None => false,
            Self::Keys(keys) => keys.contains(&key),
            Self::IntegerKeys => key.parse::<u64>().is_ok(),
        }
    }
}

/// One `BENCH_*` line: `(prefix, tracked (key, witness) pairs, extra keys)`.
type LineSpec = (&'static str, &'static [(&'static str, &'static str)], Extra);

/// Every `BENCH_*` line the boot path emits that carries a witness:
/// its tracked fields and the keys it may carry that no witness
/// tracks. Lines that carry none are listed in [`DIAGNOSTIC_LINES`].
///
/// A line's tokens are `name=value` pairs. Single-value lines spell
/// their token `count`, so the table renames it to the witness name;
/// multi-value lines already name each field and map to themselves,
/// prefixed with the line's subject where the bare key would be
/// ambiguous across lines.
const LINE_TABLE: &[LineSpec] = &[
    (
        "BENCH_VRSAVE_WITNESS:",
        &[
            ("mfvrsave_executed", "mfvrsave_executed"),
            ("vrsave_written", "vrsave_written"),
        ],
        Extra::None,
    ),
    (
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS:",
        &[("count", "host_invariant_breaks")],
        Extra::None,
    ),
    (
        "BENCH_ATOMIC_WITNESS:",
        &[
            ("ldarx", "ldarx"),
            ("stdcx", "stdcx"),
            ("lwarx", "lwarx"),
            ("stwcx", "stwcx"),
        ],
        Extra::None,
    ),
    (
        "BENCH_MEM_FAULT_WITNESS:",
        &[
            ("arm_entries", "mem_fault_arm_entries"),
            ("unmapped_routed", "mem_fault_unmapped_routed"),
        ],
        Extra::None,
    ),
    (
        // Timer sleeps bypass Lv2Host::dispatch (the TIMER_USLEEP /
        // TIMER_SLEEP fast path), so no dispatch-side witness counts
        // them.
        "BENCH_TIMER_SLEEP_WITNESS:",
        &[("count", "timer_sleeps")],
        Extra::None,
    ),
    (
        "BENCH_RSX_LABEL_WRITES_WITNESS:",
        &[("count", "rsx_label_writes")],
        Extra::None,
    ),
    (
        "BENCH_RSX_SET_REFERENCE_WITNESS:",
        &[("count", "rsx_set_reference")],
        Extra::None,
    ),
    ("BENCH_DCBZ_WITNESS:", &[("count", "dcbz")], Extra::None),
    (
        "BENCH_SPU_IMAGE_REGISTER_WITNESS:",
        &[("count", "spu_image_register")],
        Extra::None,
    ),
    (
        "BENCH_SPU_THREAD_INIT_WITNESS:",
        &[("count", "spu_thread_init")],
        Extra::None,
    ),
    (
        "BENCH_LWMUTEX_COND_WITNESS:",
        &[
            ("lwmutex_acquires", "lwmutex_acquires"),
            ("lwmutex_releases", "lwmutex_releases"),
            ("cond_reacquires", "cond_reacquires"),
        ],
        Extra::None,
    ),
    (
        "BENCH_AUTHORITY_ID_WITNESS:",
        &[("lwmutex_unknown_locks", "lwmutex_unknown_locks")],
        // Consumed by the authority-id suite directly, not tracked as
        // anchored witnesses.
        Extra::Keys(&["program_authority_id", "authid_source"]),
    ),
    (
        "BENCH_MUTEX_UNLOCK_WITNESS:",
        &[("not_owner", "mutex_unlock_not_owner")],
        Extra::None,
    ),
    (
        "BENCH_REGISTER_MODULE_WITNESS:",
        &[
            ("calls", "register_module_calls"),
            ("manual", "register_module_manual"),
            ("linked_slots", "register_module_linked_slots"),
            ("unresolved_nids", "register_module_unresolved_nids"),
        ],
        Extra::None,
    ),
    (
        "BENCH_EVENT_PORT_WITNESS:",
        &[
            ("ipc_connect_attempts", "event_port_ipc_connect_attempts"),
            ("ipc_connect_bound", "event_port_ipc_connect_bound"),
            ("keyed_queues", "event_port_keyed_queues"),
        ],
        Extra::None,
    ),
    (
        // The tail is the per-syscall inventory, `<number>=<hits>`.
        "BENCH_UNSUPPORTED_SYSCALL_WITNESS:",
        &[("distinct", "unsupported_syscalls_distinct")],
        Extra::IntegerKeys,
    ),
    (
        "BENCH_SYSTEM_IPC_WITNESS:",
        &[
            ("shm_creates", "system_ipc_shm_creates"),
            ("shm_attaches", "system_ipc_shm_attaches"),
            ("shm_maps", "system_ipc_shm_maps"),
            ("shm_writes", "system_ipc_shm_writes"),
            ("cond_creates", "system_ipc_cond_creates"),
            ("cond_waits", "system_ipc_cond_waits"),
            ("cond_signals", "system_ipc_cond_signals"),
            ("event_queue_creates", "system_ipc_event_queue_creates"),
            (
                "event_queue_references",
                "system_ipc_event_queue_references",
            ),
            ("event_queue_enqueues", "system_ipc_event_queue_enqueues"),
            ("event_port_connects", "system_ipc_event_port_connects"),
            ("distinct_keys", "system_ipc_distinct_keys"),
        ],
        Extra::None,
    ),
    (
        "BENCH_PRX_LOAD_WITNESS:",
        &[
            ("hle_stubs", "prx_load_hle_stubs"),
            ("not_found", "prx_load_not_found"),
        ],
        Extra::None,
    ),
];

/// `BENCH_*` lines the boot path emits that no witness is recorded
/// from, each with the reason it stays diagnostic-only.
///
/// A line belongs here when its key set varies by run (an inventory
/// keyed by code, site, path, or key), when it repeats within one
/// boot, or when it is suppressed on the quiet path so a baseline
/// could not hold it `Absent`. The gate in the CLI's bench tests
/// holds every emitted line to one of the two tables.
const DIAGNOSTIC_LINES: &[(&str, &str)] = &[
    (
        "BENCH_HOST_INVARIANT_BREAK_SITES:",
        "per-site inventory with a run-dependent key set; the total is tracked as host_invariant_breaks",
    ),
    (
        "BENCH_DISPATCH_RETURN_WITNESS:",
        "per-return-code inventory with a run-dependent key set, suppressed when empty",
    ),
    (
        "BENCH_DISPATCH_RETURN_PAIRS:",
        "per-(arm, code) inventory with a run-dependent key set, suppressed when empty",
    ),
    (
        "BENCH_PARK_TIMEOUT_WITNESS:",
        "per-(arm, timeout) inventory with a run-dependent key set, suppressed when empty",
    ),
    (
        "BENCH_WAIT_EXPIRY_WITNESS:",
        "per-primitive inventory with a run-dependent key set, suppressed when empty",
    ),
    (
        "BENCH_SYSTEM_IPC_KEYS:",
        "per-key inventory; its size is tracked as system_ipc_distinct_keys",
    ),
    (
        "BENCH_PRX_LOAD_MISSES:",
        "per-path inventory whose quoted paths may contain spaces; the totals are tracked on BENCH_PRX_LOAD_WITNESS",
    ),
    (
        "BENCH_FINAL_UNIT_WITNESS:",
        "one line per live PPU unit: the terminal parking map, not a counter",
    ),
    (
        "BENCH_MODULE_START_FAULTS:",
        "emitted only when a module_start faulted, so a baseline could not hold it Absent; the module list is the finding",
    ),
    (
        "BENCH_SYSTEM_IPC_WITNESS_AT_MODULE_START:",
        "per-module cumulative snapshot, repeated once per module_start; the boot-end totals are tracked on BENCH_SYSTEM_IPC_WITNESS",
    ),
    (
        "BENCH_SYSTEM_IPC_KEYS_AT_MODULE_START:",
        "per-module snapshot of the key inventory, repeated once per module_start",
    ),
    (
        "BENCH_CELLSYSUTIL_SEED_WITNESS:",
        "per-module stall signature read by the cellSysutil tripwire, repeated once per seeded module_start",
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
/// neither tracks nor admits as extra, and a recognized line emitted
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
        let Some((prefix, fields, extra)) = LINE_TABLE
            .iter()
            .find(|(p, _, _)| line.starts_with(p))
            .map(|(p, f, e)| (*p, *f, *e))
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
                if !extra.admits(key) {
                    errors.push(WitnessParseError {
                        line_prefix: prefix.to_string(),
                        token: token.to_string(),
                        reason: format!(
                            "key {key:?} is neither tracked nor admitted as extra; \
                             if the emitter grew a field, add it to the line table"
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

/// `BENCH_*` prefixes a witness is recorded from.
pub fn tracked_line_prefixes() -> Vec<&'static str> {
    LINE_TABLE.iter().map(|(p, _, _)| *p).collect()
}

/// `BENCH_*` prefixes the boot path emits that stay diagnostic-only,
/// with the stated reason for each.
pub fn diagnostic_lines() -> &'static [(&'static str, &'static str)] {
    DIAGNOSTIC_LINES
}
