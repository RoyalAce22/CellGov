//! On-disk TOML wire format consumed by the loader and translated into
//! [`super::model`].

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestFile {
    pub(super) title: ManifestTitle,
    pub(super) checkpoint: ManifestCheckpoint,
    pub(super) source: Option<ManifestSource>,
    pub(super) rsx: Option<ManifestRsx>,
    pub(super) content: Option<ManifestContent>,
    pub(super) fs: Option<ManifestFs>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestFs {
    #[serde(default)]
    pub(super) mounts: Vec<ManifestMount>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestMount {
    pub(super) prefix: String,
    pub(super) host: String,
    #[serde(default)]
    pub(super) override_env: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestContent {
    pub(super) base: String,
    #[serde(default)]
    pub(super) override_base_env: Option<String>,
    pub(super) files: Vec<ManifestContentFile>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestContentFile {
    pub(super) guest_path: String,
    pub(super) host_path: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestRsx {
    #[serde(default)]
    pub(super) mirror: bool,
    #[serde(default)]
    pub(super) consume: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestSource {
    pub(super) kind: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestTitle {
    pub(super) content_id: String,
    pub(super) short_name: String,
    pub(super) display_name: String,
    pub(super) eboot_candidates: Vec<String>,
    pub(super) year: u16,
    pub(super) developer: String,
    pub(super) engine: String,
    /// One of `"psn-hdd"`, `"retail-hdd"`, `"disc-iso"`.
    pub(super) distribution: String,
    /// Operator-supplied RAP filename for NPDRM titles, resolved at
    /// boot under `<vfs_root>/home/00000001/exdata/`. Required for
    /// PSN-HDD NPDRM titles whose `EBOOT.BIN` is NPDRM-wrapped
    /// (license type 1 / 2). Omit for disc / APP-keyed titles.
    #[serde(default)]
    pub(super) rap_filename: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestCheckpoint {
    pub(super) kind: String,
    #[serde(default)]
    pub(super) pc: Option<String>,
}
