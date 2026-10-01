//! The provenance builder: file hashes against known vectors, a record
//! that passes the loader's own check, and the clock text.

use super::*;

/// SHA-256 of `abc`, from FIPS 180-2 appendix B.1.
const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn console() -> ConsoleFacts {
    ConsoleFacts {
        profile: "cech20-cex-493".to_string(),
        model: "CECH-2001A".to_string(),
        kernel: "cex".to_string(),
        firmware: "4.93".to_string(),
        cfw: "EvilNAT 4.93 PEX".to_string(),
        cobra: "8.5".to_string(),
        webman: Some("1.47.48t".to_string()),
        debugger_attached: false,
    }
}

/// Inputs whose every file holds its own name, under `dir`.
fn inputs(dir: &Path, spu: bool) -> CaptureInputs {
    let file = |name: &str| {
        let path = dir.join(name);
        std::fs::write(&path, name).expect("write input");
        path
    };
    CaptureInputs {
        name: "spu_fixed_value".to_string(),
        manifest: file("manifest.toml"),
        sources: vec![("ppu/main.c".to_string(), file("main.c"))],
        eboot: file("EBOOT.BIN"),
        ps3_elf: file("ps3.elf"),
        reference_elf: file("reference.elf"),
        spu_elf: spu.then(|| file("spu_main.elf")),
        param_sfo: file("PARAM.SFO"),
        result_path: "/dev_hdd0/tmp/cgov_spu_fixed_value.bin".to_string(),
        console: console(),
        harness_revision: "0123456789abcdef".to_string(),
        recapture_reason: None,
    }
}

#[test]
fn the_hash_is_lowercase_hex_sha256() {
    assert_eq!(sha256_hex(b"abc"), ABC_SHA256);
}

#[test]
fn a_built_record_passes_the_loaders_check_and_hashes_every_input() {
    let dir = cellgov_testkit::scratch::scratch();
    let record = build(
        &inputs(&dir, true),
        b"abc",
        "2026-09-30T12:00:00Z".to_string(),
    )
    .expect("builds");
    record.check().expect("the loader accepts it");
    assert_eq!(record.capture_id, "micro:spu_fixed_value#ba7816bf8f01");
    assert_eq!(record.frame.sha256, ABC_SHA256);
    assert_eq!(record.frame.bytes, 3);
    assert_eq!(
        record.digest,
        format!("{ABC_SHA256}  micro:spu_fixed_value  3")
    );
    assert_eq!(
        record.microtest.manifest_sha256,
        sha256_hex(b"manifest.toml")
    );
    assert_eq!(
        record.microtest.sources["ppu/main.c"],
        sha256_hex(b"main.c")
    );
    assert_eq!(record.artifacts.eboot_sha256, sha256_hex(b"EBOOT.BIN"));
    assert_eq!(record.artifacts.ps3_elf_sha256, sha256_hex(b"ps3.elf"));
    assert_eq!(
        record.artifacts.reference_elf_sha256,
        sha256_hex(b"reference.elf")
    );
    assert_eq!(
        record.artifacts.spu_elf_sha256.as_deref(),
        Some(sha256_hex(b"spu_main.elf").as_str())
    );
    assert_eq!(record.artifacts.param_sfo_sha256, sha256_hex(b"PARAM.SFO"));
    assert_eq!(record.harness.runner, "runner_ps3");
    assert_eq!(
        record.harness.link,
        "https://github.com/RoyalAce22/CellGov/commit/0123456789abcdef"
    );
    assert_eq!(record.transport.kind, "webman-filedrop");
    assert_eq!(record.console, console());
}

#[test]
fn a_test_with_no_spu_image_records_none() {
    let dir = cellgov_testkit::scratch::scratch();
    let record = build(&inputs(&dir, false), b"abc", String::new()).expect("builds");
    assert_eq!(record.artifacts.spu_elf_sha256, None);
}

#[test]
fn a_missing_input_is_named() {
    let dir = cellgov_testkit::scratch::scratch();
    let mut missing = inputs(&dir, false);
    missing.eboot = dir.join("absent.bin");
    match build(&missing, b"abc", String::new()) {
        Err(RunnerPs3Error::LocalIo { path, .. }) => assert_eq!(path, dir.join("absent.bin")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_clock_text_is_rfc_3339_utc() {
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
    assert_eq!(rfc3339(4_107_542_399), "2100-02-28T23:59:59Z");
    assert_eq!(rfc3339(4_107_542_400), "2100-03-01T00:00:00Z");
    let now = now_rfc3339();
    assert_eq!(now.len(), "1970-01-01T00:00:00Z".len(), "{now}");
    assert!(now.ends_with('Z'), "{now}");
}
