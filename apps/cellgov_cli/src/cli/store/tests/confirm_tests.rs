use super::*;

use std::io::Cursor;

const ASK: Answers = Answers {
    yes: false,
    no_input: false,
};

fn on_a_terminal(input: &str) -> bool {
    ask(
        "remove it?",
        ASK,
        &mut Cursor::new(input.to_string()),
        || true,
    )
}

#[test]
fn yes_answers_without_reading_input() {
    let answers = Answers {
        yes: true,
        no_input: false,
    };
    // An empty reader would answer no if it were consulted.
    assert!(ask(
        "remove it?",
        answers,
        &mut Cursor::new(String::new()),
        || { panic!("--yes must not consult the terminal") }
    ));
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
    assert!(!ask("remove it?", ASK, &mut Broken, || true));
}
