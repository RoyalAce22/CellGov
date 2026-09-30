//! `cellgov dev relations-gen` and `cellgov dev relations-check` -- the SPU
//! sequence-relation catalog written as a reference, and a fused form's
//! result states checked against it.

use std::fmt::Write as _;
use std::path::PathBuf;

use cellgov_fuzz::spu::{
    check_fused_results, relation_catalog, FusedResult, FusedResultVerdict, CATALOG_JSON,
    CATALOG_MARKDOWN,
};

use super::exit::{CommandError, CommandExitCode};
use super::exit_codes;
use super::parse::{RelationsCheckArgs, RelationsGenArgs};

const DEFAULT_OUTPUT_DIR: &str = "docs";

pub(crate) fn generate(args: &RelationsGenArgs) -> Result<(), CommandError> {
    let dir = match args.output_dir.as_deref() {
        Some(dir) if dir.as_os_str().is_empty() => return Err(CommandError::failed(
            "relations-gen: --output-dir is empty; name the directory the catalog is written under",
        )),
        Some(dir) => dir.to_path_buf(),
        None => PathBuf::from(DEFAULT_OUTPUT_DIR),
    };
    let catalog = relation_catalog()
        .map_err(|error| CommandError::failed(format!("relations-gen: {error}")))?;
    for (name, body) in [
        (CATALOG_MARKDOWN, &catalog.markdown),
        (CATALOG_JSON, &catalog.json),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, body).map_err(|error| {
            CommandError::failed(format!("relations-gen: write {}: {error}", path.display()))
        })?;
        println!("relations-gen: wrote {}", path.display());
    }
    Ok(())
}

pub(crate) fn check(args: &RelationsCheckArgs) -> Result<CommandExitCode, CommandError> {
    let text = std::fs::read_to_string(&args.path).map_err(|error| {
        CommandError::failed(format!(
            "relations-check: read {}: {error}",
            args.path.display()
        ))
    })?;
    let results = check_fused_results(&text).map_err(|error| {
        CommandError::failed(format!("relations-check: {}: {error}", args.path.display()))
    })?;
    let (report, code) = outcome(&results);
    print!("{report}");
    Ok(code)
}

/// The report lines for `results`, and the status: 4 when any diverges.
fn outcome(results: &[FusedResult]) -> (String, CommandExitCode) {
    let mut out = String::new();
    let (mut matched, mut diverged, mut inapplicable) = (0, 0, 0);
    for result in results {
        let _ = write!(
            out,
            "relations-check: {} {:?}: ",
            result.name, result.relation
        );
        let _ = match &result.verdict {
            FusedResultVerdict::Match => {
                matched += 1;
                writeln!(out, "match")
            }
            FusedResultVerdict::Inapplicable => {
                inapplicable += 1;
                writeln!(
                    out,
                    "inapplicable, the start state is outside the precondition"
                )
            }
            FusedResultVerdict::Diverged {
                first_component,
                registers,
            } => {
                diverged += 1;
                let registers: Vec<String> = registers
                    .iter()
                    .map(|register| format!("r{register}"))
                    .collect();
                if registers.is_empty() {
                    writeln!(out, "diverges in {first_component:?}")
                } else {
                    writeln!(
                        out,
                        "diverges in {first_component:?} ({})",
                        registers.join(", ")
                    )
                }
            }
        };
    }
    let _ = writeln!(
        out,
        "relations-check: {} result state(s): {matched} match, {diverged} diverge, \
         {inapplicable} inapplicable",
        results.len()
    );
    let code = if diverged == 0 {
        CommandExitCode::SUCCESS
    } else {
        CommandExitCode::new(exit_codes::DIVERGED)
    };
    (out, code)
}

#[cfg(test)]
#[path = "tests/relations_tests.rs"]
mod tests;
