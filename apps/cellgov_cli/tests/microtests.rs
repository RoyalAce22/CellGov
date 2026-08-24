//! Boots every run-game-bootable microtest under `tests/micro/` and
//! checks the `CGOV` payload it reports against the values its design
//! fixes.
//!
//! Compiled only under the `microtests` feature: the ELFs are
//! gitignored build output, so opting in declares the corpus built and
//! a missing artifact is a hard error rather than a skip. Build each
//! with `tests/micro/<name>/build.sh` in a ps3dev+PSL1GHT toolchain
//! image -- requirements are in each script's header.
//!
//! Every expectation below is derived from the microtest's own
//! documented output layout (a sum is `N*(N-1)/2` for that test's
//! `MESSAGES`, a counter is `2 * INCREMENTS_PER_THREAD`), never copied
//! from a previous run's output. A table transcribed from observed
//! bytes would re-pass whatever the code currently does, which is the
//! failure this gate exists to catch.

#![allow(
    clippy::print_stderr,
    reason = "integration test harness: stderr carries per-case verdicts and diagnostics"
)]
#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use std::path::PathBuf;
use std::process::Command;

use cellgov_compare::{Observation, ObservedOutcome};

/// What a payload word must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Fixed by the test's design (a derived sum, a sentinel, a count).
    Exact(u32),
    /// Must have happened at least once, but the count is not fixed.
    NonZero,
    /// Varies with scheduler interleaving; asserting it would pin the
    /// scheduler rather than the behaviour under test.
    Any,
}

struct Case {
    name: &'static str,
    /// Scheduler-step cap. Generous: the outcome assertion catches a
    /// wedge, so a tight cap would only convert a real hang into a
    /// confusing `Timeout`.
    max_steps: usize,
    /// One entry per `u32` in the CGOV payload, in wire order. The
    /// length also pins the payload size, so a struct that grows or
    /// shrinks a field fails here rather than being silently ignored.
    fields: &'static [(&'static str, Expect)],
}

use Expect::{Any, Exact, NonZero};

/// `MESSAGES` for both producer-consumer tests; the reported sum is
/// the triangular number `N*(N-1)/2`.
const PRODCONS_MESSAGES: u32 = 32;
const PRODCONS_SUM: u32 = PRODCONS_MESSAGES * (PRODCONS_MESSAGES - 1) / 2;

/// `MESSAGES` for the event-queue test.
const PUBSUB_MESSAGES: u32 = 16;
const PUBSUB_SUM: u32 = PUBSUB_MESSAGES * (PUBSUB_MESSAGES - 1) / 2;

/// `INCREMENTS_PER_THREAD` for the two PPU counter tests; two threads
/// each do this many, so a counter short of `2 * N` means a lost
/// update.
const PPU_INCREMENTS: u32 = 64;

/// `INCREMENTS_PER_THREAD` for the two-SPU atomic test.
const SPU_INCREMENTS: u32 = 32;

/// `FLIP_STATUS_DONE` -- the terminal value of the flip-status mirror.
const FLIP_STATUS_DONE: u32 = 0;

/// `sys_ppu_thread_exit` value both thread microtests hand to `join`.
const THREAD_EXIT_RETVAL: u32 = 0xCAFE_F00D;

const CASES: &[Case] = &[
    Case {
        name: "ppu_atomic_spinlock",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("counter", Exact(2 * PPU_INCREMENTS)),
            // Retry counts are 0 whenever the scheduler does not
            // interleave the two threads inside a lwarx/stwcx window.
            ("parent_retries", Any),
            ("child_retries", Any),
        ],
    },
    Case {
        name: "ppu_cond_prodcons",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("sum", Exact(PRODCONS_SUM)),
            ("producer_errs", Exact(0)),
            ("consumer_errs", Exact(0)),
        ],
    },
    Case {
        name: "ppu_event_flag_wakeall",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            // Both waiters wake on the same set call and must observe
            // the same pattern, bits 0 and 1.
            ("waker_a", Exact(0b0011)),
            ("waker_b", Exact(0b0011)),
            ("wake_cnt", Exact(2)),
        ],
    },
    Case {
        name: "ppu_event_queue_pubsub",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("sum", Exact(PUBSUB_SUM)),
            ("errors", Exact(0)),
            ("last_data1", Exact(PUBSUB_MESSAGES - 1)),
        ],
    },
    Case {
        name: "ppu_lwmutex_counter",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("counter", Exact(2 * PPU_INCREMENTS)),
            ("lock_errors", Exact(0)),
            ("unlock_errors", Exact(0)),
        ],
    },
    Case {
        name: "ppu_semaphore_prodcons",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("sum", Exact(PRODCONS_SUM)),
            ("producer_errs", Exact(0)),
            ("consumer_errs", Exact(0)),
        ],
    },
    Case {
        name: "ppu_two_threads_disjoint_writes",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("child", Exact(0xAAAA_AAAA)),
            ("parent", Exact(0xBBBB_BBBB)),
            ("join_retval", Exact(THREAD_EXIT_RETVAL)),
        ],
    },
    Case {
        name: "process_spawn_wait",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("pid", Exact(0x0100_0600)),
            ("spawn_rc", Exact(0)),
            // The child's exit must be seen by polling, not instantly.
            ("polls", NonZero),
        ],
    },
    Case {
        name: "rsx_flip_status_transition",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("waiting_iters", Any),
            ("done_iters", Any),
            ("last_status", Exact(FLIP_STATUS_DONE)),
        ],
    },
    Case {
        name: "rsx_label_write_poll",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("label_value", Exact(0xCAFE_BABE)),
            ("expected", Exact(0xCAFE_BABE)),
            ("spin_iters", Any),
        ],
    },
    Case {
        name: "rsx_semaphore_post",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            // The guest-ticks clock CellGov wrote; the value moves with
            // step accounting, but a zero means no report ever landed.
            ("report_value", NonZero),
            ("spin_iters", Any),
            ("padding", Exact(0)),
        ],
    },
    Case {
        name: "spu_atomic_cross_spu",
        max_steps: 1_000_000,
        fields: &[
            // Slot 0: SPU index 0's view.
            ("spu0_status", Exact(0)),
            ("spu0_counter_seen", Any),
            ("spu0_retries", Any),
            ("spu0_index", Exact(0)),
            // Slot 1: SPU index 1's view.
            ("spu1_status", Exact(0)),
            ("spu1_counter_seen", Any),
            ("spu1_retries", Any),
            ("spu1_index", Exact(1)),
            // Slot 2: the settled shared counter.
            ("final_pad0", Exact(0)),
            ("final_counter", Exact(2 * SPU_INCREMENTS)),
            ("final_pad2", Exact(0)),
            ("final_pad3", Exact(0)),
        ],
    },
];

/// Walk up from `CARGO_MANIFEST_DIR` to the `[workspace]` Cargo.toml.
fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if let Ok(text) = std::fs::read_to_string(p.join("Cargo.toml")) {
            if text.contains("[workspace]") {
                return p;
            }
        }
        assert!(
            p.pop(),
            "could not find workspace root walking up from {}",
            env!("CARGO_MANIFEST_DIR")
        );
    }
}

fn manifest_path(name: &str) -> PathBuf {
    workspace_root()
        .join("tests")
        .join("micro")
        .join(name)
        .join("manifest.toml")
}

/// Boot one microtest and return its observation.
///
/// `run_id` discriminates re-runs of one case so the determinism pass
/// cannot race the payload pass on the same scratch file.
fn run_observation(case: &Case, run_id: &str) -> Observation {
    let manifest = manifest_path(case.name);
    assert!(
        manifest.is_file(),
        "{}: manifest missing at {}",
        case.name,
        manifest.display()
    );

    let scratch = workspace_root()
        .join("target")
        .join("microtests_scratch")
        .join(std::process::id().to_string())
        .join(case.name)
        .join(run_id);
    std::fs::create_dir_all(&scratch).expect("create scratch");
    let observation_path = scratch.join("observation.json");
    std::fs::remove_file(&observation_path).ok();

    let output = Command::new(env!("CARGO_BIN_EXE_cellgov_cli"))
        .arg("run-game")
        .arg("--title-manifest")
        .arg(&manifest)
        .arg("--max-steps")
        .arg(case.max_steps.to_string())
        .arg("--save-observation")
        .arg(&observation_path)
        .current_dir(workspace_root())
        // These are freestanding PSL1GHT binaries that bind no firmware
        // namespace. Suppressing auto-discovery keeps the verdict the
        // same on a machine that happens to have firmware installed.
        .env("CELLGOV_NO_FIRMWARE_DIR", "1")
        .output()
        .expect("spawn cellgov_cli run-game");

    if !output.status.success() {
        eprintln!(
            "--- stdout ---\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        eprintln!(
            "--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        panic!("{}: cellgov_cli run-game exited non-zero", case.name);
    }

    let json = std::fs::read_to_string(&observation_path).unwrap_or_else(|e| {
        panic!(
            "{}: read {}: {e}\nthe microtests feature declares the corpus built; \
             build it with tests/micro/{}/build.sh",
            case.name,
            observation_path.display(),
            case.name,
        )
    });
    serde_json::from_str(&json).expect("deserialize Observation")
}

/// The `u32` words of the guest's `CGOV` frame.
///
/// The frame is 4 magic bytes, a big-endian `u32` length, then that
/// many payload bytes. It is located by scanning rather than read at
/// offset 0: the RSX microtests emit a human-readable PASS/FAIL line
/// ahead of the struct.
fn cgov_words(case: &Case, tty: &[u8]) -> Vec<u32> {
    let start = tty
        .windows(4)
        .position(|w| w == b"CGOV")
        .unwrap_or_else(|| {
            panic!(
                "{}: no CGOV frame in {} bytes of TTY: {:?}",
                case.name,
                tty.len(),
                String::from_utf8_lossy(&tty[..tty.len().min(120)]),
            )
        });
    let len_at = start + 4;
    let body_at = len_at + 4;
    assert!(
        body_at <= tty.len(),
        "{}: CGOV frame truncated before its length field",
        case.name
    );
    let len = u32::from_be_bytes(tty[len_at..body_at].try_into().unwrap()) as usize;
    assert!(
        body_at + len <= tty.len(),
        "{}: CGOV frame declares {len} payload bytes but only {} follow",
        case.name,
        tty.len() - body_at,
    );
    assert!(
        len.is_multiple_of(4),
        "{}: CGOV payload {len} bytes is not a whole number of u32 words",
        case.name
    );
    tty[body_at..body_at + len]
        .chunks_exact(4)
        .map(|c| u32::from_be_bytes(c.try_into().unwrap()))
        .collect()
}

/// Check one case's payload, returning a description of each mismatch.
fn check_payload(case: &Case, observation: &Observation) -> Vec<String> {
    let mut problems = Vec::new();
    let words = cgov_words(case, &observation.tty_log);
    if words.len() != case.fields.len() {
        problems.push(format!(
            "payload is {} words, table expects {} ({})",
            words.len(),
            case.fields.len(),
            case.fields
                .iter()
                .map(|(n, _)| *n)
                .collect::<Vec<_>>()
                .join(", "),
        ));
        return problems;
    }
    for (&word, &(field, expect)) in words.iter().zip(case.fields) {
        match expect {
            Exact(want) if word != want => {
                problems.push(format!("{field}: expected 0x{want:08x}, got 0x{word:08x}"))
            }
            NonZero if word == 0 => {
                problems.push(format!("{field}: expected non-zero, got 0"));
            }
            _ => {}
        }
    }
    problems
}

/// A run that did not reach `sys_process_exit` leaves a truncated or
/// absent payload, so the outcome has to settle before any field is
/// read.
fn check_outcome(case: &Case, observation: &Observation) -> Option<String> {
    match observation.outcome {
        ObservedOutcome::Completed | ObservedOutcome::ProcessExit => None,
        other => Some(format!(
            "outcome={other:?} (expected ProcessExit); steps={:?}, max_steps={}",
            observation.metadata.steps, case.max_steps,
        )),
    }
}

/// Does this manifest declare a title `run-game` can boot?
///
/// Mirrors `TitleManifest::load_from_text`'s layout acceptance: a
/// `title` table under `[cellgov]`, or -- when the file carries no
/// `cellgov` key at all -- one at the root. The decision is taken off
/// the parsed document rather than off line text: a scan for the
/// literal `[cellgov.title]` misses the root-level layout and a spaced
/// `[ cellgov.title ]` header alike, and `[cellgov]` alone is the
/// compare-harness scenario shape, not a title.
///
/// `cellgov_cli` has no library target, so an integration test cannot
/// link the loader and this restates its rule.
///
/// # Panics
///
/// On a manifest that does not parse: silently classifying it as
/// non-bootable is the same hole under a different cause.
fn declares_a_cellgov_title(path: &std::path::Path, text: &str) -> bool {
    let doc: toml::Value = text
        .parse()
        .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    match doc.get("cellgov") {
        Some(nested) => nested.get("title").is_some(),
        None => doc.get("title").is_some(),
    }
}

#[path = "microtests/microtests_tests.rs"]
mod tests;
