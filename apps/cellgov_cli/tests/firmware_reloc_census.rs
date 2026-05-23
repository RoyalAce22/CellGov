//! Regenerate `docs/dev/firmware_reloc_census.md` from the installed
//! firmware corpus. Run with:
//!
//! ```text
//! cargo test -p cellgov_cli --test firmware_reloc_census --release \
//!   -- --ignored regenerate_firmware_reloc_census
//! ```
//!
//! Iterates `<firmware-dir>/*.sprx` (env `CELLGOV_FIRMWARE_DIR`,
//! default `firmware/sys/external`), decrypts and parses each,
//! and writes the per-PRX type table plus the union. The
//! "Applier covered?" column reads
//! [`cellgov_ppu::sprx::APPLIER_SUPPORTED_TYPES`] so the doc cannot
//! disagree silently with the applier.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

fn reloc_type_name(rtype: u32) -> &'static str {
    match rtype {
        0 => "R_PPC64_NONE",
        1 => "R_PPC64_ADDR32",
        2 => "R_PPC64_ADDR24",
        3 => "R_PPC64_ADDR16",
        4 => "R_PPC64_ADDR16_LO",
        5 => "R_PPC64_ADDR16_HI",
        6 => "R_PPC64_ADDR16_HA",
        7 => "R_PPC64_ADDR14",
        8 => "R_PPC64_ADDR14_BRTAKEN",
        9 => "R_PPC64_ADDR14_BRNTAKEN",
        10 => "R_PPC64_REL24",
        11 => "R_PPC64_REL14",
        12 => "R_PPC64_REL14_BRTAKEN",
        13 => "R_PPC64_REL14_BRNTAKEN",
        14 => "R_PPC64_GOT16",
        15 => "R_PPC64_GOT16_LO",
        16 => "R_PPC64_GOT16_HI",
        17 => "R_PPC64_GOT16_HA",
        19 => "R_PPC64_COPY",
        20 => "R_PPC64_GLOB_DAT",
        21 => "R_PPC64_JMP_SLOT",
        22 => "R_PPC64_RELATIVE",
        24 => "R_PPC64_UADDR32",
        25 => "R_PPC64_UADDR16",
        26 => "R_PPC64_REL32",
        27 => "R_PPC64_PLT32",
        28 => "R_PPC64_PLTREL32",
        29 => "R_PPC64_PLT16_LO",
        30 => "R_PPC64_PLT16_HI",
        31 => "R_PPC64_PLT16_HA",
        33 => "R_PPC64_SECTOFF",
        34 => "R_PPC64_SECTOFF_LO",
        35 => "R_PPC64_SECTOFF_HI",
        36 => "R_PPC64_SECTOFF_HA",
        37 => "R_PPC64_ADDR30",
        38 => "R_PPC64_ADDR64",
        39 => "R_PPC64_ADDR16_HIGHER",
        40 => "R_PPC64_ADDR16_HIGHERA",
        41 => "R_PPC64_ADDR16_HIGHEST",
        42 => "R_PPC64_ADDR16_HIGHESTA",
        43 => "R_PPC64_UADDR64",
        44 => "R_PPC64_REL64",
        45 => "R_PPC64_PLT64",
        46 => "R_PPC64_PLTREL64",
        47 => "R_PPC64_TOC16",
        48 => "R_PPC64_TOC16_LO",
        49 => "R_PPC64_TOC16_HI",
        50 => "R_PPC64_TOC16_HA",
        51 => "R_PPC64_TOC",
        52 => "R_PPC64_PLTGOT16",
        53 => "R_PPC64_PLTGOT16_LO",
        54 => "R_PPC64_PLTGOT16_HI",
        55 => "R_PPC64_PLTGOT16_HA",
        56 => "R_PPC64_ADDR16_DS",
        57 => "R_PPC64_ADDR16_LO_DS",
        58 => "R_PPC64_GOT16_DS",
        59 => "R_PPC64_GOT16_LO_DS",
        60 => "R_PPC64_PLT16_LO_DS",
        61 => "R_PPC64_SECTOFF_DS",
        62 => "R_PPC64_SECTOFF_LO_DS",
        63 => "R_PPC64_TOC16_DS",
        64 => "R_PPC64_TOC16_LO_DS",
        65 => "R_PPC64_PLTGOT16_DS",
        66 => "R_PPC64_PLTGOT16_LO_DS",
        67 => "R_PPC64_TLS",
        68 => "R_PPC64_DTPMOD64",
        69 => "R_PPC64_TPREL16",
        70 => "R_PPC64_TPREL16_LO",
        71 => "R_PPC64_TPREL16_HI",
        72 => "R_PPC64_TPREL16_HA",
        73 => "R_PPC64_TPREL64",
        74 => "R_PPC64_DTPREL16",
        75 => "R_PPC64_DTPREL16_LO",
        76 => "R_PPC64_DTPREL16_HI",
        77 => "R_PPC64_DTPREL16_HA",
        78 => "R_PPC64_DTPREL64",
        79 => "R_PPC64_GOT_TLSGD16",
        80 => "R_PPC64_GOT_TLSGD16_LO",
        81 => "R_PPC64_GOT_TLSGD16_HI",
        82 => "R_PPC64_GOT_TLSGD16_HA",
        83 => "R_PPC64_GOT_TLSLD16",
        84 => "R_PPC64_GOT_TLSLD16_LO",
        85 => "R_PPC64_GOT_TLSLD16_HI",
        86 => "R_PPC64_GOT_TLSLD16_HA",
        87 => "R_PPC64_GOT_TPREL16_DS",
        88 => "R_PPC64_GOT_TPREL16_LO_DS",
        89 => "R_PPC64_GOT_TPREL16_HI",
        90 => "R_PPC64_GOT_TPREL16_HA",
        91 => "R_PPC64_GOT_DTPREL16_DS",
        92 => "R_PPC64_GOT_DTPREL16_LO_DS",
        93 => "R_PPC64_GOT_DTPREL16_HI",
        94 => "R_PPC64_GOT_DTPREL16_HA",
        95 => "R_PPC64_TPREL16_DS",
        96 => "R_PPC64_TPREL16_LO_DS",
        97 => "R_PPC64_TPREL16_HIGHER",
        98 => "R_PPC64_TPREL16_HIGHERA",
        99 => "R_PPC64_TPREL16_HIGHEST",
        100 => "R_PPC64_TPREL16_HIGHESTA",
        101 => "R_PPC64_DTPREL16_DS",
        102 => "R_PPC64_DTPREL16_LO_DS",
        103 => "R_PPC64_DTPREL16_HIGHER",
        104 => "R_PPC64_DTPREL16_HIGHERA",
        105 => "R_PPC64_DTPREL16_HIGHEST",
        106 => "R_PPC64_DTPREL16_HIGHESTA",
        _ => "R_PPC64_UNKNOWN",
    }
}

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        // is_ok_and: a permission-denied read on an ancestor falls
        // through; the loop still terminates via pop() returning false.
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

#[test]
#[ignore = "regeneration: run with --ignored regenerate_firmware_reloc_census"]
fn regenerate_firmware_reloc_census() {
    let dir = match std::env::var("CELLGOV_FIRMWARE_DIR") {
        Ok(s) => PathBuf::from(s),
        Err(_) => workspace_root().join("firmware/sys/external"),
    };
    assert!(
        dir.is_dir(),
        "firmware dir {} not found; run `cellgov_firmware install` first or set CELLGOV_FIRMWARE_DIR",
        dir.display()
    );

    let mut sprx_paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read_dir on validated firmware dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("sprx"))
        })
        .collect();
    sprx_paths.sort();

    let mut per_module: BTreeMap<String, BTreeMap<u32, u64>> = BTreeMap::new();
    let mut union_types: BTreeSet<u32> = BTreeSet::new();
    let mut skipped: Vec<(String, String)> = Vec::new();

    for sprx_path in &sprx_paths {
        let name = sprx_path
            .file_name()
            .expect("read_dir entry always has a name")
            .to_string_lossy()
            .into_owned();
        if !filename_is_safe(&name) {
            skipped.push((
                name,
                "rejected charset (filename outside [A-Za-z0-9._-]+)".into(),
            ));
            continue;
        }
        let raw = match std::fs::read(sprx_path) {
            Ok(b) => b,
            Err(e) => {
                skipped.push((name, format!("io: {e}")));
                continue;
            }
        };
        let elf = match cellgov_firmware::sce::decrypt_self_to_elf(&raw) {
            Ok(e) => e,
            Err(e) => {
                skipped.push((name, format!("decrypt: {e}")));
                continue;
            }
        };
        let parsed = match cellgov_ppu::sprx::parse_prx(&elf) {
            Ok(p) => p,
            Err(e) => {
                skipped.push((name, format!("parse: {e:?}")));
                continue;
            }
        };
        let mut counts: BTreeMap<u32, u64> = BTreeMap::new();
        for r in &parsed.relocations {
            *counts.entry(r.rtype).or_insert(0) += 1;
            union_types.insert(r.rtype);
        }
        per_module.insert(name, counts);
    }

    let mut totals: BTreeMap<u32, u64> = BTreeMap::new();
    for counts in per_module.values() {
        for (&t, &c) in counts {
            *totals.entry(t).or_insert(0) += c;
        }
    }

    let mut out = String::new();
    writeln!(out, "# Firmware PRX relocation-type census").expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "Generated by `cargo test -p cellgov_cli --test firmware_reloc_census --release -- --ignored regenerate_firmware_reloc_census`. The test writes this file directly via `std::fs::write`; no stdout redirection."
    )
    .expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "Source: every `*.sprx` under `{}` (default `firmware/sys/external`, overridable via `CELLGOV_FIRMWARE_DIR`). Each row lists the distinct `R_PPC64_*` types observed in that PRX's `PT_PRX_RELOC` segment after SCE decryption. The applier at `crates/cellgov_ppu/src/sprx.rs::apply_relocations` must cover the union of these types for the dependency-ordered firmware loader to handle every module. The \"Applier covered?\" column reads `cellgov_ppu::sprx::APPLIER_SUPPORTED_TYPES`, so this doc and the applier never disagree silently.",
        dir.display()
    )
    .expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "Covered modules: {}/{} ({} skipped).",
        per_module.len(),
        sprx_paths.len(),
        skipped.len()
    )
    .expect("write to String");
    writeln!(out).expect("write to String");

    writeln!(out, "## Per-PRX distinct reloc types").expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(out, "| PRX | Total relocs | Distinct types | Type set |").expect("write to String");
    writeln!(out, "|---|---:|---:|---|").expect("write to String");
    for (name, counts) in &per_module {
        let total: u64 = counts.values().sum();
        let mut types_sorted: Vec<u32> = counts.keys().copied().collect();
        types_sorted.sort_unstable();
        let type_set = types_sorted
            .iter()
            .map(|t| format!("{} ({}, n={})", reloc_type_name(*t), t, counts[t]))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            out,
            "| `{name}` | {total} | {} | {type_set} |",
            counts.len()
        )
        .expect("write to String");
    }
    writeln!(out).expect("write to String");

    // Sort: uncovered first (so the next type to implement is at the
    // top), then by total-occurrences descending so the prioritization
    // signal is unambiguous, then by numeric ascending as a stable
    // tiebreak.
    let mut union_sorted: Vec<u32> = union_types.iter().copied().collect();
    union_sorted.sort_by(|&a, &b| {
        let ca = cellgov_ppu::sprx::is_applier_supported(a);
        let cb = cellgov_ppu::sprx::is_applier_supported(b);
        ca.cmp(&cb)
            .then(totals[&b].cmp(&totals[&a]))
            .then(a.cmp(&b))
    });

    writeln!(out, "## Union across all PRXes").expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "Uncovered types first (sorted by total occurrences descending); covered types follow."
    )
    .expect("write to String");
    writeln!(out).expect("write to String");
    writeln!(
        out,
        "| Type | Numeric | Applier covered? | Total occurrences |"
    )
    .expect("write to String");
    writeln!(out, "|---|---:|---|---:|").expect("write to String");
    for &t in &union_sorted {
        let covered = if cellgov_ppu::sprx::is_applier_supported(t) {
            "yes"
        } else {
            "no"
        };
        writeln!(
            out,
            "| {} | {} | {} | {} |",
            reloc_type_name(t),
            t,
            covered,
            totals[&t]
        )
        .expect("write to String");
    }
    writeln!(out).expect("write to String");

    let unknowns: Vec<u32> = union_sorted
        .iter()
        .copied()
        .filter(|&t| reloc_type_name(t) == "R_PPC64_UNKNOWN")
        .collect();
    if !unknowns.is_empty() {
        writeln!(out, "## Unknown reloc types").expect("write to String");
        writeln!(out).expect("write to String");
        writeln!(
            out,
            "Numeric values absent from `reloc_type_name`'s match. A future firmware revision that emits one of these surfaces here loudly; the `debug_assert!` at the end of the regenerator trips the test in debug builds."
        )
        .expect("write to String");
        writeln!(out).expect("write to String");
        writeln!(out, "| Numeric | Total occurrences | Modules |").expect("write to String");
        writeln!(out, "|---:|---:|---|").expect("write to String");
        for u in &unknowns {
            let mods: Vec<String> = per_module
                .iter()
                .filter(|(_, c)| c.contains_key(u))
                .map(|(n, _)| format!("`{n}`"))
                .collect();
            writeln!(out, "| {} | {} | {} |", u, totals[u], mods.join(", "))
                .expect("write to String");
        }
        writeln!(out).expect("write to String");
    }

    if !skipped.is_empty() {
        writeln!(out, "## Skipped modules").expect("write to String");
        writeln!(out).expect("write to String");
        writeln!(out, "| Module | Reason |").expect("write to String");
        writeln!(out, "|---|---|").expect("write to String");
        skipped.sort();
        for (name, reason) in &skipped {
            writeln!(out, "| `{name}` | {reason} |").expect("write to String");
        }
        writeln!(out).expect("write to String");
    }

    let dst = workspace_root().join("docs/dev/firmware_reloc_census.md");
    // docs/dev/ is gitignored; create it so fs::write does not ENOENT.
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).expect("create docs/dev parent dir");
    }
    std::fs::write(&dst, &out).expect("write firmware_reloc_census.md");

    debug_assert!(
        unknowns.is_empty(),
        "{} unknown reloc type(s) appeared in firmware corpus; extend reloc_type_name and re-run: {:?}",
        unknowns.len(),
        unknowns
    );
}

/// Every type in `APPLIER_SUPPORTED_TYPES` must resolve to a
/// non-`R_PPC64_UNKNOWN` name in `reloc_type_name`. The regeneration
/// test above is `#[ignore]` (it needs a decrypted firmware corpus);
/// this companion test runs in CI and catches a drift where the
/// applier supports a numeric the local table cannot label.
#[test]
fn reloc_type_name_covers_applier_supported() {
    for &t in cellgov_ppu::sprx::APPLIER_SUPPORTED_TYPES {
        let name = reloc_type_name(t);
        assert_ne!(
            name, "R_PPC64_UNKNOWN",
            "applier supports type {t} but reloc_type_name has no entry",
        );
        assert!(
            name.starts_with("R_PPC64_"),
            "reloc_type_name({t}) = {name:?}; expected R_PPC64_* prefix",
        );
    }
    assert_eq!(reloc_type_name(0), "R_PPC64_NONE");
    assert_eq!(reloc_type_name(38), "R_PPC64_ADDR64");
    assert_eq!(reloc_type_name(u32::MAX), "R_PPC64_UNKNOWN");
}
