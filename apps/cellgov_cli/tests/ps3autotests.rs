//! Boots whitelisted `.ppu.elf` files from `tests/ps3autotests/` via
//! `cellgov_cli run-game` and compares captured TTY against the
//! real-PS3 `.expected` file.
//!
//! Skips silently when the (gitignored) corpus is absent;
//! `CELLGOV_REQUIRE_AUTOTESTS=1` promotes that to a hard failure.
//!
//! Cross-module contract: assumes `sys_tty_write` HLE captures
//! byte-identical output to a real PS3 TTY. A capture-side
//! truncation cannot be detected from inside this harness.

#![allow(
    clippy::print_stderr,
    reason = "integration test harness: stderr carries diagnostic output for skipped corpora and verdict mismatches"
)]
#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Condvar, Mutex, OnceLock};

use cellgov_compare::{Observation, ObservedOutcome};

/// Peak RSS budget per subprocess: ~1.8 GiB guest memory plus a
/// transient JSON-array dump from `--save-observation`. 4 GiB covers
/// both with margin.
const PER_SLOT_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Env override for the concurrency limit (floor 1).
const OVERRIDE_ENV: &str = "CELLGOV_PS3AUTOTESTS_MAX_CONCURRENT";

struct Semaphore {
    available: Mutex<usize>,
    cv: Condvar,
}

impl Semaphore {
    fn new(n: usize) -> Self {
        Self {
            available: Mutex::new(n),
            cv: Condvar::new(),
        }
    }

    fn acquire(&self) -> Permit<'_> {
        let mut g = self.available.lock().expect("semaphore mutex poisoned");
        while *g == 0 {
            g = self.cv.wait(g).expect("semaphore condvar wait failed");
        }
        *g -= 1;
        Permit { sem: self }
    }
}

struct Permit<'a> {
    sem: &'a Semaphore,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut g = self.sem.available.lock().expect("semaphore mutex poisoned");
        *g += 1;
        self.sem.cv.notify_one();
    }
}

/// Acquire a slot for spawning `cellgov_cli run-game`. Without this
/// gate, `nproc * peak-RSS` can exceed host RAM and OOM the suite.
fn subprocess_permit() -> Permit<'static> {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    let sem = SEM.get_or_init(|| {
        let limit = compute_limit();
        let slot_gib = PER_SLOT_BYTES as f64 / (1024.0 * 1024.0 * 1024.0);
        eprintln!(
            "ps3autotests: gating subprocesses at {limit} concurrent \
             (per-slot budget {slot_gib:.1} GiB; override via {OVERRIDE_ENV})"
        );
        Semaphore::new(limit)
    });
    sem.acquire()
}

fn compute_limit() -> usize {
    if let Ok(s) = std::env::var(OVERRIDE_ENV) {
        return s
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|&n| n >= 1)
            .unwrap_or(1);
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    // `available_memory` reflects what the OS will hand out;
    // `total_memory` ignores RAM held by other processes.
    let ram = sys.available_memory();
    ((ram / PER_SLOT_BYTES) as usize).max(1)
}

/// `rel_dir` is relative to ps3autotests' `tests/` root; `stem` is
/// shared between `<stem>.ppu.elf` and `<stem>.expected`.
struct Case {
    rel_dir: &'static str,
    stem: &'static str,
    /// Scheduler-step cap (not retired instructions; default budget
    /// is 256 instructions/step). Tighten when the case's
    /// `expected_steps` is far below the cap.
    max_steps: usize,
    /// Reference step count. Drift outside +/-25% emits a WARN line.
    /// `None` waives the check.
    expected_steps: Option<usize>,
}

const PS3AUTOTESTS_RELPATH: &str = "tests/ps3autotests";

/// Walk up from `CARGO_MANIFEST_DIR` to the `[workspace]` Cargo.toml.
fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let cargo_toml = p.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&cargo_toml) {
            if text.contains("[workspace]") {
                return p;
            }
        }
        if !p.pop() {
            panic!(
                "could not find workspace root walking up from CARGO_MANIFEST_DIR ({})",
                env!("CARGO_MANIFEST_DIR")
            );
        }
    }
}

fn ps3autotests_root() -> Option<PathBuf> {
    let dir = workspace_root().join(PS3AUTOTESTS_RELPATH);
    dir.is_dir().then_some(dir)
}

/// # Cross-module contract
///
/// `cellgov_cli run-game` resolves the ELF
/// from argv, not from the manifest's `eboot_candidates`. ps3autotests
/// ELFs do not live in a PS3 VFS layout; the manifest carries the
/// candidate purely so the schema validates.
fn write_manifest(path: &Path, case: &Case) {
    let content = format!(
        r#"[title]
content_id = "AT_{stem_upper}"
short_name = "at_{stem}"
display_name = "ps3autotests {rel_dir}/{stem}"
eboot_candidates = ["{stem}.ppu.elf"]
year = 2007
developer = "ps3autotests"
engine = "ps3autotests"
distribution = "psn-hdd"

[checkpoint]
kind = "process-exit"
"#,
        stem_upper = case.stem.to_uppercase(),
        stem = case.stem,
        rel_dir = case.rel_dir,
    );
    std::fs::write(path, content).expect("write manifest");
}

/// `run_id` discriminates concurrent or sequential re-runs of one
/// case so they cannot race on the scratch dir's `observation.json`.
fn run_observation(case: &Case, run_id: &str) -> Option<Observation> {
    let autotests = ps3autotests_root()?;
    let test_dir = autotests.join("tests").join(case.rel_dir);
    let elf_path = test_dir.join(format!("{}.ppu.elf", case.stem));
    let expected_path = test_dir.join(format!("{}.expected", case.stem));
    assert!(
        elf_path.is_file(),
        "ps3autotests {}/{}: ELF missing at {elf_path:?}",
        case.rel_dir,
        case.stem
    );
    assert!(
        expected_path.is_file(),
        "ps3autotests {}/{}: .expected missing at {expected_path:?}",
        case.rel_dir,
        case.stem
    );

    let scratch = workspace_root()
        .join("target")
        .join("ps3autotests_scratch")
        .join(case.rel_dir.replace('/', "_"))
        .join(case.stem)
        .join(run_id);
    std::fs::create_dir_all(&scratch).expect("create scratch");

    let manifest_path = scratch.join("manifest.toml");
    write_manifest(&manifest_path, case);

    let observation_path = scratch.join("observation.json");
    if observation_path.exists() {
        std::fs::remove_file(&observation_path).ok();
    }

    let cli_bin = env!("CARGO_BIN_EXE_cellgov_cli");
    let output = {
        let _permit = subprocess_permit();
        Command::new(cli_bin)
            .arg("run-game")
            .arg("--title-manifest")
            .arg(&manifest_path)
            .arg("--max-steps")
            .arg(case.max_steps.to_string())
            .arg("--save-observation")
            .arg(&observation_path)
            .arg(&elf_path)
            .current_dir(workspace_root())
            // Synthetic test ELFs do not coexist with a real LV2 PRX
            // boot; suppress the firmware/sys/external auto-default.
            .env("CELLGOV_NO_FIRMWARE_DIR", "1")
            .output()
            .expect("spawn cellgov_cli run-game")
    };

    if !output.status.success() {
        eprintln!(
            "ps3autotests {}/{}: cellgov_cli run-game exited non-zero",
            case.rel_dir, case.stem
        );
        eprintln!("--- stdout ---");
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
        eprintln!("--- stderr ---");
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        panic!("cellgov_cli run-game failed");
    }

    let observation: Observation = {
        let json = std::fs::read_to_string(&observation_path).expect("read observation.json");
        serde_json::from_str(&json).expect("deserialize Observation")
    };
    // A `None` here silently no-ops the drift-band check below; catch
    // a future runner change that omits the field rather than letting
    // step regressions slip through unnoticed.
    debug_assert!(
        observation.metadata.steps.is_some(),
        "ps3autotests {}/{}: observation.metadata.steps was None",
        case.rel_dir,
        case.stem
    );
    Some(observation)
}

fn run_case(case: &Case) {
    let Some(observation) = run_observation(case, "r0") else {
        if std::env::var_os("CELLGOV_REQUIRE_AUTOTESTS").is_some() {
            panic!(
                "ps3autotests: CELLGOV_REQUIRE_AUTOTESTS is set but \
                 {PS3AUTOTESTS_RELPATH}/ is missing or empty -- clone \
                 https://github.com/AerialX/ps3autotests.git into that \
                 path (see tests/ps3autotests.README.md)"
            );
        }
        eprintln!(
            "ps3autotests: skipping {}/{} ({PS3AUTOTESTS_RELPATH}/ not present)",
            case.rel_dir, case.stem,
        );
        return;
    };

    let autotests = ps3autotests_root().expect("checked in run_observation");
    let expected_path = autotests
        .join("tests")
        .join(case.rel_dir)
        .join(format!("{}.expected", case.stem));
    let expected = std::fs::read(&expected_path).expect("read .expected");
    report_verdict(case, &observation, &expected);
}

/// Outcome must be checked before TTY: a `Timeout` produces a
/// truncated `tty_log` whose prefix may coincidentally match the
/// `.expected` head, so a naive byte compare passes silently.
fn report_verdict(case: &Case, observation: &Observation, expected: &[u8]) {
    let label = format!("{}/{}", case.rel_dir, case.stem);

    match observation.outcome {
        ObservedOutcome::Completed => {}
        ObservedOutcome::Timeout => panic!(
            "ps3autotests {label}: outcome=Timeout (max_steps={} reached). \
             Either the test wedged in an infinite loop or the cap is too \
             low. Investigate via `cellgov_cli run-game --max-steps N` \
             before raising the cap.",
            case.max_steps
        ),
        ObservedOutcome::Fault => panic!(
            "ps3autotests {label}: outcome=Fault. The runtime took an \
             architectural fault before reaching sys_process_exit. Run \
             `cellgov_cli run-game` on the ELF to inspect."
        ),
        ObservedOutcome::Stalled => panic!(
            "ps3autotests {label}: outcome=Stalled. No runnable units but \
             pending events remain -- a deadlock or missed wake-up."
        ),
    }

    let captured = observation.tty_log.as_slice();
    let observed_steps = observation.metadata.steps;

    if let (Some(expected), Some(actual)) = (case.expected_steps, observed_steps) {
        // Scheduling tweaks routinely shift step counts a few percent;
        // a >25% move is a real regression, not noise.
        let lower = expected * 3 / 4;
        let upper = expected * 5 / 4;
        if !(lower..=upper).contains(&actual) {
            eprintln!(
                "ps3autotests {label}: WARN step count drift: expected ~{}, got {} \
                 (band: [{}, {}])",
                expected, actual, lower, upper
            );
        }
    }

    if captured == expected {
        eprintln!(
            "ps3autotests {label}: MATCH ({} bytes, outcome={:?}, steps={:?})",
            captured.len(),
            observation.outcome,
            observed_steps,
        );
        return;
    }

    eprintln!("ps3autotests {label}: DIVERGE");
    eprintln!("  outcome: {:?}", observation.outcome);
    eprintln!("  expected: {} bytes", expected.len());
    eprintln!("  captured: {} bytes", captured.len());
    eprintln!("  expected preview: {:?}", preview(expected, 200));
    eprintln!("  captured preview: {:?}", preview(captured, 200));
    eprintln!(
        "  first differing offset: {}",
        first_diff_offset(captured, expected)
    );
    let cap_cr = count_byte(captured, b'\r');
    let exp_cr = count_byte(expected, b'\r');
    if cap_cr != exp_cr {
        eprintln!(
            "  NOTE: \\r count differs (captured={}, expected={}) -- the \
             .expected file may have been autocrlf-mangled on Windows. \
             See tests/ps3autotests.README.md.",
            cap_cr, exp_cr,
        );
    }
    panic!("ps3autotests {label}: TTY divergence vs real-PS3 .expected");
}

fn count_byte(bytes: &[u8], target: u8) -> usize {
    bytes.iter().filter(|&&b| b == target).count()
}

fn preview(bytes: &[u8], cap: usize) -> String {
    let n = bytes.len().min(cap);
    String::from_utf8_lossy(&bytes[..n]).into_owned()
}

fn first_diff_offset(a: &[u8], b: &[u8]) -> String {
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y {
            return format!("offset {i}: 0x{x:02x} vs 0x{y:02x}");
        }
    }
    if a.len() != b.len() {
        format!(
            "offset {}: length differs ({} vs {})",
            a.len().min(b.len()),
            a.len(),
            b.len(),
        )
    } else {
        "no diff".to_string()
    }
}

#[test]
#[ignore = "Default single-PRX boot path needs sysPrxForUser/sys_fs NIDs that the \
            userspace HLE used to provide; the firmware-set boot mode covers them via \
            the PUP-installed PRXes. Un-ignore once the harness either switches to \
            `--boot-mode firmware-set` or the missing NIDs route to direct LV2 syscalls."]
fn cpu_basic() {
    run_case(&Case {
        rel_dir: "cpu/basic",
        stem: "basic",
        max_steps: 200_000,
        expected_steps: Some(83),
    });
}

#[test]
#[ignore = "Default single-PRX boot path needs sysPrxForUser/sys_fs NIDs that the \
            userspace HLE used to provide; the firmware-set boot mode covers them via \
            the PUP-installed PRXes. Un-ignore once the harness either switches to \
            `--boot-mode firmware-set` or the missing NIDs route to direct LV2 syscalls."]
fn cpu_ppu_branch() {
    run_case(&Case {
        rel_dir: "cpu/ppu_branch",
        stem: "ppu_branch",
        max_steps: 50_000_000,
        expected_steps: Some(52_622),
    });
}

#[test]
#[ignore = "Default single-PRX boot path needs sysPrxForUser/sys_fs NIDs that the \
            userspace HLE used to provide; the firmware-set boot mode covers them via \
            the PUP-installed PRXes. Un-ignore once the harness either switches to \
            `--boot-mode firmware-set` or the missing NIDs route to direct LV2 syscalls."]
fn lv2_sys_event_flag() {
    run_case(&Case {
        rel_dir: "lv2/sys_event_flag",
        stem: "sys_event_flag",
        max_steps: 10_000_000,
        expected_steps: Some(1_494),
    });
}

#[test]
#[ignore = "Default single-PRX boot path needs sysPrxForUser/sys_fs NIDs that the \
            userspace HLE used to provide; the firmware-set boot mode covers them via \
            the PUP-installed PRXes. Un-ignore once the harness either switches to \
            `--boot-mode firmware-set` or the missing NIDs route to direct LV2 syscalls."]
fn lv2_sys_process() {
    run_case(&Case {
        rel_dir: "lv2/sys_process",
        stem: "sys_process",
        max_steps: 10_000_000,
        expected_steps: Some(3_686),
    });
}

#[test]
#[ignore = "Default single-PRX boot path needs sysPrxForUser/sys_fs NIDs that the \
            userspace HLE used to provide; the firmware-set boot mode covers them via \
            the PUP-installed PRXes. Un-ignore once the harness either switches to \
            `--boot-mode firmware-set` or the missing NIDs route to direct LV2 syscalls."]
fn lv2_sys_semaphore() {
    run_case(&Case {
        rel_dir: "lv2/sys_semaphore",
        stem: "sys_semaphore",
        max_steps: 10_000_000,
        expected_steps: Some(1_167),
    });
}

/// Determinism canary across two reruns of the same scenario.
#[test]
#[ignore = "Default single-PRX boot path needs sysPrxForUser/sys_fs NIDs that the \
            userspace HLE used to provide; the firmware-set boot mode covers them via \
            the PUP-installed PRXes. Un-ignore once the harness either switches to \
            `--boot-mode firmware-set` or the missing NIDs route to direct LV2 syscalls."]
fn determinism_double_run_cpu_basic() {
    let case = Case {
        rel_dir: "cpu/basic",
        stem: "basic",
        max_steps: 200_000,
        expected_steps: None,
    };
    let Some(first) = run_observation(&case, "determinism_a") else {
        if std::env::var_os("CELLGOV_REQUIRE_AUTOTESTS").is_some() {
            panic!(
                "ps3autotests: CELLGOV_REQUIRE_AUTOTESTS set but corpus \
                 missing -- cannot run determinism check"
            );
        }
        eprintln!("ps3autotests: skipping determinism_double_run_cpu_basic");
        return;
    };
    let second =
        run_observation(&case, "determinism_b").expect("second run must produce an observation");
    assert_eq!(
        first.outcome, second.outcome,
        "determinism: outcome differs between runs"
    );
    assert_eq!(
        first.metadata.steps, second.metadata.steps,
        "determinism: step count differs between runs"
    );
    assert_eq!(
        first.tty_log, second.tty_log,
        "determinism: tty_log differs between runs"
    );
    assert_eq!(
        first.memory_regions, second.memory_regions,
        "determinism: memory regions differ between runs"
    );
    assert_eq!(
        first, second,
        "determinism: full observation differs between runs"
    );
}
