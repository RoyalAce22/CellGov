//! `dev funcs`: print the OPD-derived function map for a main ELF or
//! PRX.
//!
//! Accepts the same input forms as `dev prx-imports`: a plaintext
//! ELF / PRX and, in a build with the `decrypt` feature, an APP-keyed
//! SCE wrapper or an NPDRM SELF (retail EBOOT). NPDRM titles resolve
//! their RAP from the standard vfs exdata directory by content id;
//! see [`decrypt_ppu_self_or_die`].
//! Human output is one row per function; `--json` emits the map for
//! tooling.

use cellgov_ppu::funcmap::{self, FunctionMap, FunctionName};

use crate::cli::exit::die;
use crate::cli::parse::FuncsArgs;
use crate::cli::self_load::{decrypt_ppu_self_or_die, load_file_or_die};
use crate::cli::title::resolve_ps3_vfs_root;

pub(crate) fn run(args: &FuncsArgs, vfs_flag: Option<&std::path::Path>) {
    let vfs_root = resolve_ps3_vfs_root(vfs_flag);
    let raw = load_file_or_die(&args.path);
    let elf = decrypt_ppu_self_or_die(&raw, &args.path, &vfs_root);
    let mut map =
        funcmap::build(&elf).unwrap_or_else(|e| die(&format!("funcs: {}: {e}", args.path)));
    resolve_nids(&mut map);
    if let Some(note) = truncation_note(&map) {
        eprintln!("{note}");
    }
    if args.json {
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
