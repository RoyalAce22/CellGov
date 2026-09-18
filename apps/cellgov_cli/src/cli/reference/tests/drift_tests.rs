//! The committed `docs/cli.md` against the generator.

use std::path::Path;

use super::*;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/cellgov_cli has a workspace root two levels up")
}

/// Stands in for a region that switches on the `decrypt` feature.
///
/// Help strings and one command switch on that feature, so the committed
/// document cannot be byte-exact in both builds. The gate replaces or
/// removes those regions and still compares every other byte.
const FEATURE_DEPENDENT: &str = "<feature-dependent>";

/// First line of the note that `cellgov_cli::cli::exit` spells one way
/// with `decrypt` and another way without.
const SCE_NOTE_HEADING: &str = "SCE-wrapped input:";

/// The `--vfs-root` row, whose description names the key vault only in
/// a build that reads one.
const VFS_ROOT_ROW: &str = "| `--vfs-root` |";

const FEATURE_COMMAND_HEADINGS: &[&str] = &[
    "#### `cellgov dev lv2-extract`",
    "#### `cellgov dev caller-census`",
];

const KERNEL_ONLY_EXIT_LINE: &str =
    "  42  --kernel-only completed, but the PUP yielded no stored kernel";

/// Normalizes line endings and blanks the feature-dependent regions.
fn normalize(text: &str) -> String {
    let mut out = String::new();
    let mut in_note = false;
    let mut in_feature_command = false;
    let mut skip_kernel_only_tail = 0u8;
    for line in text.replace("\r\n", "\n").lines() {
        if skip_kernel_only_tail > 0 {
            skip_kernel_only_tail -= 1;
            continue;
        }
        if line == KERNEL_ONLY_EXIT_LINE {
            const PREFIX: &str = "```\nExit codes particular to this command:\n";
            assert!(out.ends_with(PREFIX));
            out.truncate(out.len() - PREFIX.len());
            // Skip the closing fence and the blank line after it. The
            // blank line before the opening fence already remains.
            skip_kernel_only_tail = 2;
            continue;
        }
        if FEATURE_COMMAND_HEADINGS.contains(&line) {
            in_feature_command = true;
            continue;
        }
        if in_feature_command {
            if !line.starts_with("#### `") {
                continue;
            }
            in_feature_command = false;
        }
        if line == SCE_NOTE_HEADING {
            in_note = true;
            out.push_str(FEATURE_DEPENDENT);
            out.push('\n');
            continue;
        }
        // The note's own body is indented; the first unindented line
        // ends it.
        if in_note {
            if line.starts_with("  ") {
                continue;
            }
            in_note = false;
        }
        if line.starts_with(VFS_ROOT_ROW) {
            out.push_str(FEATURE_DEPENDENT);
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[test]
fn normalization_removes_the_feature_only_command() {
    let rendered = render_doc(&command_tree());
    for heading in FEATURE_COMMAND_HEADINGS {
        if cfg!(feature = "decrypt") {
            assert!(rendered.contains(heading));
            assert!(!normalize(&rendered).contains(heading));
        } else {
            assert!(!rendered.contains(heading));
        }
    }
}

fn committed_doc() -> String {
    let path = repo_root().join("docs/cli.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn committed_cli_doc_matches_generator() {
    assert_eq!(
        normalize(&committed_doc()),
        normalize(&render_doc(&command_tree())),
        "docs/cli.md is stale; regenerate with:\n  \
         cargo run --release -p cellgov_cli -- dev cli-gen"
    );
}

/// The committed document is generated without `decrypt`, so only a
/// feature build needs the blanking above.
#[cfg(not(feature = "decrypt"))]
#[test]
fn a_default_build_matches_the_committed_document_byte_for_byte() {
    assert_eq!(
        committed_doc().replace("\r\n", "\n"),
        render_doc(&command_tree()).replace("\r\n", "\n"),
    );
}

#[test]
fn the_feature_dependent_regions_are_found_in_the_committed_document() {
    let committed = committed_doc();
    assert!(
        committed.contains(SCE_NOTE_HEADING),
        "{SCE_NOTE_HEADING} names no region"
    );
    assert!(
        committed.contains(VFS_ROOT_ROW),
        "{VFS_ROOT_ROW} names no row"
    );
    let blanked = normalize(&committed).matches(FEATURE_DEPENDENT).count();
    assert!(
        blanked >= 5,
        "the gate blanked {blanked} regions; a rename would make it a byte gate again \
         under one feature set and a silent pass under the other"
    );
}

#[test]
fn drift_gate_sees_a_changed_flag_description() {
    let renamed = render_doc(&command_tree().mut_arg("quiet", |a| a.help("something else")));
    assert_ne!(normalize(&renamed), normalize(&render_doc(&command_tree())));
}

#[test]
fn drift_gate_still_sees_a_change_next_to_a_feature_dependent_region() {
    let changed = render_doc(&command_tree().mut_arg("no_color", |a| a.help("something else")));
    assert_ne!(normalize(&changed), normalize(&render_doc(&command_tree())));
}

/// Every subcommand name in the tree, without clap's own `help`.
fn command_names() -> Vec<String> {
    let mut out = Vec::new();
    collect_names(&command_tree(), &mut out);
    out
}

fn collect_names(cmd: &clap::Command, out: &mut Vec<String>) {
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" {
            continue;
        }
        out.push(sub.get_name().to_string());
        collect_names(sub, out);
    }
}

#[test]
fn completions_are_produced_for_every_declared_shell() {
    use crate::cli::parse::CompletionShell;
    let names = command_names();
    assert!(
        names.len() > 40,
        "the tree declares {} commands",
        names.len()
    );
    for shell in [
        CompletionShell::Bash,
        CompletionShell::Zsh,
        CompletionShell::Pwsh,
    ] {
        let mut out = Vec::new();
        clap_complete::generate(
            crate::cli::cli_gen::generator(shell),
            &mut command_tree(),
            "cellgov",
            &mut out,
        );
        let script = String::from_utf8(out).expect("completion scripts are UTF-8");
        for name in &names {
            assert!(
                script.contains(name.as_str()),
                "{shell:?} script names no {name} verb"
            );
        }
        assert!(
            script.contains("--no-anchor-check"),
            "{shell:?} script names no leaf flag"
        );
    }
}
