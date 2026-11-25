//! The console queue through the verbs: two captures on one stand-in
//! console run one after the other, a console that does not answer
//! stops the queue, and a peek of a listed range never waits.

use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

use super::*;
use crate::ledger::LedgerError;
use crate::peek::Space;
use crate::queue::{QueueError, DOWN_FILE};
use crate::run::PLAY_PATH;
use crate::transport::TransportError;

/// One console that two runners share, as the reference console is
/// shared. Each runner's handle tags every request it sends, and every
/// request waits a little, so two runners that are not queued overlap.
///
/// `Arc<Mutex<_>>`: the two runners run on two threads and send their
/// requests to the one console state, which is what a console is.
#[derive(Clone)]
struct SharedConsole {
    console: Arc<Mutex<MemoryConsole>>,
    log: Arc<Mutex<Vec<char>>>,
    tag: char,
    frame: (String, Vec<u8>),
}

impl SharedConsole {
    fn send<T>(&self, request: impl FnOnce(&mut MemoryConsole) -> T) -> T {
        self.log.lock().expect("log").push(self.tag);
        std::thread::sleep(Duration::from_millis(1));
        request(&mut self.console.lock().expect("console"))
    }
}

impl ConsoleOps for SharedConsole {
    fn http_status(&mut self, path: &str, t: &mut Transcript) -> Result<u16, TransportError> {
        let frame = self.frame.clone();
        self.send(|c| {
            // Each start writes the packaged frame again.
            if path.starts_with(PLAY_PATH) {
                c.on_start = Some(frame);
            }
            c.http_status(path, t)
        })
    }

    fn fetch(&mut self, path: &str, t: &mut Transcript) -> Result<Option<Vec<u8>>, TransportError> {
        self.send(|c| c.fetch(path, t))
    }

    fn make_dir(&mut self, path: &str, t: &mut Transcript) -> Result<(), TransportError> {
        self.send(|c| c.make_dir(path, t))
    }

    fn store(&mut self, path: &str, b: &[u8], t: &mut Transcript) -> Result<(), TransportError> {
        self.send(|c| c.store(path, b, t))
    }

    fn list(&mut self, dir: &str, t: &mut Transcript) -> Result<Vec<String>, TransportError> {
        self.send(|c| c.list(dir, t))
    }

    fn delete(&mut self, path: &str, t: &mut Transcript) -> Result<(), TransportError> {
        self.send(|c| c.delete(path, t))
    }

    fn remove_dir(&mut self, path: &str, t: &mut Transcript) -> Result<(), TransportError> {
        self.send(|c| c.remove_dir(path, t))
    }

    fn end_session(&mut self, t: &mut Transcript) {
        self.send(|c| c.end_session(t));
    }
}

/// The words of a capture of `bench`'s packaged test.
fn capture_words(bench: &Bench) -> Vec<String> {
    let mut words = vec!["capture".to_string()];
    words.extend(OPERATOR.iter().map(|w| w.to_string()));
    words.extend([
        "--manifest".to_string(),
        bench.manifest_arg(),
        "--harness-revision".to_string(),
        "0123456789abcdef".to_string(),
    ]);
    words
}

#[test]
fn two_concurrent_captures_run_one_after_the_other() {
    let queue_dir = cellgov_testkit::scratch::scratch();
    let benches: Vec<Bench> = (0..2)
        .map(|_| {
            let mut bench = Bench::new();
            bench.context.queue = Queue::new(
                queue_dir.join("queue"),
                Duration::from_secs(20),
                Duration::from_millis(2),
            );
            bench
        })
        .collect();
    let target = benches[0].target();
    let mut console = MemoryConsole::empty();
    console
        .files
        .insert(STATUS_PATH.to_string(), PAGE.as_bytes().to_vec());
    let console = Arc::new(Mutex::new(console));
    let log = Arc::new(Mutex::new(Vec::new()));
    let start = Barrier::new(2);
    let results: Vec<Result<Report, Box<Failure>>> = std::thread::scope(|scope| {
        let runs: Vec<_> = benches
            .iter()
            .zip(['a', 'b'])
            .map(|(bench, tag)| {
                let shared = SharedConsole {
                    console: Arc::clone(&console),
                    log: Arc::clone(&log),
                    tag,
                    frame: (target.result_path.clone(), PACKAGED_FRAME.to_vec()),
                };
                let start = &start;
                scope.spawn(move || {
                    let args: Vec<OsString> =
                        capture_words(bench).iter().map(OsString::from).collect();
                    let command = parse(&args).expect("parses");
                    start.wait();
                    execute(
                        &command,
                        &bench.context,
                        |_| shared,
                        &mut |wait| std::thread::sleep(wait.min(Duration::from_millis(2))),
                        &mut || Ok("2026-11-11T12:00:00Z".to_string()),
                        &mut |_| Ok(true),
                    )
                })
            })
            .collect();
        runs.into_iter()
            .map(|run| run.join().expect("the runner thread"))
            .collect()
    });
    for (index, result) in results.iter().enumerate() {
        match result {
            Ok(Report::Console {
                capture: Some(_), ..
            }) => {}
            Ok(other) => panic!("capture {index}: {other:?}"),
            Err(failure) => panic!("capture {index}: {}", failure.error),
        }
    }
    let log = log.lock().expect("log").clone();
    let a = log.iter().filter(|&&tag| tag == 'a').count();
    assert!(
        a > 0 && a < log.len(),
        "both runners reached the console: {log:?}"
    );
    let switches = log.windows(2).filter(|pair| pair[0] != pair[1]).count();
    assert_eq!(switches, 1, "the two captures interleaved: {log:?}");
}

/// A console whose status page answers `refusals` reads with `error`
/// before it answers, and records every request.
fn refusing(refusals: usize) -> MemoryConsole {
    let mut console = MemoryConsole::empty();
    console
        .files
        .insert(STATUS_PATH.to_string(), PAGE.as_bytes().to_vec());
    console.status_refusals = refusals;
    console
}

#[test]
fn a_console_that_does_not_answer_is_marked_down_for_every_waiter() {
    // (name, status reads refused before the page answers, verb, the
    // reads sent, whether the console is marked down)
    let cases: &[(&str, usize, &str, usize, bool)] = &[
        ("status: never answers", usize::MAX, "status", 3, true),
        ("unlock: never answers", usize::MAX, "unlock", 3, true),
        ("peek: never answers", usize::MAX, "peek", 3, true),
        ("status: answers at the last read", 2, "status", 3, false),
        ("status: answers at once", 0, "status", 1, false),
    ];
    for (name, refusals, verb, reads, down) in cases {
        let mut bench = Bench::new();
        bench.console = refusing(*refusals);
        let words: Vec<&str> = match *verb {
            "unlock" => vec!["unlock"],
            "peek" => vec!["peek", "--space", "lv1", "--width", "1", "--address", "0x0"],
            _ => std::iter::once("status").chain(OPERATOR).collect(),
        };
        let result = bench.execute(&words);
        let sent = bench
            .console
            .calls
            .iter()
            .filter(|call| *call == &format!("FETCH {STATUS_PATH}"))
            .count();
        assert_eq!(sent, *reads, "{name}: {:?}", bench.console.calls);
        let mark = std::fs::read_to_string(bench.context.queue.dir().join(DOWN_FILE)).ok();
        if *down {
            let failure = result.expect_err("refused");
            assert!(
                matches!(
                    failure.error,
                    RunnerPs3Error::Queue(QueueError::ConsoleDown { .. })
                ),
                "{name}: {}",
                failure.error
            );
            assert_eq!(failure.error.exit_code(), ExitCode::Transport, "{name}");
            assert_eq!(
                bench.console.calls.len(),
                *reads,
                "{name}: sent past the status page"
            );
            let mark = mark.unwrap_or_else(|| panic!("{name}: no down mark"));
            assert!(
                !mark.contains(HOST),
                "{name}: the mark names the console: {mark}"
            );
        } else {
            assert_eq!(mark, None, "{name}: marked down");
            if let Err(failure) = result {
                assert!(
                    !matches!(failure.error, RunnerPs3Error::Queue(_)),
                    "{name}: {}",
                    failure.error
                );
            }
        }
    }
}

#[test]
fn a_status_page_that_answers_wrongly_is_not_a_console_down() {
    let mut bench = Bench::new();
    bench.console.files.remove(STATUS_PATH);
    let mut words = vec!["status"];
    words.extend(OPERATOR);
    let failure = bench.execute(&words).expect_err("404");
    assert!(
        matches!(
            failure.error,
            RunnerPs3Error::Transport(TransportError::UnexpectedStatus { status: 404, .. })
        ),
        "{}",
        failure.error
    );
    assert!(!bench.context.queue.dir().join(DOWN_FILE).exists());
}

#[test]
fn a_console_that_answers_clears_an_earlier_down_mark() {
    let mut bench = Bench::new();
    let dir = bench.context.queue.dir().to_path_buf();
    std::fs::create_dir_all(&dir).expect("queue dir");
    std::fs::write(dir.join(DOWN_FILE), "the status page did not answer").expect("mark");
    let mut words = vec!["status"];
    words.extend(OPERATOR);
    bench.execute(&words).expect("status");
    assert!(!dir.join(DOWN_FILE).exists());
}

#[test]
fn a_peek_of_a_listed_range_is_refused_before_it_waits_or_sends() {
    let mut bench = Bench::new();
    let failure = bench
        .execute(&[
            "peek",
            "--space",
            "lv2",
            "--width",
            "8",
            "--address",
            "0x8000000000354d98,0x800000000f000100",
        ])
        .expect_err("refused");
    match &failure.error {
        RunnerPs3Error::Ledger(LedgerError::Listed { space, first, .. }) => {
            assert_eq!((*space, *first), (Space::Lv2, 0x8000_0000_0f00_0000));
        }
        other => panic!("{other}"),
    }
    assert_eq!(failure.error.exit_code(), ExitCode::Refused);
    assert_eq!(bench.console.calls, Vec::<String>::new());
    assert!(
        !bench.context.queue.dir().exists(),
        "the peek joined the queue"
    );
}

#[test]
fn a_ledger_that_disagrees_with_its_stop_list_refuses_every_console_verb() {
    let mut bench = Bench::new();
    std::fs::write(
        bench.scratch.join(crate::ledger::STOP_LIST),
        format!("{}\n", crate::ledger::HEADER),
    )
    .expect("stop list");
    let mut status = vec!["status"];
    status.extend(OPERATOR);
    let peek = ["peek", "--space", "lv1", "--width", "1", "--address", "0x0"];
    for words in [status.as_slice(), &["unlock"], &peek] {
        let failure = bench.execute(words).expect_err("refused");
        assert!(
            matches!(
                failure.error,
                RunnerPs3Error::Ledger(LedgerError::Disagree { .. })
            ),
            "{words:?}: {}",
            failure.error
        );
    }
    assert_eq!(bench.console.calls, Vec::<String>::new());
}
