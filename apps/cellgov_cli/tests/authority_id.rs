//! Titles boot under a retail-class program-authority-id, never the
//! system-class one.
//!
//! Retail SELFs (disc and NPDRM alike) all carry the same retail-app
//! authority id; what firmware gates on is the class. Serving a
//! system-class id makes libsysmodule's `module_start` skip init, and
//! its LoadModule sequence then fails on a never-created lwmutex.
//!
//! Titles come from the shared registry. Pending cells and
//! not-installed titles skip by name. At least one title must boot.

#![allow(
    clippy::print_stderr,
    reason = "integration test: named pending and not-installed skips are its only output channel"
)]

#[path = "common/registry.rs"]
mod registry;

use std::process::Command;

use cellgov_compare::witnesses::TITLE_NOT_INSTALLED_SENTINEL;
use cellgov_ps3_abi::format::sce::BDJ_SELF_PROGRAM_AUTHORITY_ID;
use registry::{titles, workspace_root, TitleUnderTest};

struct AuthorityWitness {
    program_authority_id: u64,
    authid_source: String,
    lwmutex_unknown_locks: u64,
}

fn parse_authority_witness(stderr: &str) -> Option<AuthorityWitness> {
    let rest = stderr
        .lines()
        .find_map(|l| l.strip_prefix("BENCH_AUTHORITY_ID_WITNESS:"))?;
    let mut authid = None;
    let mut source = None;
    let mut unknown = None;
    for tok in rest.split_whitespace() {
        if let Some(v) = tok.strip_prefix("program_authority_id=0x") {
            authid = u64::from_str_radix(v, 16).ok();
        } else if let Some(v) = tok.strip_prefix("authid_source=") {
            source = Some(v.to_string());
        } else if let Some(v) = tok.strip_prefix("lwmutex_unknown_locks=") {
            unknown = v.parse().ok();
        }
    }
    Some(AuthorityWitness {
        program_authority_id: authid?,
        authid_source: source?,
        lwmutex_unknown_locks: unknown?,
    })
}

/// Boot one title; `None` when its dump is not installed. Any other
/// failure panics: past the not-installed marker there is no
/// legitimate reason for this boot to break.
fn boot(title: &TitleUnderTest, force_system_authid: bool) -> Option<AuthorityWitness> {
    let cli_bin = env!("CARGO_BIN_EXE_cellgov");
    let mut cmd = Command::new(cli_bin);
    cmd.args(["boot", "bench-once"])
        .arg("--title")
        .arg(&title.short_name)
        .arg("--fw")
        .arg(&title.reference.fw)
        .arg("--max-steps")
        .arg(title.max_steps.to_string())
        .current_dir(workspace_root());
    // A firmware-shipped title has no game-version axis. The
    // composition refuses `--game-ver` for such a title.
    if let Some(v) = &title.reference.game_ver {
        cmd.arg("--game-ver").arg(v);
    }
    if force_system_authid {
        cmd.arg("--force-system-authid");
    }
    let output = cmd.output().expect("spawn cellgov boot bench-once");
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        if stderr.contains(TITLE_NOT_INSTALLED_SENTINEL) {
            return None;
        }
        let tail: Vec<&str> = stderr.lines().rev().take(20).collect();
        panic!(
            "{}: boot failed (exit {:?}). Last stderr lines (newest first):\n  {}",
            title.short_name,
            output.status.code(),
            tail.join("\n  ")
        );
    }
    Some(parse_authority_witness(&stderr).unwrap_or_else(|| {
        panic!(
            "{}: BENCH_AUTHORITY_ID_WITNESS line absent or unparseable",
            title.short_name
        )
    }))
}

fn check_titles(
    titles: &[TitleUnderTest],
    mut boot_title: impl FnMut(&TitleUnderTest, bool) -> Option<AuthorityWitness>,
) {
    let mut checked = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    let mut any_forced_delta = false;
    for title in titles {
        if let Some(why) = &title.reference.pending {
            eprintln!(
                "{} ({}): skipped -- declared pending ({why})",
                title.short_name,
                title.reference.label()
            );
            skipped.push(format!("{} {}", title.short_name, title.reference.label()));
            continue;
        }
        let Some(normal) = boot_title(title, false) else {
            eprintln!(
                "{} ({}): skipped -- not installed on this machine",
                title.short_name,
                title.reference.label()
            );
            skipped.push(format!("{} {}", title.short_name, title.reference.label()));
            continue;
        };
        checked += 1;
        assert_eq!(
            normal.authid_source, "self",
            "{}: the served authority id must come from the SELF \
             identification header, not the shared retail fallback -- \
             a fallback here means the SELF parse regressed",
            title.short_name,
        );
        assert_ne!(
            normal.program_authority_id, BDJ_SELF_PROGRAM_AUTHORITY_ID,
            "{}: normal boot served the system-class bdj.self id; \
             titles must boot under a retail-class authority id",
            title.short_name,
        );
        assert_eq!(
            normal.lwmutex_unknown_locks, 0,
            "{}: unknown-lwmutex lock failures under a retail-class authid",
            title.short_name,
        );
        let forced = boot_title(title, true).expect("installed above; forced boot must also start");
        assert_eq!(
            forced.program_authority_id, BDJ_SELF_PROGRAM_AUTHORITY_ID,
            "{}: --force-system-authid must serve the bdj.self constant",
            title.short_name,
        );
        if forced.lwmutex_unknown_locks > normal.lwmutex_unknown_locks {
            any_forced_delta = true;
        }
    }
    // Anti-vacuity floor, shared with title_witnesses: the feature
    // declares installed titles, so a run that booted nothing must not pass.
    assert!(
        checked > 0,
        "installed-title-tests is enabled but none of the {} registered title(s) is \
         installed (skipped: {})",
        titles.len(),
        skipped.join(", ")
    );
    // Negative control: the regression signature the `== 0` asserts
    // gate must still be reproducible somewhere in the installed set.
    assert!(
        any_forced_delta,
        "no installed title shows more unknown-lwmutex failures under the \
         forced system authid than under its own id; the class pin has lost \
         its negative control and the `== 0` assertions are unfalsifiable"
    );
}

#[test]
fn every_installed_title_is_served_its_own_authority_id() {
    let titles = titles();
    check_titles(&titles, boot);
}

#[cfg(test)]
mod tests {
    use super::*;
    use registry::ReferenceCell;

    fn title(short_name: &str, pending: Option<&str>) -> TitleUnderTest {
        TitleUnderTest {
            short_name: short_name.to_string(),
            content_id: format!("TEST-{short_name}"),
            max_steps: 1,
            reference: ReferenceCell {
                fw: "1.50".to_string(),
                game_ver: Some("base".to_string()),
                pending: pending.map(str::to_string),
            },
        }
    }

    fn witness(force_system_authid: bool) -> AuthorityWitness {
        AuthorityWitness {
            program_authority_id: if force_system_authid {
                BDJ_SELF_PROGRAM_AUTHORITY_ID
            } else {
                1
            },
            authid_source: "self".to_string(),
            lwmutex_unknown_locks: u64::from(force_system_authid),
        }
    }

    #[test]
    fn pending_cell_is_set_aside_while_runnable_sibling_executes() {
        let titles = [
            title("pending", Some("known boot refusal")),
            title("runnable", None),
        ];
        let mut calls = Vec::new();

        check_titles(&titles, |title, forced| {
            calls.push((title.short_name.clone(), forced));
            Some(witness(forced))
        });

        assert_eq!(
            calls,
            [
                ("runnable".to_string(), false),
                ("runnable".to_string(), true)
            ]
        );
    }

    #[test]
    #[should_panic(
        expected = "installed-title-tests is enabled but none of the 2 registered title(s) is installed"
    )]
    fn only_pending_or_uninstalled_cells_fail_the_anti_vacuity_floor() {
        let titles = [
            title("pending", Some("known boot refusal")),
            title("uninstalled", None),
        ];

        check_titles(&titles, |_, _| None);
    }
}
