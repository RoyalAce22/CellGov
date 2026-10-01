use super::*;

use std::io::Cursor;

const ASK: Answers = Answers {
    yes: false,
    no_input: false,
};

const INSTEAD: &str = "--dry-run to see the plan";

fn on_a_terminal(input: &str) -> bool {
    ask(
        "remove it?",
        ASK,
        INSTEAD,
        &mut Cursor::new(input.to_string()),
        || true,
    )
    .expect("a terminal can be asked")
}

#[test]
fn yes_answers_without_reading_input() {
    let answers = Answers {
        yes: true,
        no_input: false,
    };
    // An empty reader would answer no if it were consulted.
    assert_eq!(
        ask(
            "remove it?",
            answers,
            INSTEAD,
            &mut Cursor::new(String::new()),
            || { panic!("--yes must not consult the terminal") }
        ),
        Ok(true)
    );
}

#[test]
fn only_an_explicit_yes_goes_ahead() {
    for input in ["y\n", "Y\n", "yes\n", "YES\n", " yes \n"] {
        assert!(on_a_terminal(input), "{input:?} must confirm");
    }
    for input in ["n\n", "no\n", "\n", "maybe\n", ""] {
        assert!(!on_a_terminal(input), "{input:?} must not confirm");
    }
}

#[test]
fn an_input_that_ends_immediately_declines() {
    assert!(!on_a_terminal(""));
}

/// A terminal that goes away mid-prompt.
struct Broken;

impl std::io::Read for Broken {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "the input went away",
        ))
    }
}

impl BufRead for Broken {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "the input went away",
        ))
    }

    fn consume(&mut self, _amount: usize) {}
}

#[test]
fn an_answer_that_cannot_be_read_declines() {
    assert_eq!(
        ask("remove it?", ASK, INSTEAD, &mut Broken, || true),
        Ok(false)
    );
}

#[test]
fn a_run_that_cannot_ask_is_a_usage_message_naming_both_ways_out() {
    let no_input = Answers {
        yes: false,
        no_input: true,
    };
    for (answers, terminal, said) in [
        (no_input, true, "this run may not prompt (--no-input)"),
        (ASK, false, "stdin is not a terminal"),
    ] {
        let usage = ask(
            "remove it?",
            answers,
            "run without --reclaim",
            &mut Cursor::new(
                "y
"
                .to_string(),
            ),
            || terminal,
        )
        .expect_err("cannot ask")
        .to_string();
        assert!(
            usage.starts_with(
                "remove it?
"
            ),
            "{usage}"
        );
        assert!(usage.contains(said), "{usage}");
        assert!(
            usage.ends_with("pass --yes to answer it, or run without --reclaim"),
            "{usage}"
        );
    }
}
