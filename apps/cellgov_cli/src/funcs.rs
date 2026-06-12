//! `funcs` subcommand: print the OPD-derived function map for a
//! main ELF or PRX.
//!
//! Accepts the same input forms as `dump-prx-imports`: a plaintext
//! ELF / PRX, an APP-keyed SCE wrapper, or an NPDRM SELF (retail
//! EBOOT). NPDRM titles resolve their RAP from the standard vfs
//! exdata directory by content id; see [`decrypt_ppu_self_or_die`].
//! Human output is one row per function; `--json` emits the map for
//! tooling.

use cellgov_ppu::funcmap::{self, FunctionMap, FunctionName};

use crate::cli::exit::{decrypt_ppu_self_or_die, die, load_file_or_die};
use crate::cli::title::resolve_ps3_vfs_root;

const USAGE: &str = "cellgov_cli funcs <elf-path> [--json] [--vfs-root PATH]\n\
     \t(NPDRM EBOOTs resolve their RAP from <vfs-root>/home/00000001/exdata/;\n\
     \t vfs-root defaults to CELLGOV_PS3_VFS_ROOT, then tools/rpcs3/dev_hdd0)";

#[derive(Debug)]
struct FuncsArgs<'a> {
    path: &'a str,
    json: bool,
}

fn parse_args(args: &[String]) -> Result<FuncsArgs<'_>, String> {
    let mut path: Option<&str> = None;
    let mut json = false;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = true;
                i += 1;
            }
            // Consumed here so it is not rejected as unknown; the
            // value is re-read by `resolve_ps3_vfs_root`.
            "--vfs-root" => {
                if args.get(i + 1).is_none() {
                    return Err(format!("funcs: --vfs-root requires a path\n{USAGE}"));
                }
                i += 2;
            }
            flag if flag.starts_with("--") => {
                return Err(format!("funcs: unknown flag {flag}\n{USAGE}"));
            }
            positional => {
                if path.replace(positional).is_some() {
                    return Err(format!("funcs: more than one path argument\n{USAGE}"));
                }
                i += 1;
            }
        }
    }
    let path = path.ok_or_else(|| format!("funcs: missing <elf-path>\n{USAGE}"))?;
    Ok(FuncsArgs { path, json })
}

pub(crate) fn run(args: &[String]) {
    let parsed = parse_args(args).unwrap_or_else(|msg| die(&msg));
    let vfs_root = resolve_ps3_vfs_root(args);
    let raw = load_file_or_die(parsed.path);
    let elf = decrypt_ppu_self_or_die(&raw, parsed.path, &vfs_root);
    let mut map =
        funcmap::build(&elf).unwrap_or_else(|e| die(&format!("funcs: {}: {e}", parsed.path)));
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
