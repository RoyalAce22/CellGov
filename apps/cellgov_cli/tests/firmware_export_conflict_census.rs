//! Regenerate `docs/dev/audits/firmware_export_conflict_census.md` from
//! the installed firmware corpus. Run with:
//!
//! ```text
//! cargo test -p cellgov_cli --test firmware_export_conflict_census --release \
//!   -- --ignored regenerate_firmware_export_conflict_census
//! ```
//!
//! Answers one question: if the export table were keyed on
//! `(namespace, nid)` instead of `nid` alone, would every export in the
//! full install resolve to exactly one address?
//!
//! Walks `*.sprx` under `<firmware-dir>` and its `../internal` sibling,
//! decrypts and parses each, replays the loader's first-wins namespace
//! shadowing, and then counts collisions under both keys. The
//! shadowing replay mirrors `prx_loader::body::load_firmware_set`; if
//! that policy changes, this census changes with it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if std::fs::read_to_string(p.join("Cargo.toml")).is_ok_and(|t| t.contains("[workspace]")) {
            return p;
        }
        if !p.pop() {
            panic!(
                "workspace root not found above {}",
                env!("CARGO_MANIFEST_DIR")
            );
        }
    }
}

fn filename_is_safe(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// Panics on an unreadable directory or entry: an io failure here
/// would otherwise shrink the census silently and skew the verdict.
fn sprx_paths_in(dir: &PathBuf) -> Vec<PathBuf> {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    let mut v: Vec<PathBuf> = entries
        .map(|entry| {
            entry
                .unwrap_or_else(|e| panic!("read_dir entry under {}: {e}", dir.display()))
                .path()
        })
        .filter(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("sprx"))
        })
        .collect();
    v.sort();
    v
}

/// `(module label, library name)` -- who exports a NID.
type Exporters = BTreeSet<(String, String)>;
/// `(module label, vaddr)` -- where a keyed export lands.
type Sites = BTreeSet<(String, u32)>;

/// One export library as parsed, before any shadowing decision.
struct Lib {
    name: String,
    /// `(nid, vaddr)` in declaration order.
    funcs: Vec<(u32, u32)>,
}

struct Module {
    /// Label used throughout the report, e.g. `external/libaudio.sprx`.
    label: String,
    libs: Vec<Lib>,
}

#[test]
#[ignore = "regeneration: run with --ignored regenerate_firmware_export_conflict_census"]
fn regenerate_firmware_export_conflict_census() {
    let external = match std::env::var("CELLGOV_FIRMWARE_DIR") {
        Ok(s) => PathBuf::from(s),
        Err(_) => workspace_root().join("firmware/sys/external"),
    };
    assert!(
        external.is_dir(),
        "firmware dir {} not found; run `cellgov_install install` first or set CELLGOV_FIRMWARE_DIR",
        external.display()
    );
    let internal = external
        .parent()
        .expect("firmware dir always has a parent")
        .join("internal");

    let mut candidates: Vec<(&'static str, PathBuf)> = Vec::new();
    for p in sprx_paths_in(&external) {
        candidates.push(("external", p));
    }
    if internal.is_dir() {
        for p in sprx_paths_in(&internal) {
            candidates.push(("internal", p));
        }
    }

    let mut modules: Vec<Module> = Vec::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    // The census claims to cover the internal sibling; an absent
    // directory must show up in the report, not shrink it silently.
    if !internal.is_dir() {
        skipped.push((
            "internal/".to_string(),
            format!(
                "directory absent ({}); internal modules not censused",
                internal.display()
            ),
        ));
    }

    for (origin, path) in &candidates {
        let name = path
            .file_name()
            .expect("read_dir entry always has a name")
            .to_string_lossy()
            .into_owned();
        let label = format!("{origin}/{name}");
        if !filename_is_safe(&name) {
            skipped.push((label, "rejected charset (outside [A-Za-z0-9._-]+)".into()));
            continue;
        }
        let raw = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                skipped.push((label, format!("io: {e}")));
                continue;
            }
        };
        let elf = match cellgov_install::sce::decrypt_self_to_elf(&raw) {
            Ok(e) => e,
            Err(e) => {
                skipped.push((label, format!("decrypt: {e}")));
                continue;
            }
        };
        let parsed = match cellgov_ppu::sprx::parse_prx(&elf) {
            Ok(p) => p,
            Err(e) => {
                skipped.push((label, format!("parse: {e:?}")));
                continue;
            }
        };
        let libs = parsed
            .exports
            .iter()
            .map(|lib| Lib {
                name: lib.name.clone(),
                funcs: lib.functions.iter().map(|f| (f.nid, f.vaddr)).collect(),
            })
            .collect();
        modules.push(Module { label, libs });
    }

    // Replay the loader's first-wins namespace shadowing. body.rs keys
    // by the hashed namespace id and compares providers by module id;
    // this keys by name, which is the same partition for distinct
    // names and is readable in the report.
    let mut provider_of_namespace: BTreeMap<String, String> = BTreeMap::new();
    let mut shadowed: Vec<(String, String, String)> = Vec::new();
    let mut retained: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for m in &modules {
        for lib in &m.libs {
            match provider_of_namespace.get(&lib.name) {
                Some(first) if *first != m.label => {
                    shadowed.push((lib.name.clone(), first.clone(), m.label.clone()));
                }
                _ => {
                    provider_of_namespace.insert(lib.name.clone(), m.label.clone());
                    retained
                        .entry(m.label.clone())
                        .or_default()
                        .insert(lib.name.clone());
                }
            }
        }
    }

    // Key A: nid alone, over retained libs. A nid exported by two
    // different modules lands at two different OPDs, because each
    // module loads at its own base -- that is ConflictingExport.
    let mut by_nid: BTreeMap<u32, Exporters> = BTreeMap::new();
    // The same, ignoring shadowing. The delta between the two is the
    // set of conflicts shadowing already resolves, which is where a
    // conflict observed by hand can go missing from the post-shadow
    // tables.
    let mut by_nid_preshadow: BTreeMap<u32, Exporters> = BTreeMap::new();
    // Key B: (namespace, nid). A collision here is a nid exported
    // twice under one namespace name, which the re-key cannot fix.
    let mut by_ns_nid: BTreeMap<(String, u32), Sites> = BTreeMap::new();
    // Same module, same nid, two libraries: LoadedPrx.exports is a
    // BTreeMap<u32, u64>, so the second write silently wins today.
    let mut intra_module: BTreeMap<(String, u32), Sites> = BTreeMap::new();

    for m in &modules {
        let keep = retained.get(&m.label);
        let mut seen_in_module: BTreeMap<u32, BTreeSet<(String, u32)>> = BTreeMap::new();
        for lib in &m.libs {
            for &(nid, _) in &lib.funcs {
                by_nid_preshadow
                    .entry(nid)
                    .or_default()
                    .insert((m.label.clone(), lib.name.clone()));
            }
            if !keep.is_some_and(|k| k.contains(&lib.name)) {
                continue;
            }
            for &(nid, vaddr) in &lib.funcs {
                by_nid
                    .entry(nid)
                    .or_default()
                    .insert((m.label.clone(), lib.name.clone()));
                by_ns_nid
                    .entry((lib.name.clone(), nid))
                    .or_default()
                    .insert((m.label.clone(), vaddr));
                seen_in_module
                    .entry(nid)
                    .or_default()
                    .insert((lib.name.clone(), vaddr));
            }
        }
        for (nid, sites) in seen_in_module {
            let addrs: BTreeSet<u32> = sites.iter().map(|(_, v)| *v).collect();
            if sites.len() > 1 && addrs.len() > 1 {
                intra_module.insert((m.label.clone(), nid), sites);
            }
        }
    }

    let is_cross_module =
        |sites: &Exporters| sites.iter().map(|(m, _)| m).collect::<BTreeSet<_>>().len() > 1;

    let cross_module_nid_conflicts: Vec<(u32, &Exporters)> = by_nid
        .iter()
        .filter(|(_, sites)| is_cross_module(sites))
        .map(|(nid, sites)| (*nid, sites))
        .collect();

    // Conflicts that exist in the raw firmware but not in what the
    // export table sees, because shadowing dropped one exporter's
    // library first.
    let resolved_by_shadowing: Vec<(u32, &Exporters)> = by_nid_preshadow
        .iter()
        .filter(|(nid, sites)| {
            is_cross_module(sites) && !by_nid.get(nid).is_some_and(is_cross_module)
        })
        .map(|(nid, sites)| (*nid, sites))
        .collect();

    // Post-shadow, each namespace has exactly one provider, so a
    // same-namespace cross-module conflict cannot exist by
    // construction. Assert it rather than assume it -- if shadowing
    // ever stops being first-wins, this is where it shows up.
    let same_namespace_conflicts: Vec<(u32, &Exporters)> = cross_module_nid_conflicts
        .iter()
        .filter(|(_, sites)| {
            sites.iter().map(|(_, l)| l).collect::<BTreeSet<_>>().len() < sites.len()
        })
        .map(|(nid, sites)| (*nid, *sites))
        .collect();

    let unresolvable: Vec<((String, u32), &Sites)> = by_ns_nid
        .iter()
        .filter(|(_, sites)| {
            sites.iter().map(|(_, v)| v).collect::<BTreeSet<_>>().len() > 1
                || sites.iter().map(|(m, _)| m).collect::<BTreeSet<_>>().len() > 1
        })
        .map(|(k, sites)| (k.clone(), sites))
        .collect();

    let world = if !unresolvable.is_empty() || !same_namespace_conflicts.is_empty() {
        "2 -- re-key is INSUFFICIENT; a precedence rule is required"
    } else if cross_module_nid_conflicts.is_empty() {
        "0 -- no cross-module NID conflicts at all; the re-key is unnecessary for this corpus"
    } else {
        "1 -- re-key is SUFFICIENT"
    };

    let mut out = String::new();
    writeln!(out, "# Firmware export-conflict census").expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "Generated by `cargo test -p cellgov_cli --test firmware_export_conflict_census --release -- --ignored regenerate_firmware_export_conflict_census`. The test writes this file directly; no stdout redirection."
    )
    .expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "Source: every `*.sprx` under the firmware directory (default `firmware/sys/external`, overridable via `CELLGOV_FIRMWARE_DIR`) and its `internal` sibling. Each module is SCE-decrypted and parsed, then the loader's first-wins namespace shadowing is replayed so the counts describe what `FirmwareExportTable::build` would actually see."
    )
    .expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "Modules parsed: {} of {} candidates ({} skipped).",
        modules.len(),
        candidates.len(),
        skipped.len()
    )
    .expect("write to String");
    writeln!(out).expect("write to String");

    writeln!(out, "## Verdict").expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(out, "**World {world}.**").expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(out, "| Measurement | Count |").expect("write to String");
    writeln!(out, "|---|---:|").expect("write to String");
    writeln!(out, "| Shadowed export libraries | {} |", shadowed.len()).expect("write to String");
    writeln!(
        out,
        "| Cross-module NID conflicts before shadowing | {} |",
        by_nid_preshadow
            .values()
            .filter(|s| is_cross_module(s))
            .count()
    )
    .expect("write to String");
    writeln!(
        out,
        "| ... already resolved by shadowing | {} |",
        resolved_by_shadowing.len()
    )
    .expect("write to String");
    writeln!(
        out,
        "| Cross-module NID conflicts (key = nid) | {} |",
        cross_module_nid_conflicts.len()
    )
    .expect("write to String");
    writeln!(
        out,
        "| ... of those, under the same namespace | {} |",
        same_namespace_conflicts.len()
    )
    .expect("write to String");
    writeln!(
        out,
        "| Collisions surviving key = (namespace, nid) | {} |",
        unresolvable.len()
    )
    .expect("write to String");
    writeln!(
        out,
        "| Intra-module NID duplicates across libraries | {} |",
        intra_module.len()
    )
    .expect("write to String");
    writeln!(out).expect("write to String");

    writeln!(out, "## Shadowed export libraries").expect("write to String");
    writeln!(out).expect("write to String");
    if shadowed.is_empty() {
        writeln!(out, "None.").expect("write to String");
    } else {
        writeln!(out, "| Library | Kept from | Dropped from |").expect("write to String");
        writeln!(out, "|---|---|---|").expect("write to String");
        for (lib, first, second) in &shadowed {
            writeln!(out, "| `{lib}` | `{first}` | `{second}` |").expect("write to String");
        }
    }
    writeln!(out).expect("write to String");

    writeln!(out, "## Cross-module NID conflicts").expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "Each row is a NID exported by two or more distinct modules. Under today's NID-only key the first module in dependency order wins and the rest are `ConflictingExport`; under a `(namespace, nid)` key each resolves to its own exporter."
    )
    .expect("write to String");
    writeln!(out).expect("write to String");
    if cross_module_nid_conflicts.is_empty() {
        writeln!(out, "None.").expect("write to String");
    } else {
        writeln!(out, "| NID | Exported by (module :: library) |").expect("write to String");
        writeln!(out, "|---|---|").expect("write to String");
        for (nid, sites) in &cross_module_nid_conflicts {
            let rendered = sites
                .iter()
                .map(|(m, l)| format!("`{m}` :: `{l}`"))
                .collect::<Vec<_>>()
                .join("<br>");
            writeln!(out, "| `0x{nid:08x}` | {rendered} |").expect("write to String");
        }
    }
    writeln!(out).expect("write to String");

    writeln!(out, "## Conflicts already resolved by shadowing").expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "These NIDs collide in the raw firmware but not in what the export table sees: one exporter's library was dropped as a duplicate publisher first. A conflict found by inspecting two modules by hand can land here and appear to be missing from the tables above."
    )
    .expect("write to String");
    writeln!(out).expect("write to String");
    if resolved_by_shadowing.is_empty() {
        writeln!(out, "None.").expect("write to String");
    } else {
        writeln!(out, "| NID | Raw exporters (module :: library) |").expect("write to String");
        writeln!(out, "|---|---|").expect("write to String");
        for (nid, sites) in &resolved_by_shadowing {
            let rendered = sites
                .iter()
                .map(|(m, l)| format!("`{m}` :: `{l}`"))
                .collect::<Vec<_>>()
                .join("<br>");
            writeln!(out, "| `0x{nid:08x}` | {rendered} |").expect("write to String");
        }
    }
    writeln!(out).expect("write to String");

    writeln!(out, "## Collisions surviving the (namespace, nid) key").expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "A row here is a NID exported more than once under a single namespace name at differing addresses. The re-key cannot separate these; they would need a precedence rule."
    )
    .expect("write to String");
    writeln!(out).expect("write to String");
    if unresolvable.is_empty() {
        writeln!(out, "None.").expect("write to String");
    } else {
        writeln!(out, "| Namespace | NID | Sites (module :: vaddr) |").expect("write to String");
        writeln!(out, "|---|---|---|").expect("write to String");
        for ((ns, nid), sites) in &unresolvable {
            let rendered = sites
                .iter()
                .map(|(m, v)| format!("`{m}` :: `0x{v:08x}`"))
                .collect::<Vec<_>>()
                .join("<br>");
            writeln!(out, "| `{ns}` | `0x{nid:08x}` | {rendered} |").expect("write to String");
        }
    }
    writeln!(out).expect("write to String");

    writeln!(out, "## Intra-module NID duplicates").expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "One module exporting the same NID from two libraries at different addresses. `LoadedPrx.exports` is a `BTreeMap<u32, u64>`, so today the later library silently overwrites the earlier one. A `(namespace, nid)` key preserves both."
    )
    .expect("write to String");
    writeln!(out).expect("write to String");
    if intra_module.is_empty() {
        writeln!(out, "None.").expect("write to String");
    } else {
        writeln!(out, "| Module | NID | Libraries (library :: vaddr) |").expect("write to String");
        writeln!(out, "|---|---|---|").expect("write to String");
        for ((m, nid), sites) in &intra_module {
            let rendered = sites
                .iter()
                .map(|(l, v)| format!("`{l}` :: `0x{v:08x}`"))
                .collect::<Vec<_>>()
                .join("<br>");
            writeln!(out, "| `{m}` | `0x{nid:08x}` | {rendered} |").expect("write to String");
        }
    }
    writeln!(out).expect("write to String");

    if !skipped.is_empty() {
        writeln!(out, "## Skipped").expect("write to String");
        writeln!(out).expect("write to String");
        writeln!(out, "| File | Reason |").expect("write to String");
        writeln!(out, "|---|---|").expect("write to String");
        for (name, why) in &skipped {
            writeln!(out, "| `{name}` | {why} |").expect("write to String");
        }
        writeln!(out).expect("write to String");
    }

    let dst = workspace_root().join("docs/dev/audits/firmware_export_conflict_census.md");
    std::fs::create_dir_all(dst.parent().expect("audits path always has a parent"))
        .expect("create audits dir");
    std::fs::write(&dst, out).expect("write census doc");
}
