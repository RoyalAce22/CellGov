//! Boots whitelisted `.ppu.elf` files from `tests/ps3autotests/` via
//! `cellgov boot run` and compares captured TTY against the
//! real-PS3 `.expected` file.
//!
//! Compiled only under the `ps3autotests` feature: the input is a
//! GPLv2 tree cloned by the developer (see tests/ps3autotests.README.md)
//! and gitignored, so opting in declares it present and its absence is
//! a hard error rather than a silent pass.
//!
//! The feature also activates `installed-firmware-tests`. These ELFs import
//! sysPrxForUser NIDs no HLE module binds, so `firmware_dir` resolves
//! the firmware version each generated manifest names. A store
//! without that version fails, and the refusal names it.
//!
//! Cross-module contract: assumes `sys_tty_write` HLE captures
//! byte-identical output to a real PS3 TTY. A capture-side
//! truncation cannot be detected from inside this harness.

#![allow(
    clippy::print_stderr,
    reason = "integration test harness: stderr carries the concurrency gate, step-drift warnings and verdict diagnostics"
)]
#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Condvar, Mutex, OnceLock};

use cellgov_compare::{Observation, ObservationMetadata, ObservedOutcome};

#[path = "common/installed_firmware.rs"]
mod installed_firmware;

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

/// Without this gate, `nproc * peak-RSS` can exceed host RAM and OOM
/// the suite.
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
        let n: usize = s
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("{OVERRIDE_ENV}={s:?}: not a non-negative integer ({e})"));
        // 0 clamps to the documented floor rather than deadlocking on a
        // semaphore no permit can ever be taken from.
        return n.max(1);
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
#[derive(Clone, Copy)]
struct Case {
    rel_dir: &'static str,
    stem: &'static str,
    /// Scheduler-step cap; the default budget retires up to 256
    /// instructions per step.
    max_steps: usize,
    /// Reference step count. Drift outside +/-25% emits a WARN line.
    /// `None` waives the check.
    expected_steps: Option<usize>,
}

// Reference counts come from firmware-set boots, so each is tied to
// whichever firmware `firmware_dir()` resolves. A stale count only
// warns in `report_verdict`; it never fails a run.
const CPU_BASIC: Case = Case {
    rel_dir: "cpu/basic",
    stem: "basic",
    max_steps: 200_000,
    expected_steps: Some(53),
};

const CPU_PPU_BRANCH: Case = Case {
    rel_dir: "cpu/ppu_branch",
    stem: "ppu_branch",
    max_steps: 50_000_000,
    expected_steps: Some(31_791),
};

const LV2_SYS_EVENT_FLAG: Case = Case {
    rel_dir: "lv2/sys_event_flag",
    stem: "sys_event_flag",
    max_steps: 10_000_000,
    expected_steps: Some(745),
};

const LV2_SYS_PROCESS: Case = Case {
    rel_dir: "lv2/sys_process",
    stem: "sys_process",
    max_steps: 10_000_000,
    expected_steps: Some(2_180),
};

const LV2_SYS_SEMAPHORE: Case = Case {
    rel_dir: "lv2/sys_semaphore",
    stem: "sys_semaphore",
    max_steps: 10_000_000,
    expected_steps: Some(930),
};

/// Every case the boot tests below name, including the `#[ignore]`d
/// ones, so the external-data gate walks the whole set.
const CASES: &[Case] = &[
    CPU_BASIC,
    CPU_PPU_BRANCH,
    LV2_SYS_EVENT_FLAG,
    LV2_SYS_PROCESS,
    LV2_SYS_SEMAPHORE,
];

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

/// Why `dir` cannot serve as the firmware set, or `None` when it can.
///
/// `--firmware-dir` only checks that its argument is a directory. An
/// empty one loads no PRX, and `load_firmware_set` then installs the
/// unresolved-import trampolines without complaint. A directory that
/// holds at least one `.sprx` separates the two cases.
fn firmware_set_reject_reason(dir: &Path) -> Option<String> {
    if !dir.is_dir() {
        return Some(format!("{}: absent", dir.display()));
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => return Some(format!("{}: unreadable ({e})", dir.display())),
    };
    let holds_a_module = entries.filter_map(Result::ok).any(|e| {
        e.path()
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("sprx"))
    });
    if holds_a_module {
        None
    } else {
        Some(format!("{}: holds no .sprx", dir.display()))
    }
}

/// The `sys/external` directory of the firmware these ELFs are held
/// against.
///
/// The harness passes this path to `boot run` explicitly, so a move in
/// that command's default cannot change which modules boot here.
///
/// # Panics
///
/// Panics when the store:
///
/// - holds no entry for that firmware version, or
/// - names a tree that holds no module.
fn firmware_dir() -> PathBuf {
    let dir = installed_firmware::firmware_external_dir();
    if let Some(reason) = firmware_set_reject_reason(&dir) {
        panic!(
            "ps3autotests: {reason} -- these ELFs import sysPrxForUser NIDs that only the \
             firmware PRX resolves, so install firmware with \
             `cellgov firmware install <PS3UPDAT.PUP>` before running this suite"
        );
    }
    dir
}

fn ps3autotests_root() -> PathBuf {
    let dir = workspace_root().join(PS3AUTOTESTS_RELPATH);
    assert!(
        dir.is_dir(),
        "ps3autotests: the feature declares the suite present but \
         {} is missing -- clone \
         https://github.com/AerialX/ps3autotests.git into that path \
         (see tests/ps3autotests.README.md)",
        dir.display()
    );
    dir
}

/// `cellgov boot run` resolves the ELF from argv rather than from
/// the manifest's `eboot_candidates`; ps3autotests ELFs do not live in
/// a PS3 VFS layout, so the manifest carries the candidate purely so
/// the schema validates.
fn write_manifest(path: &Path, case: &Case) {
    std::fs::write(path, manifest_content(case)).expect("write manifest");
}

fn manifest_content(case: &Case) -> String {
    format!(
        r#"[title]
content_id = "AT_{stem_upper}"
short_name = "at_{stem}"
display_name = "ps3autotests {rel_dir}/{stem}"
eboot_candidates = ["{stem}.ppu.elf"]
year = 2007
developer = "ps3autotests"
engine = "ps3autotests"
distribution = "psn-hdd"
system_ver = "{system_ver}"

[checkpoint]
kind = "process-exit"
"#,
        stem_upper = case.stem.to_uppercase(),
        stem = case.stem,
        rel_dir = case.rel_dir,
        system_ver = installed_firmware::TEST_SYSTEM_VERSION,
    )
}

/// `run_id` discriminates concurrent or sequential re-runs of one
/// case so they cannot race on the scratch dir's `observation.json`.
fn run_observation(case: &Case, run_id: &str) -> Observation {
    let autotests = ps3autotests_root();
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

    // The path carries the process id for the same reason every other
    // scratch dir in the workspace does: the debug and release passes
    // of the CI gate otherwise resolve to one directory and race on
    // observation.json.
    let scratch = workspace_root()
        .join("target")
        .join("ps3autotests_scratch")
        .join(std::process::id().to_string())
        .join(case.rel_dir.replace('/', "_"))
        .join(case.stem)
        .join(run_id);
    std::fs::create_dir_all(&scratch).expect("create scratch");

    let manifest_path = scratch.join("manifest.toml");
    write_manifest(&manifest_path, case);

    let observation_path = scratch.join("observation.json");
    // Presence of this file is what decides below whether the run
    // produced an outcome, so a survivor from an earlier run would be
    // read as if this run had written it.
    if observation_path.exists() {
        std::fs::remove_file(&observation_path).unwrap_or_else(|e| {
            panic!(
                "ps3autotests {}/{}: cannot clear the previous {} ({e})",
                case.rel_dir,
                case.stem,
                observation_path.display(),
            )
        });
    }

    let cli_bin = env!("CARGO_BIN_EXE_cellgov");
    let output = {
        let _permit = subprocess_permit();
        Command::new(cli_bin)
            .args(["boot", "run"])
            .arg("--title-manifest")
            .arg(&manifest_path)
            .arg("--max-steps")
            .arg(case.max_steps.to_string())
            .arg("--save-observation")
            .arg(&observation_path)
            .arg(&elf_path)
            .current_dir(workspace_root())
            // These ELFs import firmware namespaces: `cellgov dev
            // prx-imports` on cpu/basic lists 12 sysPrxForUser
            // NIDs. No HLE module binds them, so the boot must reach
            // the installed firmware for the real PRX to fill those
            // GOT slots.
            .arg("--firmware-dir")
            .arg(firmware_dir())
            .output()
            .expect("spawn cellgov boot run")
    };

    // `boot run` exits non-zero on Fault and on MaxSteps, and writes a
    // full observation for both. Those are outcomes for
    // [`report_verdict`] to classify, so the exit status decides
    // nothing here -- a run that left no readable observation is the
    // only harness failure.
    let json = std::fs::read_to_string(&observation_path).unwrap_or_else(|e| {
        dump_run_output(case, &output);
        panic!(
            "ps3autotests {}/{}: boot run {} and left no observation at {} ({e})",
            case.rel_dir,
            case.stem,
            describe_exit(&output.status),
            observation_path.display(),
        )
    });
    let observation: Observation = serde_json::from_str(&json).unwrap_or_else(|e| {
        dump_run_output(case, &output);
        panic!(
            "ps3autotests {}/{}: {} is not a deserializable Observation ({e})",
            case.rel_dir,
            case.stem,
            observation_path.display(),
        )
    });
    // A `None` here silently no-ops the drift-band check below, and a
    // zero makes every cross-run equality downstream compare two empty
    // runs. `assert!` rather than `debug_assert!`: the CI gate runs the
    // suite under `--release` too, where a debug assertion compiles out
    // and the drift band goes quiet.
    assert!(
        observation.metadata.steps.is_some_and(|s| s > 0),
        "ps3autotests {}/{}: observation.metadata.steps is {:?}; the boot \
         retired nothing, so every comparison against it is vacuous",
        case.rel_dir,
        case.stem,
        observation.metadata.steps,
    );
    observation
}

fn describe_exit(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exited {code}"),
        None => "was terminated by a signal".to_string(),
    }
}

/// Both streams of a `boot run` whose observation the harness could
/// not read; the failure itself is only visible in the child's output.
fn dump_run_output(case: &Case, output: &std::process::Output) {
    eprintln!(
        "ps3autotests {}/{}: cellgov boot run {}",
        case.rel_dir,
        case.stem,
        describe_exit(&output.status),
    );
    eprintln!("--- stdout ---");
    eprintln!("{}", String::from_utf8_lossy(&output.stdout));
    eprintln!("--- stderr ---");
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
}

fn run_case(case: &Case) {
    let observation = run_observation(case, "r0");
    let autotests = ps3autotests_root();
    let expected_path = autotests
        .join("tests")
        .join(case.rel_dir)
        .join(format!("{}.expected", case.stem));
    let expected = std::fs::read(&expected_path).expect("read .expected");
    // Anti-vacuity floor: `report_verdict` reports MATCH on a
    // byte-equal compare, and an empty `.expected` matches an empty
    // capture. A truncated or line-ending-mangled checkout must be a
    // failure, not a green run that compared nothing.
    assert!(
        !expected.is_empty(),
        "ps3autotests {}/{}: {} is empty; nothing to compare against",
        case.rel_dir,
        case.stem,
        expected_path.display(),
    );
    report_verdict(case, &observation, &expected);
}

/// Outcome must be checked before TTY: a `Timeout` produces a
/// truncated `tty_log` whose prefix may coincidentally match the
/// `.expected` head, so a naive byte compare passes silently.
fn report_verdict(case: &Case, observation: &Observation, expected: &[u8]) {
    let label = format!("{}/{}", case.rel_dir, case.stem);

    match observation.outcome {
        ObservedOutcome::Completed | ObservedOutcome::ProcessExit => {}
        ObservedOutcome::Timeout => panic!(
            "ps3autotests {label}: outcome=Timeout (max_steps={} reached). \
             Either the test wedged in an infinite loop or the cap is too \
             low. Investigate via `cellgov boot run --max-steps N` \
             before raising the cap.",
            case.max_steps
        ),
        ObservedOutcome::Fault => panic!(
            "ps3autotests {label}: outcome=Fault. The runtime took an \
             architectural fault before reaching sys_process_exit. Run \
             `cellgov boot run` on the ELF to inspect."
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

/// Three of the five boot tests below are `#[ignore]`, so this gate is
/// what holds their fixtures. Without it, a case could lose its ELF or
/// `.expected` and nothing would say so. The feature declares the tree
/// present, so a missing or empty file is a hard error here.
#[test]
fn every_declared_case_names_a_present_non_empty_fixture_pair() {
    assert!(!CASES.is_empty(), "the case table is empty");
    let root = ps3autotests_root().join("tests");
    for case in CASES {
        let dir = root.join(case.rel_dir);
        for file in [
            dir.join(format!("{}.ppu.elf", case.stem)),
            dir.join(format!("{}.expected", case.stem)),
        ] {
            let len = std::fs::metadata(&file)
                .unwrap_or_else(|e| {
                    panic!(
                        "ps3autotests {}/{}: {} ({e}); the feature declares the suite \
                         present -- clone \
                         https://github.com/AerialX/ps3autotests.git into {} \
                         (see tests/ps3autotests.README.md)",
                        case.rel_dir,
                        case.stem,
                        file.display(),
                        PS3AUTOTESTS_RELPATH,
                    )
                })
                .len();
            // `report_verdict` calls a byte-equal compare MATCH, and an
            // empty capture matches an empty `.expected`.
            assert!(
                len > 0,
                "ps3autotests {}/{}: {} is empty",
                case.rel_dir,
                case.stem,
                file.display(),
            );
        }
    }
}

#[test]
fn every_generated_boot_manifest_names_the_data_firmware_floor() {
    for case in CASES {
        let manifest = cellgov_boot::manifest::TitleManifest::load_from_text(
            &manifest_content(case),
            Path::new("ps3autotests-generated.toml"),
        )
        .unwrap_or_else(|error| {
            panic!(
                "ps3autotests {}/{} generated an invalid manifest: {error}",
                case.rel_dir, case.stem
            )
        });
        assert_eq!(
            manifest.system_ver.as_deref(),
            Some(installed_firmware::TEST_SYSTEM_VERSION)
        );
    }
}

/// A case named twice would boot the same ELF under two test names and
/// read as broader coverage than the table holds.
#[test]
fn the_case_table_names_each_fixture_once() {
    let mut seen: std::collections::BTreeSet<(&str, &str)> = std::collections::BTreeSet::new();
    for case in CASES {
        assert!(
            seen.insert((case.rel_dir, case.stem)),
            "duplicate case {}/{}",
            case.rel_dir,
            case.stem
        );
    }
}

/// [`report_verdict`] reads only the outcome, the step count, and
/// `tty_log`; every other field stays empty.
fn observation_with(outcome: ObservedOutcome, steps: usize) -> Observation {
    Observation {
        outcome,
        memory_regions: Vec::new(),
        events: Vec::new(),
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: "cellgov-boot".into(),
            steps: Some(steps),
        },
        tty_log: Vec::new(),
        identity: cellgov_compare::RunIdentity::default(),
        runner_firmware: None,
    }
}

#[test]
#[should_panic(expected = "outcome=Fault")]
fn a_faulted_run_is_named_as_a_fault_not_compared_against_expected() {
    report_verdict(
        &CPU_BASIC,
        &observation_with(ObservedOutcome::Fault, 40),
        b"expected output",
    );
}

#[test]
#[should_panic(expected = "outcome=Timeout")]
fn a_capped_run_is_named_as_a_timeout_not_compared_against_expected() {
    report_verdict(
        &CPU_PPU_BRANCH,
        &observation_with(ObservedOutcome::Timeout, 195_312),
        b"expected output",
    );
}

#[test]
#[should_panic(expected = "outcome=Stalled")]
fn a_stalled_run_is_named_as_a_stall_not_compared_against_expected() {
    report_verdict(
        &LV2_SYS_SEMAPHORE,
        &observation_with(ObservedOutcome::Stalled, 500),
        b"expected output",
    );
}

#[test]
fn a_clean_exit_whose_tty_matches_reports_a_match() {
    let mut observation = observation_with(ObservedOutcome::ProcessExit, 83);
    observation.tty_log = b"hello\n".to_vec();
    report_verdict(&CPU_BASIC, &observation, b"hello\n");
}

/// Negative control for the match above: without it, a byte compare
/// that always succeeded would leave every test here green.
#[test]
#[should_panic(expected = "TTY divergence")]
fn a_clean_exit_whose_tty_differs_is_a_divergence() {
    let mut observation = observation_with(ObservedOutcome::ProcessExit, 83);
    observation.tty_log = b"hello\n".to_vec();
    report_verdict(&CPU_BASIC, &observation, b"goodbye\n");
}

#[test]
fn cpu_basic() {
    run_case(&CPU_BASIC);
}

#[test]
#[ignore = "Known divergence: the run reaches PROCESS_EXIT(0) at step 31791 and \
            produces no TTY at all, against 41490 bytes on the console. The boot \
            reports syscall 988 unhandled -- a call eleven installed modules \
            issue, liblv2.sprx among them -- so the guest computes the branch \
            results and never reports them. Un-ignore when the capture is \
            non-empty."]
fn cpu_ppu_branch() {
    run_case(&CPU_PPU_BRANCH);
}

#[test]
#[ignore = "Known divergence: the run reproduces the console's error ladder \
            line for line, then deadlocks at the master/worker section. The \
            fixture creates the master and its five workers with \
            SYS_PPU_THREAD_CREATE_JOINABLE, and the boot reports \
            dispatch.ppu_thread_create_unmodeled_flags, so CellGov drops the \
            flag. The fixture joins only the master, never the workers, so \
            which dropped flag stalls the section is not yet pinned. Tracked \
            on the thread-create flag gap; un-ignore when the boot reaches \
            'Master: Exiting.'"]
fn lv2_sys_event_flag() {
    run_case(&LV2_SYS_EVENT_FLAG);
}

#[test]
#[ignore = "Known divergence, one cell wide: the SYS_LWCOND_OBJECT column reads \
            0 where the console reads 1. Syscall 111 has no dispatch handler, \
            so nothing increments the lwcond counter. Every other byte of the \
            capture matches. Tracked as the lwcond object-count gap; un-ignore \
            when the count moves."]
fn lv2_sys_process() {
    run_case(&LV2_SYS_PROCESS);
}

#[test]
fn lv2_sys_semaphore() {
    run_case(&LV2_SYS_SEMAPHORE);
}

/// The whole claim is that two boots agree, so this test holds
/// whatever trajectory the ELF takes, converging or not.
#[test]
fn two_boots_of_cpu_basic_produce_the_same_observation() {
    let case = Case {
        expected_steps: None,
        ..CPU_BASIC
    };
    let first = run_observation(&case, "determinism_a");
    let second = run_observation(&case, "determinism_b");
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
