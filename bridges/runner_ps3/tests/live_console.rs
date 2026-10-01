//! The one test target that reaches a console. It builds only with the
//! `ps3-hardware` feature, and with it on, every missing input is a hard
//! error naming it, never a skip: the feature is the statement that a
//! console is on the network.
//!
//! Besides `CELLGOV_PS3_HOST` and `CELLGOV_PS3_PROFILE`, the suite reads
//! the three facts the status page does not state, which the runner
//! takes as flags: [`MODEL`], [`CFW`] and [`DEBUGGER`]. Every test takes
//! the console's lease, so they run one at a time.

use std::ffi::OsString;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use cellgov_observation::hardware_capture::{CAPTURE_DIR, FRAME_FILE};
use runner_ps3::cli;
use runner_ps3::console::StatusVerdict;
use runner_ps3::run::{ConsoleOps, Target, WebmanConsole};
use runner_ps3::transcript::Transcript;
use runner_ps3::transport::Endpoint;
use runner_ps3::verbs::{self, Context, Failure, Report};
use runner_ps3::{env, load, ExitCode, RunnerPs3Error};

/// The console's model, for `--model`.
const MODEL: &str = "CELLGOV_PS3_MODEL";

/// The CFW name and build, for `--cfw`.
const CFW: &str = "CELLGOV_PS3_CFW";

/// `none` or `attached`, for `--debugger`.
const DEBUGGER: &str = "CELLGOV_PS3_DEBUGGER";

/// The microtest the capture and refusal tests deploy.
const TEST: &str = "spu_fixed_value";

/// One test on the console at a time: each takes its lease.
static CONSOLE: Mutex<()> = Mutex::new(());

fn required(name: &str) -> String {
    let value =
        std::env::var(name).unwrap_or_else(|_| panic!("ps3-hardware is on, so {name} must be set"));
    assert!(!value.trim().is_empty(), "{name} is set but empty");
    value
}

fn console() -> Endpoint {
    Endpoint::new(required(env::HOST))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manifest() -> PathBuf {
    let dir = workspace_root().join("tests/micro").join(TEST);
    let eboot = dir.join("build/ps3/EBOOT.BIN");
    assert!(
        eboot.is_file(),
        "{} is missing: build the microtest's PS3 package first",
        eboot.display()
    );
    dir.join("manifest.toml")
}

/// The runner's verb `verb` with the identity flags and `extra`, run
/// through the library against the console.
fn run(verb: &str, extra: &[&str]) -> Result<Report, Box<Failure>> {
    let (model, cfw, debugger) = (required(MODEL), required(CFW), required(DEBUGGER));
    let mut words = vec![
        verb,
        "--model",
        model.as_str(),
        "--cfw",
        cfw.as_str(),
        "--debugger",
        debugger.as_str(),
    ];
    words.extend_from_slice(extra);
    let args: Vec<OsString> = words.iter().map(OsString::from).collect();
    let command = cli::parse(&args).unwrap_or_else(|e| panic!("{words:?}: {e}"));
    let context = Context {
        host_env: Some(required(env::HOST)),
        profile_env: Some(required(env::PROFILE)),
        marker_dir: load::default_marker_dir(),
        who: env::who(|name| std::env::var(name).ok()),
        workspace_root: workspace_root(),
        invocation: "runner_ps3".to_string(),
    };
    verbs::execute(
        &command,
        &context,
        |host| WebmanConsole::new(Endpoint::new(host)),
        &mut std::thread::sleep,
        &mut runner_ps3::provenance::now_rfc3339,
        &mut |_| Ok(false),
    )
}

/// The verb's report, or a panic naming its error.
fn ok(verb: &str, result: Result<Report, Box<Failure>>) -> Report {
    result.unwrap_or_else(|failure| panic!("{verb}: {}", failure.error))
}

#[test]
fn the_console_accepts_a_connection_on_webmans_http_port() {
    let endpoint = console();
    let address = (endpoint.host.as_str(), endpoint.http_port)
        .to_socket_addrs()
        .expect("the host resolves")
        .next()
        .expect("the host has an address");
    let timeout = Duration::from_millis(endpoint.io_timeout_ms);
    let stream = TcpStream::connect_timeout(&address, timeout).unwrap_or_else(|e| {
        panic!(
            "{}:{} did not accept a connection: {e}",
            endpoint.host, endpoint.http_port
        )
    });
    assert_eq!(
        stream.peer_addr().expect("peer address").port(),
        endpoint.http_port
    );
}

#[test]
fn the_console_satisfies_its_claimed_profile() {
    let _console = CONSOLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match ok("status", run("status", &[])) {
        Report::Status(report) => {
            assert_eq!(report.claimed, required(env::PROFILE));
            assert_eq!(report.verdict, StatusVerdict::Pass, "{:#?}", report.lines());
            assert!(
                report.thermal.is_some(),
                "the page states the temperatures: {:#?}",
                report.lines()
            );
        }
        other => panic!("status reported {other:?}"),
    }
}

#[test]
fn a_capture_reproduces_the_committed_frame() {
    let _console = CONSOLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifest = manifest();
    let profile = required(env::PROFILE);
    let committed = manifest
        .parent()
        .expect("test dir")
        .join(CAPTURE_DIR)
        .join(&profile)
        .join(FRAME_FILE);
    assert!(
        committed.is_file(),
        "profile {profile} has no committed capture of {TEST} at {}",
        committed.display()
    );
    let scratch = cellgov_testkit::scratch::scratch();
    let out = scratch.join("capture");
    let manifest_arg = manifest.display().to_string();
    let out_arg = out.display().to_string();
    ok(
        "capture",
        run(
            "capture",
            &[
                "--manifest",
                &manifest_arg,
                "--harness-revision",
                "live-console-suite",
                "--out",
                &out_arg,
                "--wait-cool",
            ],
        ),
    );
    assert_eq!(
        std::fs::read(out.join(FRAME_FILE)).expect("the capture wrote a frame"),
        std::fs::read(&committed).expect("committed frame"),
        "the console's frame differs from {}",
        committed.display()
    );
}

#[test]
fn a_dirty_console_is_refused_with_a_recovery_command() {
    let _console = CONSOLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifest = manifest();
    let manifest_arg = manifest.display().to_string();
    let target = {
        let parsed =
            cellgov_observation::manifest::load_console(&manifest).expect("the manifest loads");
        Target::new(&parsed.ps3.appid, &parsed.result_file_name())
    };
    ok(
        "deploy",
        run("deploy", &["--manifest", &manifest_arg, "--wait-cool"]),
    );
    // A stale result beside the deployed package: the runner deletes it
    // once, then refuses the occupied game directory.
    WebmanConsole::new(console())
        .store(&target.result_path, b"stale", &mut Transcript::new())
        .expect("plant a stale result");
    let refused = run(
        "capture",
        &[
            "--manifest",
            &manifest_arg,
            "--harness-revision",
            "live-console-suite",
            "--out",
            &cellgov_testkit::scratch::scratch()
                .join("capture")
                .display()
                .to_string(),
            "--wait-cool",
        ],
    );
    let cleaned = run("cleanup", &["--manifest", &manifest_arg]);
    let failure = refused.expect_err("an occupied game directory is refused");
    ok("cleanup", cleaned);
    assert_eq!(
        failure.error.exit_code(),
        ExitCode::Refused,
        "{}",
        failure.error
    );
    match &failure.error {
        RunnerPs3Error::Refused { reason, clear_with } => {
            assert_eq!(
                reason,
                &format!("{} already exists on the console", target.game_dir)
            );
            assert!(
                clear_with.starts_with("runner_ps3 cleanup --host ")
                    && clear_with.ends_with(", or rerun with --reclaim"),
                "{clear_with}"
            );
        }
        other => panic!("{other}"),
    }
    match failure.report {
        Some(Report::Console { transcript, .. }) => assert!(
            transcript
                .iter()
                .any(|line| line.ends_with(&format!("> DELE {}", target.result_path))),
            "the stale result was deleted first: {transcript:#?}"
        ),
        other => panic!("{other:?}"),
    }
}
