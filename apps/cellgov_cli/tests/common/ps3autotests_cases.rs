//! The ps3autotests case table, the boot manifest each case runs under,
//! and the verdict over a run's observation.
//!
//! None of it reads the gitignored suite, so the boot suite and its
//! data-free checks share it.
#![allow(
    dead_code,
    reason = "each integration-test binary compiles this module separately and uses a subset"
)]
#![allow(
    clippy::print_stderr,
    reason = "the verdict reports each run's match or divergence on stderr"
)]

use cellgov_compare::{Observation, ObservedOutcome};

/// `rel_dir` is relative to ps3autotests' `tests/` root; `stem` is
/// shared between `<stem>.ppu.elf` and `<stem>.expected`.
#[derive(Clone, Copy)]
pub struct Case {
    pub rel_dir: &'static str,
    pub stem: &'static str,
    /// Scheduler-step cap; the default budget retires up to 256
    /// instructions per step.
    pub max_steps: usize,
}

pub const CPU_BASIC: Case = Case {
    rel_dir: "cpu/basic",
    stem: "basic",
    max_steps: 200_000,
};

pub const CPU_PPU_BRANCH: Case = Case {
    rel_dir: "cpu/ppu_branch",
    stem: "ppu_branch",
    max_steps: 50_000_000,
};

pub const LV2_SYS_EVENT_FLAG: Case = Case {
    rel_dir: "lv2/sys_event_flag",
    stem: "sys_event_flag",
    max_steps: 10_000_000,
};

pub const LV2_SYS_PROCESS: Case = Case {
    rel_dir: "lv2/sys_process",
    stem: "sys_process",
    max_steps: 10_000_000,
};

pub const LV2_SYS_SEMAPHORE: Case = Case {
    rel_dir: "lv2/sys_semaphore",
    stem: "sys_semaphore",
    max_steps: 10_000_000,
};

/// Every case the boot suite names, including the `#[ignore]`d ones,
/// so its fixture gate walks the whole set.
pub const CASES: &[Case] = &[
    CPU_BASIC,
    CPU_PPU_BRANCH,
    LV2_SYS_EVENT_FLAG,
    LV2_SYS_PROCESS,
    LV2_SYS_SEMAPHORE,
];

/// The title manifest a case boots under, naming firmware
/// `system_ver`.
///
/// `cellgov boot run` resolves the ELF from argv rather than from the
/// manifest's `eboot_candidates`; ps3autotests ELFs do not live in a
/// PS3 VFS layout, so the manifest carries the candidate purely so the
/// schema validates.
pub fn manifest_content(case: &Case, system_ver: &str) -> String {
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
    )
}

/// Outcome must be checked before TTY: a `Timeout` produces a
/// truncated `tty_log` whose prefix may coincidentally match the
/// `.expected` head, so a naive byte compare passes silently.
pub fn report_verdict(case: &Case, observation: &Observation, expected: &[u8]) {
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
