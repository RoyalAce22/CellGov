//! The console queue's rules, one case table each: first in, first
//! out; the bounded wait; the reclaim of a ticket whose owner is gone;
//! and the down mark that stops every waiter.
//!
//! Every wait goes through a sleep that counts its calls and panics past
//! a cap, so no case can loop without bound.

use std::time::Duration;

use cellgov_testkit::scratch::{scratch, ScratchDir};

use super::*;

/// The most sleeps any case may take.
const SLEEP_CAP: usize = 64;

fn queue_in(dir: &ScratchDir, wait_ms: u64, poll_ms: u64) -> Queue {
    Queue::new(
        dir.join("queue"),
        Duration::from_millis(wait_ms),
        Duration::from_millis(poll_ms),
    )
}

/// Join `queue` as `holder`, counting the sleeps; `on_sleep` runs at
/// each sleep with its 1-based count.
fn join_counting(
    queue: &Queue,
    holder: &str,
    mut on_sleep: impl FnMut(usize),
) -> (Result<Turn, QueueError>, usize) {
    let mut slept = 0;
    let result = queue.join(holder, &mut |_| {
        slept += 1;
        assert!(slept <= SLEEP_CAP, "the wait ran past {SLEEP_CAP} sleeps");
        on_sleep(slept);
    });
    (result, slept)
}

/// Leave ticket `number` as a process that ended leaves it: both files,
/// no lock.
fn plant_dead(queue: &Queue, number: u64, holder: &str) {
    fs::create_dir_all(queue.dir()).expect("queue dir");
    File::create(ticket_path(queue.dir(), number)).expect("ticket");
    fs::write(holder_path(queue.dir(), number), holder).expect("holder");
}

fn listed(queue: &Queue) -> Vec<u64> {
    queue.numbers().expect("lists")
}

#[test]
fn waiters_are_served_in_the_order_they_joined() {
    // `+x` joins as x; `-` serves the head and checks every waiter's
    // place behind it.
    let cases: &[(&str, &[&str], &[&str])] = &[
        ("two in a row", &["+a", "+b", "-", "-"], &["a", "b"]),
        (
            "five in a row",
            &["+a", "+b", "+c", "+d", "+e", "-", "-", "-", "-", "-"],
            &["a", "b", "c", "d", "e"],
        ),
        (
            "late arrivals wait behind the earlier ones",
            &["+a", "+b", "-", "+c", "+d", "-", "-", "-"],
            &["a", "b", "c", "d"],
        ),
        (
            "an arrival after the queue empties is served at once",
            &["+a", "-", "+b", "-"],
            &["a", "b"],
        ),
    ];
    for (name, script, expected) in cases {
        let dir = scratch();
        let queue = queue_in(&dir, 0, 1);
        let mut waiting: Vec<(String, Ticket)> = Vec::new();
        let mut served = Vec::new();
        for step in *script {
            if let Some(who) = step.strip_prefix('+') {
                let ticket = queue.enqueue(&format!("holder {who}")).expect("joins");
                waiting.push((who.to_string(), ticket));
                continue;
            }
            assert!(!waiting.is_empty(), "{name}: nothing to serve");
            let head_holder = format!("holder {}", waiting[0].0);
            for (place, (who, ticket)) in waiting.iter().enumerate() {
                let expected_position = if place == 0 {
                    Position::Head
                } else {
                    Position::Behind {
                        ahead: place,
                        holder: head_holder.clone(),
                    }
                };
                assert_eq!(
                    queue.position(ticket).expect("position"),
                    expected_position,
                    "{name}: {who}"
                );
            }
            served.push(waiting.remove(0).0);
        }
        assert_eq!(&served, expected, "{name}");
        assert_eq!(
            listed(&queue),
            Vec::<u64>::new(),
            "{name}: every ticket left"
        );
    }
}

#[test]
fn a_waiter_gives_up_once_the_wait_runs_out_and_leaves_the_queue() {
    let cases: &[(&str, u64, u64, usize)] = &[
        ("no wait", 0, 10, 0),
        ("one poll", 10, 10, 1),
        ("a wait that is not a whole number of polls", 25, 10, 3),
        ("four polls", 120, 30, 4),
    ];
    for (name, wait_ms, poll_ms, sleeps) in cases {
        let dir = scratch();
        let queue = queue_in(&dir, *wait_ms, *poll_ms);
        let holder = queue
            .enqueue("pid 1, ana@bench-a, running capture")
            .expect("joins");
        let (result, slept) = join_counting(&queue, "pid 2, waiter", |_| {});
        assert_eq!(slept, *sleeps, "{name}");
        match result {
            Err(QueueError::Timeout {
                waited_ms,
                ahead,
                holder: named,
                ..
            }) => {
                assert_eq!(waited_ms, *sleeps as u64 * poll_ms, "{name}");
                assert_eq!(ahead, 1, "{name}");
                assert_eq!(named, "pid 1, ana@bench-a, running capture", "{name}");
            }
            other => panic!("{name}: {other:?}"),
        }
        assert_eq!(
            listed(&queue),
            vec![holder.number()],
            "{name}: the waiter left"
        );
    }
}

/// Who holds a ticket ahead of the waiter.
#[derive(Clone, Copy)]
enum Owner {
    /// A process that ended without removing its ticket.
    Dead,
    /// A live process.
    Live,
}

#[test]
fn a_ticket_whose_owner_is_gone_is_reclaimed() {
    use Owner::{Dead, Live};
    let cases: &[(&str, &[Owner])] = &[
        ("a dead holder", &[Dead]),
        ("two dead tickets ahead", &[Dead, Dead]),
        ("a dead holder ahead of a live waiter", &[Dead, Live]),
        ("a dead waiter behind a live holder", &[Live, Dead]),
        ("a dead ticket between two live ones", &[Live, Dead, Live]),
    ];
    for (name, ahead) in cases {
        let dir = scratch();
        let queue = queue_in(&dir, 0, 1);
        let mut live = Vec::new();
        let mut dead = Vec::new();
        for (index, owner) in ahead.iter().enumerate() {
            let number = index as u64 + 1;
            match owner {
                Dead => {
                    plant_dead(&queue, number, &format!("pid 9{number}, gone"));
                    dead.push(number);
                }
                Live => live.push(queue.enqueue(&format!("pid {number}")).expect("joins")),
            }
        }
        let (result, slept) = join_counting(&queue, "the waiter", |_| {});
        assert_eq!(slept, 0, "{name}");
        match (live.first(), result) {
            (None, Ok(turn)) => drop(turn),
            (Some(first), Err(QueueError::Timeout { ahead, holder, .. })) => {
                assert_eq!(ahead, live.len(), "{name}: only live tickets are ahead");
                assert_eq!(holder, format!("pid {}", first.number()), "{name}");
            }
            (_, other) => panic!("{name}: {other:?}"),
        }
        let left = listed(&queue);
        for number in &dead {
            assert!(!left.contains(number), "{name}: dead ticket {number} stays");
            assert!(
                !holder_path(queue.dir(), *number).exists(),
                "{name}: its holder file stays"
            );
        }
        let log = fs::read_to_string(queue.dir().join(LOG_FILE)).expect("log");
        for number in &dead {
            assert!(
                log.lines().any(|line| line.ends_with(&format!(
                    "ticket {number} reclaimed: no live owner (pid 9{number}, gone)"
                ))),
                "{name}: no reclaim line for {number}:\n{log}"
            );
        }
        let live_numbers: Vec<u64> = live.iter().map(Ticket::number).collect();
        assert_eq!(left, live_numbers, "{name}: the live tickets stay");
    }
}

#[test]
fn a_console_marked_down_stops_every_waiter_but_the_head() {
    // (name, a live holder ahead, the sleep at which the mark appears,
    // sleeps taken, whether the waiter gets the turn)
    let cases: &[(&str, bool, Option<usize>, usize, bool)] = &[
        ("a mark before the waiter joins", true, Some(0), 0, false),
        (
            "a mark that appears while it waits",
            true,
            Some(2),
            2,
            false,
        ),
        (
            "no mark: the waiter waits on",
            true,
            None,
            SLEEP_CAP / 2,
            false,
        ),
        (
            "the head is granted despite a mark",
            false,
            Some(0),
            0,
            true,
        ),
    ];
    for (name, held, mark_at, sleeps, granted) in cases {
        let dir = scratch();
        let wait_ms = (SLEEP_CAP / 2) as u64;
        let queue = queue_in(&dir, wait_ms, 1);
        let holder = held.then(|| queue.enqueue("pid 1").expect("joins"));
        let mark = || {
            fs::create_dir_all(queue.dir()).expect("queue dir");
            fs::write(
                queue.dir().join(DOWN_FILE),
                "the status page did not answer",
            )
            .expect("mark");
        };
        if *mark_at == Some(0) {
            mark();
        }
        let (result, slept) = join_counting(&queue, "pid 2", |count| {
            if *mark_at == Some(count) {
                mark();
            }
        });
        assert_eq!(slept, *sleeps, "{name}");
        match (granted, result) {
            (true, Ok(turn)) => {
                turn.console_up().expect("clears");
                assert_eq!(queue.down().expect("reads"), None, "{name}: mark cleared");
            }
            (false, Err(QueueError::ConsoleDown { cause })) => {
                assert!(mark_at.is_some(), "{name}: down without a mark");
                assert_eq!(cause, "the status page did not answer", "{name}");
            }
            (false, Err(QueueError::Timeout { .. })) => {
                assert!(mark_at.is_none(), "{name}: waited out a mark");
            }
            (_, other) => panic!("{name}: {other:?}"),
        }
        let expected: Vec<u64> = holder.iter().map(Ticket::number).collect();
        assert_eq!(listed(&queue), expected, "{name}: the waiter left");
    }
}

#[test]
fn the_holder_that_finds_the_console_down_marks_it_and_leaves() {
    let dir = scratch();
    let queue = queue_in(&dir, 0, 1);
    let turn = queue
        .join("pid 1", &mut |_| panic!("no wait"))
        .expect("head");
    match turn.console_down("connect to the console: timed out") {
        QueueError::ConsoleDown { cause } => {
            assert_eq!(cause, "connect to the console: timed out");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        queue.down().expect("reads").as_deref(),
        Some("connect to the console: timed out")
    );
    assert_eq!(listed(&queue), Vec::<u64>::new());
}

#[test]
fn the_holder_text_masks_the_console_wherever_the_command_line_names_it() {
    let words = |line: &[&str]| line.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    let cases: &[(&str, &[&str], &str, &str)] = &[
        (
            "a --host flag",
            &["runner_ps3", "status", "--host", "192.0.2.2"],
            "192.0.2.2",
            "runner_ps3 status --host <host>",
        ),
        (
            "a --host= flag",
            &["runner_ps3", "status", "--host=ps3.local"],
            "ps3.local",
            "runner_ps3 status --host=<host>",
        ),
        (
            "the host inside another word",
            &["cellgov", "ps3", "fetch", "--out", "192.0.2.2.bin"],
            "192.0.2.2",
            "cellgov ps3 fetch --out <host>.bin",
        ),
        (
            "a word with a space, and a MAC",
            &[
                "runner_ps3",
                "capture",
                "--reason",
                "MAC 00:11:22:33:44:55 moved",
            ],
            "192.0.2.2",
            "runner_ps3 capture --reason \"MAC <mac> moved\"",
        ),
    ];
    for (name, line, host, masked) in cases {
        let text = masked_command_line(&words(line), host);
        assert_eq!(text, *masked, "{name}");
        assert!(!text.contains(host), "{name}");
    }
}

#[test]
fn with_no_host_an_empty_word_stays_an_empty_quoted_word() {
    let words = ["runner_ps3", "capture", "--reason", ""].map(String::from);
    assert_eq!(
        masked_command_line(&words, " "),
        "runner_ps3 capture --reason \"\""
    );
}
