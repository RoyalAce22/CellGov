//! Every verb driven through the library against the in-memory console:
//! the report each returns, what it leaves on the console and on disk,
//! and the remedies, which name the front end that ran the verb.

use std::ffi::OsString;

use cellgov_observation::console_profile::console_profiles_path;
use cellgov_testkit::scratch::ScratchDir;

use super::*;
use crate::cli::parse;
use crate::console::STATUS_PATH;
use crate::memory_console::{package_on_disk, MemoryConsole, PACKAGED_FRAME};
use crate::ExitCode;

const PAGE: &str = include_str!("fixtures/cpursx.html");

const HOST: &str = "10.77.0.2";

const PROFILE: &str = "cech20-cex-493";

/// A front end other than the standalone binary.
const INVOCATION: &str = "cellgov ps3";

const PROFILES: &str = r#"
reference = "cech20-cex-493"

[load]
hot_c = 80
cool_c = 72
hdd_floor_mib = 1024
wait_poll_s = 15
wait_limit_s = 1800

[profile.cech20-cex-493]
models = ["CECH-20"]
kernel = "cex"
firmware = "4.93"
cfw = "EvilNAT"
cobra = "8.5"
debugger_attached = false

[profile.cech20-cex-492]
models = ["CECH-20"]
kernel = "cex"
firmware = "4.92"
cfw = "EvilNAT"
cobra = "8.5"
debugger_attached = false
"#;

/// The identity flags the fixture page needs from the operator.
const OPERATOR: [&str; 6] = [
    "--model",
    "CECH-2001A",
    "--cfw",
    "EvilNAT 4.93 PEX",
    "--debugger",
    "none",
];

/// A packaged test, a profiles file and a lease directory in scratch,
/// the context a front end passes for them, and the console.
struct Bench {
    scratch: ScratchDir,
    manifest: PathBuf,
    context: Context,
    console: MemoryConsole,
}

impl Bench {
    fn new() -> Self {
        let scratch = cellgov_testkit::scratch::scratch();
        let manifest = package_on_disk(&scratch);
        let profiles = console_profiles_path(&scratch);
        std::fs::create_dir_all(profiles.parent().expect("micro dir")).expect("micro dir");
        std::fs::write(&profiles, PROFILES).expect("profiles");
        let lease_dir = scratch.join("leases");
        std::fs::create_dir_all(&lease_dir).expect("lease dir");
        let mut console = MemoryConsole::empty();
        console
            .files
            .insert(STATUS_PATH.to_string(), PAGE.as_bytes().to_vec());
        let context = Context {
            host_env: Some(HOST.to_string()),
            profile_env: Some(PROFILE.to_string()),
            lease_dir,
            workspace_root: scratch.to_path_buf(),
            invocation: INVOCATION.to_string(),
        };
        Self {
            scratch,
            manifest,
            context,
            console,
        }
    }

    fn manifest_arg(&self) -> String {
        self.manifest.display().to_string()
    }

    fn target(&self) -> Target {
        let manifest = manifest::load_console(&self.manifest).expect("manifest");
        Target::new(&manifest.ps3.appid, &manifest.result_file_name())
    }

    /// The console's next start writes the packaged frame.
    fn arm(&mut self) {
        self.console.on_start = Some((self.target().result_path, PACKAGED_FRAME.to_vec()));
    }

    /// `verb` with the operator's flags, the manifest, and `extra`.
    fn on_console(&mut self, verb: &str, extra: &[&str]) -> Result<Report, Box<Failure>> {
        let manifest = self.manifest_arg();
        let mut words = vec![verb];
        words.extend(OPERATOR);
        words.extend(["--manifest", manifest.as_str()]);
        words.extend_from_slice(extra);
        self.execute(&words)
    }

    /// Parse `words` and run them, asserting the console opened, if at
    /// all, is the one the context names, and the lease is as the verb
    /// found it.
    fn execute(&mut self, words: &[&str]) -> Result<Report, Box<Failure>> {
        let args: Vec<OsString> = words.iter().map(OsString::from).collect();
        let command = parse(&args).expect("parses");
        let lease = lease::lease_path(&self.context.lease_dir, HOST);
        let held = lease.exists();
        let console = &mut self.console;
        let mut opened = None;
        let result = execute(
            &command,
            &self.context,
            |host| {
                opened = Some(host.to_string());
                console
            },
            &mut |_| {},
            &mut || Ok("2026-09-30T12:00:00Z".to_string()),
            &mut |_| Ok(true),
        );
        if let Some(host) = opened {
            assert_eq!(host, HOST);
        }
        if words[0] != "unlock" {
            assert_eq!(lease.exists(), held, "{words:?} changed the lease");
        }
        result
    }
}

fn transcript_of(report: Report) -> Vec<String> {
    match report {
        Report::Console {
            capture: None,
            transcript,
        } => transcript,
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_verb_runs_through_the_library() {
    let mut bench = Bench::new();
    let target = bench.target();

    let mut status = vec!["status"];
    status.extend(OPERATOR);
    match bench.execute(&status).expect("status") {
        Report::Status(status) => {
            let lines = status.lines();
            assert_eq!(lines[0], "profile cech20-cex-493");
            assert!(
                lines.contains(&"  firmware: 4.93 ok".to_string()),
                "{lines:?}"
            );
        }
        other => panic!("{other:?}"),
    }

    let report = bench.on_console("deploy", &[]).expect("deploy");
    let lines = report.lines();
    let deployed = transcript_of(report);
    assert_eq!(lines, deployed, "a console verb's lines are its transcript");
    assert!(
        deployed
            .iter()
            .any(|line| line.ends_with("console satisfies profile cech20-cex-493")),
        "{deployed:?}"
    );
    assert!(bench
        .console
        .files
        .contains_key(&format!("{}/EBOOT.BIN", target.usrdir)));

    bench.arm();
    transcript_of(bench.on_console("run", &[]).expect("run"));
    assert!(bench.console.files.contains_key(&target.result_path));

    let frame = bench.scratch.join("frame.bin");
    let frame_arg = frame.display().to_string();
    transcript_of(
        bench
            .on_console("fetch", &["--out", &frame_arg])
            .expect("fetch"),
    );
    assert_eq!(std::fs::read(&frame).expect("fetched"), PACKAGED_FRAME);

    transcript_of(bench.on_console("cleanup", &[]).expect("cleanup"));
    assert!(!bench.console.dirs.contains(&target.game_dir));
    assert!(!bench.console.files.contains_key(&target.result_path));

    bench.arm();
    let report = bench
        .on_console("capture", &["--harness-revision", "0123456789abcdef"])
        .expect("capture");
    let out = bench
        .manifest
        .parent()
        .expect("test dir")
        .join(CAPTURE_DIR)
        .join(PROFILE);
    match &report {
        Report::Console {
            capture: Some(capture),
            ..
        } => {
            assert_eq!(capture.out, out);
            assert_eq!(
                report.lines()[0],
                format!("{} -> {}", capture.capture_id, out.display())
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(out.join("observation.json").is_file());

    let manifest = bench.manifest_arg();
    let json = match bench
        .execute(&["convert", "--frame", &frame_arg, "--manifest", &manifest])
        .expect("convert")
    {
        report @ Report::Convert { out: None, .. } => {
            let lines = report.lines();
            assert_eq!(lines.len(), 1);
            lines[0].clone()
        }
        other => panic!("{other:?}"),
    };
    let written = bench.scratch.join("observation.json");
    let written_arg = written.display().to_string();
    let report = bench
        .execute(&[
            "convert",
            "--frame",
            &frame_arg,
            "--manifest",
            &manifest,
            "--out",
            &written_arg,
        ])
        .expect("convert to a file");
    assert!(report.lines().is_empty(), "{report:?}");
    assert_eq!(std::fs::read_to_string(&written).expect("written"), json);

    let report = bench.execute(&["unlock"]).expect("unlock");
    assert_eq!(report.lines(), ["no lease on 10.77.0.2"]);
    std::mem::forget(
        Lease::acquire(&bench.context.lease_dir, HOST, "stale", INVOCATION).expect("free"),
    );
    let report = bench.execute(&["unlock"]).expect("unlock");
    assert_eq!(
        report,
        Report::Unlock {
            host: HOST.to_string(),
            removed: true
        }
    );
    assert_eq!(report.lines(), ["removed the lease on 10.77.0.2"]);
}

#[test]
fn a_failed_claim_still_reports_status_lines() {
    let mut bench = Bench::new();
    let mut words = vec!["status", "--profile", "cech20-cex-492"];
    words.extend(OPERATOR);
    let failure = bench.execute(&words).expect_err("4.93 fails a 4.92 claim");
    assert!(
        matches!(failure.error, RunnerPs3Error::Profile(_)),
        "{failure:?}"
    );
    match failure.report {
        Some(Report::Status(status)) => {
            let lines = status.lines();
            assert!(
                lines
                    .contains(&"  firmware: 4.93 FAILS, the profile requires \"4.92\"".to_string()),
                "{lines:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_remedy_names_the_front_end_that_ran_the_verb() {
    let mut bench = Bench::new();
    bench.on_console("deploy", &[]).expect("deploy");
    let failure = bench
        .on_console("deploy", &[])
        .expect_err("the game directory is occupied");
    match &failure.error {
        RunnerPs3Error::Refused { clear_with, .. } => assert_eq!(
            clear_with,
            &format!(
                "cellgov ps3 cleanup --host 10.77.0.2 --profile cech20-cex-493 --model CECH-2001A \
                 --cfw \"EvilNAT 4.93 PEX\" --debugger none --manifest {}, or rerun with --reclaim",
                shell_word(&bench.manifest_arg())
            )
        ),
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(failure.report, Some(Report::Console { .. })),
        "a refused verb still reports its transcript: {failure:?}"
    );

    let held = Lease::acquire(&bench.context.lease_dir, HOST, "another", INVOCATION).expect("free");
    let failure = bench.on_console("cleanup", &[]).expect_err("held");
    assert!(failure.report.is_none(), "{failure:?}");
    assert!(
        failure
            .error
            .to_string()
            .ends_with("clear it with `cellgov ps3 unlock --host 10.77.0.2`"),
        "{}",
        failure.error
    );
    held.release().expect("release");
}

/// The fixture page with the Cell at `cpu_c`.
fn page_at(cpu_c: u32) -> Vec<u8> {
    PAGE.replace("CPU: 67&deg;C", &format!("CPU: {cpu_c}&deg;C"))
        .into_bytes()
}

#[test]
fn a_hot_console_refuses_every_verb_that_loads_it_and_leaves_status_unlock_and_cleanup_open() {
    let mut bench = Bench::new();
    bench
        .console
        .files
        .insert(STATUS_PATH.to_string(), page_at(85));
    let frame = bench.scratch.join("frame.bin").display().to_string();
    for (verb, extra) in [
        ("deploy", vec![]),
        ("run", vec![]),
        ("fetch", vec!["--out", frame.as_str()]),
        ("capture", vec!["--harness-revision", "0123456789abcdef"]),
    ] {
        let failure = bench.on_console(verb, &extra).expect_err("hot");
        assert_eq!(failure.error.exit_code(), ExitCode::Refused, "{verb}");
        assert_eq!(
            failure.error.to_string(),
            "load: the console is hot: Cell 85 C, RSX 64 C; it is hot at 80 C and cool again \
             only below 72 C. Poll `cellgov ps3 status --host 10.77.0.2 --profile cech20-cex-493 \
             --model CECH-2001A --cfw \"EvilNAT 4.93 PEX\" --debugger none` until it reads cool, \
             or rerun with --wait-cool",
            "{verb}"
        );
    }
    assert!(
        bench
            .console
            .calls
            .iter()
            .all(|call| call == "FETCH /cpursx.ps3"),
        "a refused verb changes nothing: {:?}",
        bench.console.calls
    );
    let mut status = vec!["status"];
    status.extend(OPERATOR);
    match bench.execute(&status).expect("status stays open") {
        Report::Status(report) => {
            assert_eq!(report.thermal, Some(load::Thermal::Hot));
            assert!(
                report.lines().contains(
                    &"thermal: hot; deploy, run, fetch and capture refuse until it is cool"
                        .to_string()
                ),
                "{report:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    transcript_of(
        bench
            .on_console("cleanup", &[])
            .expect("cleanup stays open"),
    );
    bench.execute(&["unlock"]).expect("unlock stays open");
}

#[test]
fn a_reading_between_the_floor_and_the_ceiling_is_refused_only_after_a_hot_one() {
    let mut warm = Bench::new();
    warm.console
        .files
        .insert(STATUS_PATH.to_string(), page_at(75));
    warm.on_console("deploy", &[])
        .expect("75 C after a cool reading is cool");

    let mut bench = Bench::new();
    let mut status = vec!["status"];
    status.extend(OPERATOR);
    bench
        .console
        .files
        .insert(STATUS_PATH.to_string(), page_at(80));
    bench.execute(&status).expect("status reads it hot");
    bench
        .console
        .files
        .insert(STATUS_PATH.to_string(), page_at(75));
    let failure = bench.on_console("deploy", &[]).expect_err("still cooling");
    assert!(
        failure
            .error
            .to_string()
            .starts_with("load: the console is still cooling: Cell 75 C"),
        "{}",
        failure.error
    );
    bench
        .console
        .files
        .insert(STATUS_PATH.to_string(), page_at(71));
    bench
        .on_console("deploy", &[])
        .expect("cool below the floor");
}

#[test]
fn a_capture_records_the_reading_it_started_from() {
    let mut bench = Bench::new();
    bench
        .console
        .files
        .insert(STATUS_PATH.to_string(), page_at(84));
    bench.console.later_status = [page_at(70)].into();
    bench.arm();
    bench
        .on_console(
            "capture",
            &["--harness-revision", "0123456789abcdef", "--wait-cool"],
        )
        .expect("capture once cool");
    let provenance = bench
        .manifest
        .parent()
        .expect("test dir")
        .join(CAPTURE_DIR)
        .join(PROFILE)
        .join("provenance.json");
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(provenance).expect("written")).expect("json");
    assert_eq!(
        record["console"]["load_at_start"],
        serde_json::json!({ "cpu_c": 70, "rsx_c": 64, "fan_percent": null })
    );
}
