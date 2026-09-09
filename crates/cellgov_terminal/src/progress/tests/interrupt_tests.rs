//! The Ctrl-C path, end to end: a child process runs an `Ansi` bar,
//! the test interrupts it, and the child's stderr must end with the
//! restore while its status is the interrupted one.
//!
//! The parent half is Unix only. Delivering a console Ctrl-C to one
//! child on Windows is `GenerateConsoleCtrlEvent`, FFI that
//! `forbid(unsafe_code)` rules out, and the event reaches every
//! process on the console besides.

use super::*;
use crate::progress::sink::ProgressSink;
use crate::progress::task::Unit;

const LONG: Task = Task {
    verb: "Installing",
    tag: "install",
    phases: &["staging", "writing"],
    measured: 1,
    unit: Unit::Bytes,
    items: "files",
    streaming: false,
};

/// How long the child half runs before it finishes on its own, so a
/// stray `--include-ignored` run ends instead of hanging.
const CHILD_LIFETIME: Duration = Duration::from_secs(10);

/// The child half: an `Ansi` bar with a measured phase under way, so
/// the bar sets the taskbar state as well as hiding the cursor. It
/// runs until a signal interrupts it or [`CHILD_LIFETIME`] passes.
///
/// Its render thread writes escape sequences to stderr, which the
/// harness cannot capture, so it runs only in the process its parent
/// spawns for it.
#[test]
#[ignore = "child half of the interrupt test; its parent runs it in a process of its own"]
fn child_half_an_ansi_bar_that_runs_until_it_is_interrupted() {
    let caps = TermCaps {
        mode: RenderMode::Ansi,
        color: false,
        width: 80,
    };
    let bar = ProgressBar::start(caps, &LONG, "interrupt.pkg");
    let sink = bar.sink();
    sink.phase(1);
    sink.totals(4, 1 << 20);
    let start = Instant::now();
    while start.elapsed() < CHILD_LIFETIME {
        sink.advanced(1024);
        std::thread::sleep(TICK);
    }
    bar.finish();
}

#[cfg(unix)]
mod parent {
    use super::*;
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    use std::io::Read as _;
    use std::process::{Command, Stdio};

    /// How long the parent waits for the child to hide its cursor.
    const CHILD_STARTUP: Duration = Duration::from_secs(10);

    /// The sequences spelled out rather than read from the bar's own
    /// constants, so a wrong constant cannot agree with itself here.
    const HIDE: &[u8] = b"\x1b[?25l";
    const CURSOR_BACK_AND_TASKBAR_CLEAR: &[u8] = b"\x1b[?25h\x1b]9;4;0\x07\n";
    /// 128 + SIGINT, what a shell reports for a run the default action
    /// ended.
    const INTERRUPTED: i32 = 130;

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn an_interrupted_ansi_bar_leaves_the_cursor_visible_and_exits_130() {
        let exe = std::env::current_exe().expect("the test binary knows its own path");
        let mut child = Command::new(exe)
            .args([
                "child_half_an_ansi_bar_that_runs_until_it_is_interrupted",
                "--ignored",
                "--nocapture",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the child half spawns");
        let mut pipe = child.stderr.take().expect("stderr is piped");
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let reader = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        // The interrupt waits for the hide: the bar installs its
        // handler before it hides, so a hide on the stream means the
        // handler is up. An earlier interrupt would test the default
        // action instead.
        let mut out = Vec::new();
        let start = Instant::now();
        while !contains(&out, HIDE) {
            let left = CHILD_STARTUP.saturating_sub(start.elapsed());
            assert!(
                !left.is_zero(),
                "the child never hid the cursor; its stderr so far: {:?}",
                String::from_utf8_lossy(&out)
            );
            match rx.recv_timeout(left) {
                Ok(chunk) => out.extend(chunk),
                Err(_) => break,
            }
        }
        assert!(
            contains(&out, HIDE),
            "the child's stderr closed before it hid the cursor: {:?}",
            String::from_utf8_lossy(&out)
        );

        let pid = Pid::from_raw(i32::try_from(child.id()).expect("a pid fits in i32"));
        kill(pid, Signal::SIGINT).expect("SIGINT reaches the child");

        let status = child.wait().expect("the child exits");
        reader.join().expect("the reader thread joins");
        for chunk in rx.iter() {
            out.extend(chunk);
        }

        let tail = String::from_utf8_lossy(&out[out.len().saturating_sub(64)..]);
        assert!(
            out.ends_with(CURSOR_BACK_AND_TASKBAR_CLEAR),
            "an interrupted bar must end its stderr with the restore; tail: {tail:?}"
        );
        assert_eq!(
            status.code(),
            Some(INTERRUPTED),
            "an interrupted run must exit as the default action would"
        );
    }
}
