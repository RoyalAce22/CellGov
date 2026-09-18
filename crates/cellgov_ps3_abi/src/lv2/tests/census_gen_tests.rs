use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn archive_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lv2")
}

fn table_rows(path: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .lines()
        .skip(1)
        .map(|line| line.split('\t').map(str::to_string).collect())
        .collect()
}

fn render_table() -> String {
    let archive = archive_dir();
    let firmware_by_pup: BTreeMap<String, String> = table_rows(&archive.join("pup.tsv"))
        .into_iter()
        .map(|row| (row[0].clone(), row[1].clone()))
        .collect();
    let mut rows = table_rows(&archive.join("kernel.tsv"));
    rows.sort_by(|left, right| left[0].cmp(&right[0]));
    let mut out = String::from(
        "//! `PUP_CENSUS`, rendered from the committed LV2 archive.\n//!\n//! Generated: run `cargo test -p cellgov_ps3_abi --lib -- --ignored regenerate_pup_census`.\n\nuse super::PupCensus;\n\n#[rustfmt::skip]\npub(super) static PUP_CENSUS: &[PupCensus] = &[\n",
    );
    for row in rows {
        let pup = &row[0];
        let firmware = firmware_by_pup
            .get(pup)
            .unwrap_or_else(|| panic!("kernel PUP {pup} has no provenance row"));
        let census = table_rows(&archive.join(format!("census/fw-{firmware}.tsv")));
        out.push_str("    PupCensus { pup_sha256: [");
        for (index, byte) in pup.as_bytes().chunks_exact(2).enumerate() {
            if index != 0 {
                out.push_str(", ");
            }
            out.push_str("0x");
            out.push_str(std::str::from_utf8(byte).expect("PUP hash is ASCII"));
        }
        out.push_str("], classes: &[\n");
        for census_row in census {
            let class = match census_row[2].as_str() {
                "implemented" => "0",
                "stub" => "1",
                "absent" => "2",
                other => panic!("unknown census class {other}"),
            };
            out.push_str(class);
            out.push_str(",\n");
        }
        out.push_str("    ] },\n");
    }
    out.push_str("];\n");
    out
}

fn pup_digest(hex: &str) -> [u8; 32] {
    let mut digest = [0; 32];
    for (slot, pair) in digest.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).expect("PUP hash is ASCII"), 16)
            .expect("PUP hash is hexadecimal");
    }
    digest
}

#[test]
fn committed_pup_census_matches_archive() {
    assert_eq!(
        include_str!("../census_table.rs"),
        render_table(),
        "generated PUP census table is stale; run `cargo test -p cellgov_ps3_abi --lib -- --ignored regenerate_pup_census`"
    );
}

#[test]
fn provenance_pups_without_kernel_rows_are_not_extracted() {
    let archive = archive_dir();
    let kernels: std::collections::BTreeSet<String> = table_rows(&archive.join("kernel.tsv"))
        .into_iter()
        .map(|row| row[0].clone())
        .collect();
    let absent: Vec<String> = table_rows(&archive.join("pup.tsv"))
        .into_iter()
        .map(|row| row[0].clone())
        .filter(|pup| !kernels.contains(pup))
        .collect();
    assert!(!absent.is_empty(), "the partial corpus must stay visible");
    for pup in absent {
        assert_eq!(
            super::lookup(&pup_digest(&pup), 0),
            super::PupCensusClass::NotExtracted
        );
    }
}

#[test]
#[ignore = "rewrites the generated PUP census table"]
fn regenerate_pup_census() {
    std::fs::write(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lv2/census_table.rs"),
        render_table(),
    )
    .expect("write generated PUP census table");
}
