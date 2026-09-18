use super::*;

use crate::cli::store::read::model::STORE_FORMAT_VERSION;

fn row(version: &str, state: &str, detail: Option<&str>) -> KernelCoverageEntryDoc {
    KernelCoverageEntryDoc {
        version: version.to_string(),
        state: state.to_string(),
        detail: detail.map(str::to_string),
        kernel_version: None,
        elf_bytes: None,
        elf_sha256: None,
    }
}

fn doc(entries: Vec<KernelCoverageEntryDoc>) -> KernelCoverageDoc {
    KernelCoverageDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        vault: "store/keys.toml".to_string(),
        entries,
    }
}

#[test]
fn every_state_renders_its_own_row_and_the_tally_counts_each() {
    let mut decrypted = row("4.93", "decrypted", None);
    decrypted.kernel_version = Some("4.93".to_string());
    decrypted.elf_bytes = Some(3 * 1024 * 1024);
    decrypted.elf_sha256 = Some("ab".repeat(32));
    let doc = doc(vec![
        row(
            "1.02",
            "no_key",
            Some("an LV2 keyset for firmware 1.02 (the vault holds none)"),
        ),
        row(
            "1.94",
            "not_unpacked",
            Some("update_files carries no CORE_OS_PACKAGE.pkg"),
        ),
        row(
            "3.55",
            "failed",
            Some("SCE: no usable section found in decrypted package"),
        ),
        row("3.70", "unreadable", Some("read x: gone")),
        decrypted,
    ]);
    let rendered = render(&doc);
    assert!(rendered.starts_with("key vault: store/keys.toml\n"));
    assert!(
        rendered.contains(
            "  1.02     no key        an LV2 keyset for firmware 1.02 (the vault holds none)\n"
        ),
        "{rendered}"
    );
    assert!(
        rendered.contains("  1.94     not unpacked  update_files carries no CORE_OS_PACKAGE.pkg\n"),
        "{rendered}"
    );
    assert!(
        rendered.contains(&format!(
            "  4.93     decrypted     3.0 MB ELF (header names firmware 4.93), sha256 {}\n",
            "ab".repeat(32)
        )),
        "{rendered}"
    );
    assert!(
        rendered.ends_with(
            "5 version(s): 1 decrypted, 1 no key, 1 not unpacked, 1 unreadable, 1 failed\n"
        ),
        "{rendered}"
    );
    assert_eq!(exit_status(&doc), EXIT_KERNEL_NOT_DECRYPTED);
}

#[test]
fn a_missing_key_is_not_an_exit_failure_and_a_failed_decrypt_is() {
    let normal = doc(vec![
        row("1.02", "no_key", Some("an LV2 keyset for firmware 1.02")),
        row("1.94", "not_unpacked", Some("not unpacked")),
        row("4.93", "decrypted", None),
    ]);
    assert_eq!(exit_status(&normal), 0);
    for state in NOT_DECRYPTED_STATES {
        let bad = doc(vec![row("4.93", state, Some("x"))]);
        assert_eq!(exit_status(&bad), EXIT_KERNEL_NOT_DECRYPTED, "{state}");
    }
}

#[test]
fn an_empty_store_names_itself() {
    assert_eq!(
        render(&doc(Vec::new())),
        "no firmware installed under vfs\n"
    );
}

#[test]
fn every_state_the_run_produces_is_a_tallied_one() {
    use cellgov_install::kernel_decrypt::KernelCoverage;
    use cellgov_install::manifest::Sha256;

    let produced = [
        KernelCoverage::Decrypted {
            version: 0,
            elf_len: 0,
            elf_sha256: Sha256([0; 32]),
        },
        KernelCoverage::NoKey {
            version: None,
            missing: String::new(),
        },
        KernelCoverage::Unreadable {
            reason: String::new(),
        },
        KernelCoverage::Failed {
            version: None,
            reason: String::new(),
        },
    ];
    for state in produced {
        assert!(STATES.contains(&state.label()), "{}", state.label());
    }
    assert!(STATES.contains(&NOT_UNPACKED));
    for state in NOT_DECRYPTED_STATES {
        assert!(STATES.contains(&state));
    }
}

#[test]
fn readable_header_versions_reach_no_key_and_failed_rows() {
    use cellgov_install::kernel_decrypt::KernelCoverage;
    use cellgov_ps3_abi::format::sce::self_version;

    for (state, coverage) in [
        (
            "no_key",
            KernelCoverage::NoKey {
                version: Some(self_version(3, 0x60)),
                missing: "missing key".to_string(),
            },
        ),
        (
            "failed",
            KernelCoverage::Failed {
                version: Some(self_version(3, 0x60)),
                reason: "refused".to_string(),
            },
        ),
    ] {
        let mut entry = row("3.60", "not_unpacked", None);
        apply_coverage(&mut entry, coverage);
        assert_eq!(entry.state, state);
        assert_eq!(entry.kernel_version.as_deref(), Some("3.60"));
    }
}
