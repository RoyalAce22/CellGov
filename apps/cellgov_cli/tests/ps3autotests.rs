//! Boot whitelisted `.ppu.elf` files from `tools/ps3autotests/` via
//! the `cellgov_cli run-game` binary and compare captured TTY
//! against the real-PS3 `.expected` file. When the (gitignored)
//! `tools/ps3autotests/` directory is absent, each test logs a
//! skip note and returns clean so CI without the fixture stays
//! green.

use std::path::{Path, PathBuf};
use std::process::Command;

use cellgov_compare::Observation;

/// Whitelist entry. `rel_dir` is relative to ps3autotests' `tests/`
/// root. `stem` is both the ELF file stem (`<stem>.ppu.elf`) and the
/// expected-output stem (`<stem>.expected`).
struct Case {
    rel_dir: &'static str,
    stem: &'static str,
    max_steps: usize,
}

const PS3AUTOTESTS_RELPATH: &str = "tools/ps3autotests";

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is `apps/cellgov_cli`; workspace root is two up.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn ps3autotests_root() -> Option<PathBuf> {
    let dir = workspace_root().join(PS3AUTOTESTS_RELPATH);
    dir.is_dir().then_some(dir)
}

fn run_case(case: &Case) {
    let Some(autotests) = ps3autotests_root() else {
        eprintln!(
            "ps3autotests: tools/ps3autotests/ not present; skipping {}/{}",
            case.rel_dir, case.stem,
        );
        return;
    };

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

    let expected = std::fs::read(&expected_path).expect("read .expected");

    // Per-case scratch dir so parallel tests do not collide on
    // manifest or observation.json paths.
    let scratch = workspace_root()
        .join("target")
        .join("ps3autotests_scratch")
        .join(case.rel_dir.replace('/', "_"))
        .join(case.stem);
    std::fs::create_dir_all(&scratch).expect("create scratch");

    let manifest_path = scratch.join("manifest.toml");
    write_manifest(&manifest_path, case);

    let observation_path = scratch.join("observation.json");
    if observation_path.exists() {
        std::fs::remove_file(&observation_path).ok();
    }

    let cli_bin = env!("CARGO_BIN_EXE_cellgov_cli");
    let output = Command::new(cli_bin)
        .arg("run-game")
        .arg("--title-manifest")
        .arg(&manifest_path)
        .arg("--max-steps")
        .arg(case.max_steps.to_string())
        .arg("--save-observation")
        .arg(&observation_path)
        .arg(&elf_path)
        .current_dir(workspace_root())
        .output()
        .expect("spawn cellgov_cli run-game");

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

    report_verdict(case, &observation, &expected);
}

fn write_manifest(path: &Path, case: &Case) {
    let content = format!(
        r#"[title]
content_id = "AT_{stem_upper}"
short_name = "at_{stem}"
display_name = "ps3autotests {rel_dir}/{stem}"
eboot_candidates = ["{stem}.ppu.elf"]

[checkpoint]
kind = "process-exit"
"#,
        stem_upper = case.stem.to_uppercase(),
        stem = case.stem,
        rel_dir = case.rel_dir,
    );
    std::fs::write(path, content).expect("write manifest");
}

fn report_verdict(case: &Case, observation: &Observation, expected: &[u8]) {
    let captured = observation.tty_log.as_slice();
    let label = format!("{}/{}", case.rel_dir, case.stem);

    if captured == expected {
        eprintln!(
            "ps3autotests {label}: MATCH ({} bytes, outcome={:?})",
            captured.len(),
            observation.outcome
        );
        return;
    }

    eprintln!("ps3autotests {label}: DIVERGE");
    eprintln!("  outcome: {:?}", observation.outcome);
    eprintln!("  expected: {} bytes", expected.len(),);
    eprintln!("  captured: {} bytes", captured.len());
    eprintln!("  expected preview: {:?}", preview(expected, 200));
    eprintln!("  captured preview: {:?}", preview(captured, 200));
    eprintln!(
        "  first differing offset: {}",
        first_diff_offset(captured, expected)
    );
    panic!("ps3autotests {label}: TTY divergence vs real-PS3 .expected");
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
fn cpu_basic() {
    run_case(&Case {
        rel_dir: "cpu/basic",
        stem: "basic",
        max_steps: 200_000,
    });
}

#[test]
fn cpu_ppu_branch() {
    run_case(&Case {
        rel_dir: "cpu/ppu_branch",
        stem: "ppu_branch",
        max_steps: 50_000_000,
    });
}

#[test]
fn lv2_sys_event_flag() {
    run_case(&Case {
        rel_dir: "lv2/sys_event_flag",
        stem: "sys_event_flag",
        max_steps: 10_000_000,
    });
}

#[test]
fn lv2_sys_process() {
    run_case(&Case {
        rel_dir: "lv2/sys_process",
        stem: "sys_process",
        max_steps: 10_000_000,
    });
}

#[test]
fn lv2_sys_semaphore() {
    run_case(&Case {
        rel_dir: "lv2/sys_semaphore",
        stem: "sys_semaphore",
        max_steps: 10_000_000,
    });
}
