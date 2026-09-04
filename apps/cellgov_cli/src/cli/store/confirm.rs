//! The confirmation a destructive command asks before it removes
//! anything.
//!
//! Prompting is for destruction only, and every prompt has a flag that
//! answers it. A run that cannot ask -- `--no-input`, or a stdin that
//! is not a terminal -- exits with a usage error naming that flag.

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

/// Whether to go ahead with the step `question` describes.
///
/// Both the prompt and the answer go to stderr.
pub(crate) fn confirm(question: &str, answers: Answers) -> bool {
    ask(question, answers, &mut std::io::stdin().lock(), || {
        std::io::stdin().is_terminal()
    })
}

/// [`confirm`] against an explicit input, so a test can drive an answer
/// without a terminal.
fn ask(
    question: &str,
    answers: Answers,
    input: &mut dyn BufRead,
    stdin_is_terminal: impl Fn() -> bool,
) -> bool {
    if answers.yes {
        return true;
    }
    if answers.no_input {
        die_usage(&format!(
            "{question}\nthis run may not prompt (--no-input); pass --yes to answer it, or \
             --dry-run to see the plan"
        ));
    }
    if !stdin_is_terminal() {
        die_usage(&format!(
            "{question}\nstdin is not a terminal, so this run cannot prompt; pass --yes to \
             answer it, or --dry-run to see the plan"
        ));
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
        return false;
    }
    let answer = line.trim().to_ascii_lowercase();
    answer == "y" || answer == "yes"
}

#[cfg(test)]
#[path = "tests/confirm_tests.rs"]
mod tests;
