//! Boots whitelisted PPU executables from `tests/ps3autotests/` via
//! `cellgov boot run` and compares captured TTY against the
//! real-PS3 `.expected` file.
//!
//! Compiled only under the `ps3autotests` feature: the input is a
//! GPLv2 tree cloned by the developer (see tests/ps3_autotests.README.md)
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
    reason = "integration test harness: stderr carries the concurrency gate and verdict diagnostics"
)]
#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Condvar, Mutex, OnceLock};

use cellgov_compare::Observation;

#[path = "common/installed_firmware.rs"]
mod installed_firmware;

#[path = "common/ps3autotests_cases.rs"]
mod cases;
use cases::{
    manifest_content, report_verdict, Case, CASES, CPU_BASIC, CPU_PPU_BRANCH, CPU_SPU_ALU,
    CPU_SPU_FPU, CPU_SPU_GENERIC, LV2_SYS_EVENT_FLAG, LV2_SYS_PROCESS, LV2_SYS_SEMAPHORE,
    RSX_METHODS_PFIFO_PULLER, RSX_METHODS_SCE_DRIVER,
};

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
         (see tests/ps3_autotests.README.md)",
        dir.display()
    );
    dir
}

/// The case's manifest, naming the firmware the suite boots against.
fn write_manifest(path: &Path, case: &Case) {
    std::fs::write(
        path,
        manifest_content(case, installed_firmware::TEST_SYSTEM_VERSION),
    )
    .expect("write manifest");
}

/// `run_id` discriminates concurrent or sequential re-runs of one
/// case so they cannot race on the scratch dir's `observation.json`.
fn run_observation(case: &Case, run_id: &str) -> Observation {
    let autotests = ps3autotests_root();
    let test_dir = autotests.join("tests").join(case.rel_dir);
    let elf_path = test_dir.join(case.executable);
    let expected_path = test_dir.join(case.expected);
    assert!(
        elf_path.is_file(),
        "ps3autotests {}/{}: executable missing at {elf_path:?}",
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
    // A zero or absent count makes every cross-run equality downstream
    // compare two empty runs. `assert!` rather than `debug_assert!`: the
    // CI gate runs the suite under `--release` too, where a debug
    // assertion compiles out.
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
    hold_against_console(case, &run_observation(case, "r0"));
}

/// Compare one run of `case` against the console's `.expected`.
fn hold_against_console(case: &Case, observation: &Observation) {
    let autotests = ps3autotests_root();
    let expected_path = autotests
        .join("tests")
        .join(case.rel_dir)
        .join(case.expected);
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
    report_verdict(case, observation, &expected);
}

/// An `#[ignore]`d boot test below reads nothing in an ordinary run, so
/// this gate is what holds its fixtures. Without it, a case could lose its
/// executable or `.expected` and nothing would say so. The feature
/// declares the tree present, so a missing or empty file is a hard
/// error here.
#[test]
fn every_declared_case_names_a_present_non_empty_fixture_pair() {
    assert!(!CASES.is_empty(), "the case table is empty");
    let root = ps3autotests_root().join("tests");
    for case in CASES {
        let dir = root.join(case.rel_dir);
        for file in [dir.join(case.executable), dir.join(case.expected)] {
            let len = std::fs::metadata(&file)
                .unwrap_or_else(|e| {
                    panic!(
                        "ps3autotests {}/{}: {} ({e}); the feature declares the suite \
                         present -- clone \
                         https://github.com/AerialX/ps3autotests.git into {} \
                         (see tests/ps3_autotests.README.md)",
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

/// Two boots of `cpu/basic` must agree whatever trajectory the ELF
/// takes, so the test compares them before it holds the first against
/// the console. One test does both, which keeps the suite at two boots
/// of this ELF.
#[test]
fn cpu_basic() {
    let first = run_observation(&CPU_BASIC, "r0");
    let second = run_observation(&CPU_BASIC, "r1");
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
    hold_against_console(&CPU_BASIC, &first);
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
fn lv2_sys_process() {
    run_case(&LV2_SYS_PROCESS);
}

#[test]
fn lv2_sys_semaphore() {
    run_case(&LV2_SYS_SEMAPHORE);
}

#[test]
#[ignore = "sys_spu_image_open answers ENOENT for /app_home/spu_alu.spu.elf, \
            because it sees only two fixed sibling names, so the run exits \
            0x80010006 at step 47 with no TTY (#2112)"]
fn cpu_spu_alu() {
    run_case(&CPU_SPU_ALU);
}

#[test]
#[ignore = "sys_spu_image_open answers ENOENT for /app_home/spu_fpu.spu.elf, \
            because it sees only two fixed sibling names, so the run exits \
            0x80010006 at step 47 with no TTY (#2112)"]
fn cpu_spu_fpu() {
    run_case(&CPU_SPU_FPU);
}

#[test]
#[ignore = "the PPU's first store to a Raw SPU's MFC proxy command registers \
            (offset 0x3004) faults the run, which then reaches MAX_STEPS (#2113)"]
fn cpu_spu_generic() {
    run_case(&CPU_SPU_GENERIC);
}

#[test]
#[ignore = "the suite ships this case only as a fake-signed SELF, which boot \
            run refuses at load (#2114)"]
fn rsx_methods_pfifo_puller() {
    run_case(&RSX_METHODS_PFIFO_PULLER);
}

#[test]
#[ignore = "the suite ships this case only as a fake-signed SELF, which boot \
            run refuses at load (#2114)"]
fn rsx_methods_sce_driver() {
    run_case(&RSX_METHODS_SCE_DRIVER);
}
