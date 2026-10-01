//! The confirmation a destructive command asks before it removes
//! anything.
//!
//! Prompting is for destruction only, and every prompt has a flag that
//! answers it. A run that cannot ask -- `--no-input`, or a stdin that
//! is not a terminal -- is a usage error naming that flag and the
//! caller's other way out.

use std::io::{BufRead, IsTerminal, Write};

use crate::cli::parse::die_usage;

/// How the invocation answers a confirmation without being asked.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Answers {
    /// `--yes`: answer every prompt yes.
    pub yes: bool,
    /// `--no-input`: ask no prompt.
    pub no_input: bool,
}

/// A confirmation the run cannot ask: the question, why it cannot ask,
/// and the flags that answer it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct CannotAsk(String);

/// Whether to go ahead with the step `question` describes, exiting
/// with the usage status when the run cannot ask. `instead` names the
/// way out other than `--yes`.
///
/// Both the prompt and the answer go to stderr.
pub(crate) fn confirm(question: &str, answers: Answers, instead: &str) -> bool {
    decide(question, answers, instead).unwrap_or_else(|usage| die_usage(&usage.to_string()))
}

/// [`confirm`], returning the usage message for a run that cannot ask
/// instead of exiting with it.
///
/// # Errors
///
/// [`CannotAsk`] under `--no-input` or a stdin that is not a terminal.
pub(crate) fn decide(question: &str, answers: Answers, instead: &str) -> Result<bool, CannotAsk> {
    ask(
        question,
        answers,
        instead,
        &mut std::io::stdin().lock(),
        || std::io::stdin().is_terminal(),
    )
}

/// [`decide`] against an explicit input, so a test can drive an answer
/// without a terminal.
fn ask(
    question: &str,
    answers: Answers,
    instead: &str,
    input: &mut dyn BufRead,
    stdin_is_terminal: impl Fn() -> bool,
) -> Result<bool, CannotAsk> {
    if answers.yes {
        return Ok(true);
    }
    if answers.no_input {
        return Err(CannotAsk(format!(
            "{question}\nthis run may not prompt (--no-input); pass --yes to answer it, or \
             {instead}"
        )));
    }
    if !stdin_is_terminal() {
        return Err(CannotAsk(format!(
            "{question}\nstdin is not a terminal, so this run cannot prompt; pass --yes to \
             answer it, or {instead}"
        )));
    }
    eprint!("{question} [y/N] ");
    // stderr is unbuffered, so the prompt is already on screen and a
    // failed flush cannot hide it.
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    if let Err(e) = input.read_line(&mut line) {
        // Declining an unreadable answer is safe, but a silent decline
        // reads as an operator who said no.
        eprintln!("cellgov: reading the answer failed ({e}); taking it as no");
        return Ok(false);
    }
    let answer = line.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

#[cfg(test)]
#[path = "tests/confirm_tests.rs"]
mod tests;
