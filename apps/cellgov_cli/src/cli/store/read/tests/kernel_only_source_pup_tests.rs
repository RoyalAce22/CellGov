use super::*;

fn pre_block_entry(pup_sha256: &str) -> FirmwareDoc {
    FirmwareDoc {
        version: "2.76".to_string(),
        entry_dir: "firmware/2.76".to_string(),
        record: Some(".cellgov/installs/firmware/2.76.install.toml".to_string()),
        pup_sha256: pup_sha256.to_string(),
        image_version: None,
        modules: Some(412),
        manifest_error: None,
        core_os: None,
    }
}

fn line_with_label<'a>(rendered: &'a str, label: &str) -> &'a str {
    let lines: Vec<&str> = rendered
        .lines()
        .filter(|line| line.starts_with(label))
        .collect();
    assert_eq!(lines.len(), 1, "{rendered}");
    lines[0]
}

#[test]
fn a_pre_block_entry_shows_the_source_pup_digest_its_kernel_hint_points_to() {
    let source = "03".repeat(32);
    let rendered = render_firmware_detail(&pre_block_entry(&source));
    assert_eq!(
        line_with_label(&rendered, "  pup sha256 "),
        format!("  pup sha256 {source}")
    );
    assert_eq!(
        line_with_label(&rendered, "  kernel     "),
        "  kernel     not unpacked (installed before the kernel was kept; add it with \
         `cellgov firmware install <PUP> --kernel-only`, giving the PUP this entry was \
         installed from, whose sha256 `cellgov firmware show <VERSION>` prints)"
    );
}
