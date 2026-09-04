//! The reference body against the tree it renders.

use super::*;
use crate::cli::reference::command_tree;

fn body() -> String {
    render_doc(&command_tree())
}

#[test]
fn every_command_in_the_tree_has_a_section() {
    let body = body();
    let headings: Vec<&str> = body
        .lines()
        .filter(|l| l.starts_with('#'))
        .map(|l| l.trim_start_matches(['#', ' ']))
        .collect();
    let mut paths = Vec::new();
    collect_paths(&command_tree(), "cellgov", &mut paths);
    assert!(
        paths.len() > 30,
        "the walk found only {} commands",
        paths.len()
    );
    for path in paths {
        let heading = format!("`{path}`");
        assert!(
            headings.contains(&heading.as_str()),
            "no section heading for {path}"
        );
    }
}

fn collect_paths(cmd: &clap::Command, path: &str, out: &mut Vec<String>) {
    out.push(path.to_string());
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" {
            continue;
        }
        collect_paths(sub, &format!("{path} {}", sub.get_name()), out);
    }
}

#[test]
fn no_placeholder_survives_the_substitution() {
    assert!(!body().contains("{{"), "an unfilled template key remains");
}

#[test]
fn the_shared_exit_code_contract_is_the_one_help_prints() {
    assert!(body().contains(crate::cli::exit_codes::CONTRACT));
}

#[test]
fn a_commands_own_exit_codes_reach_its_section() {
    assert!(
        body().contains("11  the step cap was reached before the checkpoint"),
        "boot run's own exit codes are missing"
    );
}

#[test]
fn every_global_option_is_listed_once_and_not_per_command() {
    let body = body();
    let mut root = command_tree();
    root.build();
    let rows: Vec<String> = globals(&root)
        .map(|arg| format!("| `{}` |", flag_spelling(arg)))
        .collect();
    assert!(
        rows.len() > 4,
        "the tree declares only {} global options",
        rows.len()
    );
    for row in rows {
        assert_eq!(
            body.matches(&row).count(),
            1,
            "{row} is not listed exactly once"
        );
    }
}

#[test]
fn a_commands_flags_reach_its_section() {
    let body = body();
    for flag in ["| `--runs` |", "| `--strict-perf` |", "| `--symbolize` |"] {
        assert!(body.contains(flag), "{flag} missing");
    }
}

#[test]
fn a_positional_reaches_its_arguments_table() {
    assert!(body().contains("| Argument | Description |"));
}

#[test]
fn clap_builtins_are_left_out_of_the_tables() {
    let body = body();
    let rows: Vec<&str> = body.lines().filter(|l| l.starts_with("| `")).collect();
    assert!(
        rows.len() > 100,
        "the render produced only {} table rows",
        rows.len()
    );
    for row in rows {
        let named = row.split('`').nth(1).unwrap_or_default();
        assert!(
            !named.contains("--help") && !named.contains("--version"),
            "a clap built-in is tabulated: {row}"
        );
    }
}

#[test]
fn the_terminal_caveats_are_stated_once() {
    let body = body();
    assert_eq!(body.matches("no SIGINT handler").count(), 1);
    assert_eq!(
        body.matches("render thread is presentation-only").count(),
        1
    );
}

#[test]
fn rendering_is_byte_identical_across_two_invocations() {
    assert_eq!(body(), body());
}

#[test]
fn a_default_value_reaches_the_description() {
    assert!(
        body().contains("Default `3`."),
        "boot bench --runs should carry its default"
    );
}

#[test]
fn an_enum_flags_accepted_values_reach_the_description() {
    assert!(body().contains("One of `human`, `json`."), "{}", body());
}

#[test]
fn a_pipe_in_a_value_name_is_escaped_rather_than_splitting_the_row() {
    let body = body();
    let rows: Vec<&str> = body
        .lines()
        .filter(|l| l.starts_with("| `--game-ver`"))
        .collect();
    assert!(!rows.is_empty(), "no --game-ver row was rendered");
    for row in rows {
        assert!(row.contains("base\\|VERSION"), "unescaped pipe: {row}");
        assert_eq!(
            row.matches('|').count() - row.matches("\\|").count(),
            4,
            "row does not hold exactly three cells: {row}"
        );
    }
}

#[test]
fn a_switch_does_not_advertise_clap_s_implicit_false_default() {
    let body = body();
    let switches: Vec<&str> = body
        .lines()
        .filter(|l| l.starts_with("| `") && l.contains("| -- |"))
        .collect();
    assert!(
        switches.len() > 10,
        "the render produced only {} switch rows",
        switches.len()
    );
    for row in switches {
        assert!(
            !row.contains("Default"),
            "a switch advertises a default it cannot take: {row}"
        );
    }
}

#[test]
fn every_declared_example_reaches_the_reference() {
    let body = body();
    let mut checked = 0usize;
    for entry in crate::cli::reference::examples::EXAMPLES {
        for line in entry.lines {
            assert!(
                body.contains(&format!("\n$ {line}\n")),
                "{line:?} is declared but reaches no section of the reference"
            );
            checked += 1;
        }
    }
    assert!(checked > 30, "only {checked} example lines were checked");
}

#[test]
fn the_shared_contract_is_not_repeated_under_the_root_command() {
    assert_eq!(body().matches(">=10 an outcome particular").count(), 1);
}

#[test]
fn cell_escapes_a_pipe_and_flattens_a_newline() {
    assert_eq!(cell("a|b\nc"), "a\\|b c");
}

#[test]
fn sentence_adds_a_full_stop_only_where_one_is_missing() {
    assert_eq!(sentence("no stop"), "no stop.");
    assert_eq!(sentence("has one."), "has one.");
    assert_eq!(sentence("a question?"), "a question?");
}
