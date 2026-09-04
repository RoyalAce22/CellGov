//! `docs/cli.md` from the decorated command tree.

use clap::{Arg, ArgAction, Command};

use super::{examples, schema};
use crate::cli::exit_codes::CONTRACT;

const CLI_TEMPLATE: &str = include_str!("../templates/cli.md.template");

/// The whole reference body.
///
/// The function builds the tree first. Before the build, a global flag
/// lives only on the root, so a subcommand section misses the arguments
/// it inherits.
pub(crate) fn render_doc(root: &Command) -> String {
    let mut root = root.clone();
    root.build();
    let mut sections = Vec::new();
    push_sections(&root, "cellgov", 2, &mut sections);
    crate::cli::fixture_gen::apply_subs(
        CLI_TEMPLATE,
        &[
            ("exit_codes", CONTRACT),
            ("global_options", &option_table(globals(&root))),
            ("json_documents", &schema::render()),
            ("commands", &sections.join("\n")),
        ],
    )
}

/// Render `cmd`, then each of its subcommands one heading level deeper.
///
/// `depth` is the markdown heading level, capped at `######` so a tree
/// deeper than four levels still renders valid markdown.
fn push_sections(cmd: &Command, path: &str, depth: usize, out: &mut Vec<String>) {
    out.push(section(cmd, path, depth));
    for sub in cmd.get_subcommands() {
        // A hidden subcommand is absent from `--help`, and the
        // reference documents what help documents.
        if sub.get_name() == "help" || sub.is_hide_set() {
            continue;
        }
        let child = depth.saturating_add(1);
        debug_assert!(
            child <= 6,
            "{path} {} nests past the deepest markdown heading, so its section \
             would share a level with its parent",
            sub.get_name()
        );
        push_sections(
            sub,
            &format!("{path} {}", sub.get_name()),
            child.min(6),
            out,
        );
    }
}

/// One command's section of `docs/cli.md`.
fn section(cmd: &Command, path: &str, depth: usize) -> String {
    let mut s = format!("{} `{path}`\n\n", "#".repeat(depth));

    debug_assert!(
        cmd.get_about().is_some(),
        "{path} renders a section with no description"
    );
    // `--help` prints the long about where a command declares one, so
    // the section would carry text the binary never shows.
    debug_assert!(
        cmd.get_long_about().is_none(),
        "{path} declares a long about the reference does not publish"
    );
    if let Some(about) = cmd.get_about() {
        s.push_str(&sentence(&about.to_string()));
        s.push_str("\n\n");
    }

    let key = path.strip_prefix("cellgov").unwrap_or(path).trim();
    if let Some(lines) = examples::lines_for(key) {
        s.push_str("```console\n");
        for line in lines {
            s.push_str("$ ");
            s.push_str(line);
            s.push('\n');
        }
        s.push_str("```\n\n");
    }

    s.push_str("```\n");
    s.push_str(usage_line(cmd).trim_end());
    s.push_str("\n```\n");

    let positionals: Vec<&Arg> = documented(cmd).filter(|a| a.is_positional()).collect();
    if !positionals.is_empty() {
        s.push_str("\n| Argument | Description |\n| --- | --- |\n");
        for arg in positionals {
            s.push_str(&format!(
                "| `{}` | {} |\n",
                cell(&value_slot(arg)),
                cell(&describe(arg))
            ));
        }
    }

    let options: Vec<&Arg> = documented(cmd).filter(|a| !a.is_positional()).collect();
    if !options.is_empty() {
        s.push_str(&option_table(options.into_iter()));
    }

    // The root's after-help is the shared contract, which the document
    // states once in its own section.
    if let Some(after) = cmd.get_after_help().map(ToString::to_string) {
        if after.trim() != CONTRACT.trim() {
            s.push_str("\n```\n");
            s.push_str(after.trim_end());
            s.push_str("\n```\n");
        }
    }

    s
}

/// The option table for `args`, including its own leading blank line.
fn option_table<'a>(args: impl Iterator<Item = &'a Arg>) -> String {
    let mut s = String::from("\n| Option | Value | Description |\n| --- | --- | --- |\n");
    for arg in args {
        let value = match arg.get_action().takes_values() {
            true => format!("`{}`", cell(&value_slot(arg))),
            false => "--".to_string(),
        };
        s.push_str(&format!(
            "| `{}` | {value} | {} |\n",
            flag_spelling(arg),
            cell(&describe(arg))
        ));
    }
    s
}

/// `cellgov`'s global flags, which every command accepts and only the
/// reference's own section lists.
fn globals(root: &Command) -> impl Iterator<Item = &Arg> {
    root.get_arguments()
        .filter(|a| a.is_global_set() && !is_builtin(a))
}

/// The arguments `cmd`'s own section lists.
fn documented(cmd: &Command) -> impl Iterator<Item = &Arg> {
    cmd.get_arguments()
        .filter(|a| !a.is_hide_set() && !a.is_global_set() && !is_builtin(a))
}

/// Whether `arg` is the `--help` or `--version` clap adds itself.
///
/// The check reads the action clap gives the two arguments it injects,
/// not the name. An argument the tree declares under either name still
/// reaches the tables.
fn is_builtin(arg: &Arg) -> bool {
    matches!(
        arg.get_action(),
        ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong | ArgAction::Version
    )
}

/// `-s, --long` for an option, or whichever spelling it has alone.
fn flag_spelling(arg: &Arg) -> String {
    match (arg.get_short(), arg.get_long()) {
        (Some(short), Some(long)) => format!("-{short}, --{long}"),
        (Some(short), None) => format!("-{short}"),
        (None, Some(long)) => format!("--{long}"),
        // A positional never reaches an option table, and its only
        // spelling is its id.
        (None, None) => arg.get_id().to_string(),
    }
}

/// The value placeholder clap prints for `arg`, without angle brackets.
fn value_slot(arg: &Arg) -> String {
    let slot = match arg.get_value_names() {
        Some(names) if !names.is_empty() => names
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(" "),
        // The id verbatim: clap itself prints the id for an argument
        // that names no value.
        _ => arg.get_id().to_string(),
    };
    // Every caller renders the slot inside a code span on one table row.
    debug_assert!(
        !slot.contains('`') && !slot.contains('\n'),
        "value slot {slot:?} does not survive the code span it is rendered in"
    );
    slot
}

/// The help sentence, plus the default and the accepted values where
/// clap holds them.
fn describe(arg: &Arg) -> String {
    // `--help` prints the long help where an argument declares one, so
    // the table cell would carry text the binary never shows.
    debug_assert!(
        arg.get_long_help().is_none(),
        "{} declares a long help the reference does not publish",
        arg.get_id()
    );
    let mut s = arg
        .get_help()
        .map(|h| sentence(&h.to_string()))
        .unwrap_or_default();
    let possible: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|v| !v.is_hide_set())
        .map(|v| format!("`{}`", v.get_name()))
        .collect();
    if !possible.is_empty() {
        s.push_str(&format!(" One of {}.", possible.join(", ")));
    }
    // A switch carries clap's implicit `false`, which says nothing an
    // absent flag does not already say.
    let defaults: Vec<String> = match arg.get_action().takes_values() {
        true => arg
            .get_default_values()
            .iter()
            .map(|v| format!("`{}`", v.to_string_lossy()))
            .collect(),
        false => Vec::new(),
    };
    if !defaults.is_empty() {
        s.push_str(&format!(" Default {}.", defaults.join(", ")));
    }
    if arg.is_required_set() {
        s.push_str(" Required.");
    }
    s.trim().to_string()
}

/// `cmd`'s usage line, prefixed the way clap's help prints it.
fn usage_line(cmd: &Command) -> String {
    // `render_usage` needs `&mut`, and the caller holds the tree by
    // reference for the whole walk.
    let mut owned = cmd.clone();
    owned.render_usage().to_string()
}

/// One markdown table cell: line breaks flattened, `\` and `|` escaped.
///
/// The backslash escape runs first. A pipe escape over an already
/// escaped `\|` leaves `\\|`: an escaped backslash, then a live cell
/// separator.
fn cell(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(['\n', '\r'], " ")
        .replace('|', "\\|")
}

/// `text` with a full stop, so table cells and section prose read as
/// sentences where clap's one-line help does not punctuate.
fn sentence(text: &str) -> String {
    let text = text.trim();
    match text.chars().last() {
        Some(last) if !".!?:".contains(last) => format!("{text}."),
        _ => text.to_string(),
    }
}

#[cfg(test)]
#[path = "tests/render_tests.rs"]
mod tests;
