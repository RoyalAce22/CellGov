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
    pub(super) bench: Option<ManifestBench>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestBench {
    #[serde(default)]
    pub(super) matrix: Vec<ManifestMatrixRow>,
}

/// One `[[bench.matrix]]` row.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestMatrixRow {
    pub(super) fw: String,
    /// `"base"` or an update version key; see
    /// [`super::MatrixCell::game_ver`].
    #[serde(default)]
    pub(super) game_ver: Option<String>,
    #[serde(default)]
    pub(super) reference: bool,
    /// One of `"frontier"`, `"probe"`; defaults to `"frontier"`.
    #[serde(default)]
    pub(super) expect: Option<String>,
    #[serde(default)]
    pub(super) bench_max_steps: Option<u64>,
    #[serde(default)]
    pub(super) checkpoint: Option<ManifestCheckpoint>,
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
    /// One of `"hdd"`, `"disc"`, `"firmware-exec"`,
    /// `"manifest-relative"`.
    pub(super) kind: String,
    /// Host directory holding the executable. Required by
    /// `firmware-exec` and `manifest-relative`, rejected by the other
    /// kinds, which derive the directory from `content_id`. A
    /// `manifest-relative` path resolves against the manifest's own
    /// directory; a `firmware-exec` one against the process cwd. Empty
    /// is rejected for both, and a rooted or drive-prefixed path is
    /// rejected for `manifest-relative`, which such a path would
    /// silently stop being.
    #[serde(default)]
    pub(super) path: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestTitle {
    /// Omittable only by a `manifest-relative` title, which has no PSN
    /// identity; the loader then derives it from the manifest's
    /// directory name.
    #[serde(default)]
    pub(super) content_id: Option<String>,
    pub(super) short_name: String,
    pub(super) display_name: String,
    pub(super) eboot_candidates: Vec<String>,
    pub(super) year: u16,
    pub(super) developer: String,
    pub(super) engine: String,
    /// One of `"psn-hdd"`, `"retail-hdd"`, `"disc-iso"`,
    /// `"firmware-exec"`, `"microtest"`.
    pub(super) distribution: String,
    /// Operator-supplied RAP filename for NPDRM titles, resolved at
    /// boot under `<vfs_root>/home/00000001/exdata/`. Required for
    /// PSN-HDD NPDRM titles whose `EBOOT.BIN` is NPDRM-wrapped
    /// (license type 1 / 2). Omit for disc / APP-keyed titles.
    #[serde(default)]
    pub(super) rap_filename: Option<String>,
    /// Instruction cap the witness suite boots this title under.
    /// A title whose recorded outcome is `MaxSteps` reproduces it
    /// only at the same cap.
    #[serde(default)]
    pub(super) bench_max_steps: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestCheckpoint {
    pub(super) kind: String,
    #[serde(default)]
    pub(super) pc: Option<String>,
}
