//! Builds the operator-local oracle dispatch-gap overlay.

use std::path::Path;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::paths::workspace_root;

/// Writes the source revision and each unbound table slot.
///
/// # Errors
///
/// Returns an error if:
///
/// - Git cannot read the checkout revision.
/// - The command cannot read the dispatch table.
/// - The command cannot write the overlay.
pub(crate) fn run(vfs_flag: Option<&Path>) -> Result<CommandExitCode, CommandError> {
    let root = vfs_flag.unwrap_or_else(|| Path::new("vfs"));
    let checkout = ["rpc", "s3-src"].concat();
    let source = workspace_root()
        .join("tools")
        .join(checkout)
        .join(["rpc", "s3/Emu/Cell/lv2/lv2.cpp"].concat());
    if !source.exists() {
        println!("oracle gap: not computed -- local oracle checkout is unavailable");
        return Ok(CommandExitCode::SUCCESS);
    }
    let revision = std::process::Command::new("git")
        .args([
            "-C",
            source
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .and_then(Path::parent)
                .and_then(Path::parent)
                .expect("oracle source has repository parents")
                .to_str()
                .expect("checkout path is UTF-8"),
            "rev-parse",
            "HEAD",
        ])
        .output()
        .map_err(|error| {
            CommandError::failed(format!("oracle gap: read checkout revision: {error}"))
        })?;
    if !revision.status.success() {
        return Err(CommandError::failed(
            "oracle gap: checkout has no readable revision",
        ));
    }
    let table = std::fs::read_to_string(&source).map_err(|error| {
        CommandError::failed(format!("oracle gap: read dispatch table: {error}"))
    })?;
    let mut ordinals = std::collections::BTreeSet::new();
    for line in table.lines() {
        let Some(comment) = line.split("//").nth(1) else {
            continue;
        };
        let digits: String = comment
            .chars()
            .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
            .collect();
        let Some((first, last)) = digits.split_once('-').map_or_else(
            || digits.parse::<u64>().ok().map(|value| (value, value)),
            |(first, last)| Some((first.parse::<u64>().ok()?, last.parse::<u64>().ok()?)),
        ) else {
            continue;
        };
        if !line.contains("BIND_SYSC") {
            ordinals.extend(first..=last);
        }
    }
    let out = root.join(".cellgov/oracle-gap.tsv");
    std::fs::create_dir_all(out.parent().expect("overlay has parent"))
        .map_err(|error| CommandError::failed(format!("oracle gap: create overlay: {error}")))?;
    let mut text = format!(
        "revision\t{}\nordinal\n",
        String::from_utf8_lossy(&revision.stdout).trim()
    );
    for ordinal in ordinals {
        text.push_str(&format!("{ordinal}\n"));
    }
    std::fs::write(&out, text)
        .map_err(|error| CommandError::failed(format!("oracle gap: write overlay: {error}")))?;
    println!("oracle gap: wrote {}", out.display());
    Ok(CommandExitCode::SUCCESS)
}
