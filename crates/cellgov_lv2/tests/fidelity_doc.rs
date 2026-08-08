//! `docs/lv2_fidelity.md` drift gate.
//!
//! The committed doc is rendered from the fidelity map in
//! `request::fidelity`; this test fails when they disagree.
//! Regenerate with:
//!
//! ```text
//! cargo test -p cellgov_lv2 --test fidelity_doc -- --ignored regenerate
//! ```

use cellgov_lv2::request::fidelity::{unsupported_arm_fidelity, ROUTED_UNSUPPORTED_ARMS};
use cellgov_lv2::request::{classify, Lv2Request, Lv2RequestKind};
use strum::VariantArray;

fn doc_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lv2_fidelity.md")
}

/// Census over LV2's 1024 syscall slots: how many classify to a typed
/// arm, how many route to a dedicated `Unsupported` arm, and how many
/// fall to the null backend.
fn slot_census() -> (usize, usize, usize) {
    let mut typed = 0usize;
    let mut routed = 0usize;
    for number in 0..1024u64 {
        // Exhaustive on the non-typed shapes: a future classify-time
        // gate that rejects the zero probe args must fail here, not
        // silently inflate the typed count in a published doc.
        match classify(number, &[0u64; 8]) {
            Lv2Request::Unsupported { .. } => {
                if ROUTED_UNSUPPORTED_ARMS.iter().any(|(n, _, _)| *n == number) {
                    routed += 1;
                }
            }
            Lv2Request::Malformed { .. } => {
                panic!("slot {number}: zero probe args classified Malformed; the census is invalid")
            }
            Lv2Request::Hypercall { .. } => {
                panic!("slot {number}: zero probe args classified Hypercall; the census is invalid")
            }
            Lv2Request::UnresolvedImport { .. } => panic!(
                "slot {number}: zero probe args classified UnresolvedImport; the census is invalid"
            ),
            _ => typed += 1,
        }
    }
    (typed, routed, 1024 - typed - routed)
}

fn render() -> String {
    let (typed, routed, null_backend) = slot_census();
    let mut out = String::new();
    out.push_str(
        "# LV2 arm fidelity\n\n\
         <!-- Rendered from `cellgov_lv2::request::fidelity` by\n\
         `cargo test -p cellgov_lv2 --test fidelity_doc -- --ignored regenerate`.\n\
         Do not edit by hand: the non-ignored test in the same file fails on drift. -->\n\n",
    );
    out.push_str(&format!(
        "Of LV2's 1024 syscall slots, {typed} classify to a typed arm\n\
         and {routed} route to a dedicated arm inside `Unsupported`;\n\
         the remaining **{null_backend} slots are the null backend** --\n\
         the honest traced `CELL_ENOSYS` refusal is the default, not\n\
         the exception. The `null-backend` rows below are the handful\n\
         of syscalls given _dedicated_ routing or diagnostics that\n\
         still refuse; the unlisted mass refuses through the generic\n\
         arm.\n\n",
    ));
    out.push_str(
        "The routing-layer guarantee -- an unhandled syscall returns\n\
         `CELL_ENOSYS` through the null backend, never a fabricated\n\
         success -- holds for the whole surface. How much real LV2\n\
         behavior each _handled_ arm reproduces is a per-arm property.\n\
         The tags are reviewed claims: table membership is probe-gated\n\
         against dispatch, but a tag's accuracy rests on the review\n\
         recorded in each arm's rustdoc, not on a machine check.\n\n\
         | Tag | Meaning |\n\
         | --- | --- |\n\
         | `modeled` | Full modeled state and ABI, faithful to the oracle. |\n\
         | `partial-state` | ABI faithful; some kernel-visible state simplified or omitted. |\n\
         | `abi-only` | Plausible return value with little or no backing state. |\n\
         | `null-backend` | Honest `CELL_ENOSYS`-class refusal with a logged diagnostic. |\n\n\
         Per-arm rationale lives on each dispatch method's rustdoc in\n\
         `crates/cellgov_lv2/src/host/`.\n\n\
         ## Typed request arms\n\n\
         | Arm | Fidelity |\n\
         | --- | --- |\n",
    );
    for kind in Lv2RequestKind::VARIANTS {
        if let Some(f) = kind.fidelity() {
            let name: &'static str = (*kind).into();
            out.push_str(&format!("| `{name}` | {} |\n", f.label()));
        }
    }
    out.push_str(
        "\n## Routed `Unsupported` arms\n\n\
         Syscalls without a typed variant that still reach a dedicated\n\
         arm. Any number not listed dispatches to the null backend.\n\n\
         | Syscall | Name | Fidelity |\n\
         | --- | --- | --- |\n",
    );
    for (n, name, f) in ROUTED_UNSUPPORTED_ARMS {
        assert_eq!(unsupported_arm_fidelity(*n), *f);
        out.push_str(&format!("| {n} | `{name}` | {} |\n", f.label()));
    }
    out
}

/// Collapse table-cell padding so a markdown formatter's column
/// alignment does not read as drift; content changes still do.
fn normalize(text: &str) -> String {
    let mut out = String::new();
    for line in text.replace("\r\n", "\n").lines() {
        let line = line.trim_end();
        if line.starts_with('|') && line.ends_with('|') {
            let cells: Vec<String> = line
                .trim_matches('|')
                .split('|')
                .map(|c| {
                    let c = c.trim();
                    if c.len() >= 3 && c.chars().all(|ch| ch == '-') {
                        "---".to_string()
                    } else {
                        c.to_string()
                    }
                })
                .collect();
            out.push_str(&format!("| {} |\n", cells.join(" | ")));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn committed_doc_matches_fidelity_map() {
    let path = doc_path();
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}; regenerate with the ignored test",
            path.display()
        )
    });
    assert_eq!(
        normalize(&committed),
        normalize(&render()),
        "docs/lv2_fidelity.md is stale; regenerate with:\n  \
         cargo test -p cellgov_lv2 --test fidelity_doc -- --ignored regenerate"
    );
}

#[test]
#[ignore = "writes docs/lv2_fidelity.md; run on fidelity-map changes"]
fn regenerate() {
    let path = doc_path();
    std::fs::write(&path, render()).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}
