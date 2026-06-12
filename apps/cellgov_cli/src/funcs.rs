//! `funcs` subcommand: print the OPD-derived function map for a
//! main ELF or PRX.
//!
//! Accepts the same input forms as `dump-prx-imports`: a plaintext
//! ELF / PRX, or an SCE-wrapped SELF/SPRX decrypted via
//! `cellgov_firmware`. Human output is one row per function; `--json`
//! emits the map for tooling.

use cellgov_ppu::funcmap::{self, FunctionMap, FunctionName};
use cellgov_ps3_abi::elf::ELF_MAGIC;
use cellgov_ps3_abi::sce::SCE_MAGIC;

use crate::cli::exit::die;

const USAGE: &str = "cellgov_cli funcs <elf-path> [--json]";

#[derive(Debug)]
struct FuncsArgs<'a> {
    path: &'a str,
    json: bool,
}

fn parse_args(args: &[String]) -> Result<FuncsArgs<'_>, String> {
    let mut path: Option<&str> = None;
    let mut json = false;
    for arg in &args[2..] {
        match arg.as_str() {
            "--json" => json = true,
            flag if flag.starts_with("--") => {
                return Err(format!("funcs: unknown flag {flag}\n{USAGE}"));
            }
            positional => {
                if path.replace(positional).is_some() {
                    return Err(format!("funcs: more than one path argument\n{USAGE}"));
                }
            }
        }
    }
    let path = path.ok_or_else(|| format!("funcs: missing <elf-path>\n{USAGE}"))?;
    Ok(FuncsArgs { path, json })
}

pub(crate) fn run(args: &[String]) {
    let parsed = parse_args(args).unwrap_or_else(|msg| die(&msg));
    let path = std::path::Path::new(parsed.path);
    let elf = load_elf_bytes(path);
    let mut map =
        funcmap::build(&elf).unwrap_or_else(|e| die(&format!("funcs: {}: {e}", path.display())));
    resolve_nids(&mut map);
    if let Some(note) = truncation_note(&map) {
        eprintln!("{note}");
    }
    if parsed.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&render_json(&map)).expect("funcmap JSON is plain data")
        );
    } else {
        print!("{}", render_human(&map));
    }
}

/// Stderr note when the map is a prefix of reality (discovery hit
/// the span cap), `None` for a complete map.
fn truncation_note(map: &FunctionMap) -> Option<&'static str> {
    map.truncated
        .then_some("note: function discovery hit the span cap; output is a prefix")
}

/// Resolve NID-named spans to their symbol names via the workspace
/// NID table. Unknown NIDs keep the `nid_<hex>` rendering.
pub(crate) fn resolve_nids(map: &mut FunctionMap) {
    for span in &mut map.functions {
        if let FunctionName::Nid(nid) = span.name {
            if let Some((_module, symbol)) = cellgov_ps3_abi::nid::lookup(nid) {
                span.name = FunctionName::Known(symbol);
            }
        }
    }
}

/// Read `path` as plaintext ELF bytes, decrypting SCE wrappers.
fn load_elf_bytes(path: &std::path::Path) -> Vec<u8> {
    let raw = std::fs::read(path)
        .unwrap_or_else(|e| die(&format!("funcs: read {}: {e}", path.display())));
    if raw.len() >= 4 && raw[0..4] == SCE_MAGIC {
        return cellgov_firmware::sce::decrypt_self_to_elf(&raw).unwrap_or_else(|e| {
            die(&format!(
                "funcs: SCE decrypt of {} failed: {e}",
                path.display()
            ))
        });
    }
    if raw.len() >= 4 && raw[0..4] == ELF_MAGIC {
        return raw;
    }
    die(&format!(
        "funcs: {} has neither ELF nor SCE magic",
        path.display()
    ))
}

fn render_human(map: &FunctionMap) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<12}{:<12}{:<10}{:<12}name",
        "start", "end", "size", "origin"
    );
    for span in &map.functions {
        let _ = writeln!(
            out,
            "0x{:08x}  0x{:08x}  0x{:<6x}  {:<12}{}",
            span.start,
            span.end,
            span.end - span.start,
            span.origin.as_str(),
            span.display_name(),
        );
    }
    let _ = writeln!(out, "total: {} function(s)", map.functions.len());
    out
}

fn render_json(map: &FunctionMap) -> serde_json::Value {
    let functions: Vec<serde_json::Value> = map
        .functions
        .iter()
        .map(|span| {
            let mut obj = serde_json::json!({
                "start": span.start,
                "end": span.end,
                "size": span.end - span.start,
                "origin": span.origin.as_str(),
                "name": span.display_name().to_string(),
            });
            if let FunctionName::Nid(nid) = span.name {
                obj["nid"] = serde_json::json!(nid);
            }
            obj
        })
        .collect();
    serde_json::json!({
        "functions": functions,
        "truncated": map.truncated,
    })
}

#[cfg(test)]
#[path = "tests/funcs_tests.rs"]
mod tests;
