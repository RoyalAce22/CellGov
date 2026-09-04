//! `dev gen-manifest`: turn an install record into a title-manifest
//! stub, closing the loop from "installed a PKG/ISO" to "registered,
//! bootable title".
//!
//! For a title, the PARAM.SFO-derived fields (`content_id`,
//! `display_name`, `eboot_candidates`, `distribution`, `rap_filename`)
//! are filled from the install record; the curated fields
//! (`short_name`, `year`, `developer`, `engine`, `rsx`, `content`,
//! `mounts`) are written as placeholders for an author to fill. For a
//! firmware entry the generated fields are the position the system
//! software holds inside a firmware tree. The stub names no version:
//! the store holds the versions, and `--fw` selects the one a boot
//! resolves against.
//!
//! `gen-manifest` never overwrites an existing manifest, so its curated
//! fields survive. It writes a stub only where no manifest exists, and
//! otherwise reports the generated identity for the user to reconcile.

use std::path::{Path, PathBuf};

use cellgov_install::store::{
    preflight, record_rel_path, Artifact, ArtifactKind, InstallRecord, StoreLayout, TitleId,
    TitleRecord, VersionKey, DEFAULT_VFS_ROOT,
};
use cellgov_ps3_abi::dev_flash::{FLASH_MOUNT, VSH_MODULE_DIR, VSH_SELF};

use crate::cli::exit::die;
use crate::cli::parse::GenManifestArgs;
use crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR;
use crate::game::manifest::TitleManifest;

/// Registry key of the system software a firmware image ships.
const VSH_CONTENT_ID: &str = "VSH";

/// The system software's directory, as the manifest's `[source] path`
/// spells it: relative to a firmware entry, `/`-separated.
fn vsh_source_path() -> String {
    format!("{FLASH_MOUNT}/{VSH_MODULE_DIR}")
}

/// The system software's `[title] distribution` tag.
const VSH_DISTRIBUTION: &str = "firmware-exec";

/// The install-record directory under the default store root, where
/// the installers write. `--installs` names that directory directly,
/// for a store rooted elsewhere.
fn default_installs() -> PathBuf {
    StoreLayout::new(DEFAULT_VFS_ROOT).installs_dir()
}

/// The default install-record directory, refusing a root the store
/// cannot read.
///
/// The default is the only invocation that resolves a store root:
/// `--installs` and `--record` each name a path directly.
fn default_installs_checked() -> PathBuf {
    preflight(Path::new(DEFAULT_VFS_ROOT))
        .unwrap_or_else(|e| die(&format!("gen-manifest failed: {e}")));
    default_installs()
}

/// The base record for `title_id` under an `installs/` directory.
///
/// `--installs` names the record directory itself, so the path is built
/// from the same relative arithmetic [`StoreLayout::record_path`] uses
/// below a VFS root.
fn base_record_under(installs: &Path, title_id: &str) -> PathBuf {
    let title_id =
        TitleId::new(title_id).unwrap_or_else(|e| die(&format!("gen-manifest --title-id: {e}")));
    installs.join(record_rel_path(&Artifact::TitleBase { title_id }))
}

/// The firmware record for `version` under an `installs/` directory.
///
/// See [`base_record_under`] for why the path is built relatively.
fn firmware_record_under(installs: &Path, version: &str) -> PathBuf {
    let version =
        VersionKey::new(version).unwrap_or_else(|e| die(&format!("gen-manifest --firmware: {e}")));
    installs.join(record_rel_path(&Artifact::Firmware { version }))
}

/// Escape a string for a double-quoted TOML basic string.
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(crate) fn run(args: &GenManifestArgs) {
    let (record_path, asked) = resolve_record_path(args);
    let registry = args
        .registry
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_TITLE_REGISTRY_DIR));
    let force = args.force;

    let text = std::fs::read_to_string(&record_path)
        .unwrap_or_else(|e| die(&format!("failed to read {}: {e}", record_path.display())));
    let record = InstallRecord::parse(&text)
        .unwrap_or_else(|e| die(&format!("parse {}: {e}", record_path.display())));
    if let Some(refusal) = selector_mismatch(asked, &record, &record_path) {
        die(&refusal);
    }

    let gen = Generated::from_record(&record, &record_path);
    let manifest_path = registry.join(format!("{}.toml", gen.content_id()));

    let stub = gen.render_stub(&record_path);
    // A record's `distribution` is a free-form string and its `title`
    // is PARAM.SFO text, so a stub can carry a field the manifest
    // loader refuses. The check runs ahead of the identity report as
    // well as the write, so both answer the same for every record.
    if let Err(e) = TitleManifest::load_from_text(&stub, &manifest_path) {
        die(&format!(
            "the stub generated from {} is not a title manifest: {e}",
            record_path.display()
        ));
    }

    if manifest_path.exists() && !force {
        println!(
            "title manifest {} already exists; not overwriting (curated fields preserved).",
            manifest_path.display()
        );
        println!("  generated identity from {}:", record_path.display());
        gen.print_identity();
        println!("  reconcile by hand if any generated field drifted.");
        return;
    }

    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| die(&format!("create {}: {e}", parent.display())));
    }
    std::fs::write(&manifest_path, stub)
        .unwrap_or_else(|e| die(&format!("write {}: {e}", manifest_path.display())));
    println!("wrote title-manifest stub {}", manifest_path.display());
    println!("  fill in the curated fields ({}).", gen.curated_fields());
}

/// The record a selector names, and the kind that selector asked for.
///
/// `--record` names a path, so no kind is asked for and the record's
/// own kind is the answer. `--title-id` and `--firmware` each resolve
/// one kind's records directory; clap requires exactly one of the
/// three.
fn resolve_record_path(args: &GenManifestArgs) -> (PathBuf, Option<ArtifactKind>) {
    let lookup = |id: &str, resolve: fn(&Path, &str) -> PathBuf| {
        let installs = args
            .installs
            .clone()
            .unwrap_or_else(default_installs_checked);
        resolve(&installs, id)
    };
    match (&args.record, &args.title_id, &args.firmware) {
        (Some(p), _, _) => (p.clone(), None),
        (None, Some(id), _) => (lookup(id, base_record_under), Some(ArtifactKind::TitleBase)),
        (None, None, Some(version)) => (
            lookup(version, firmware_record_under),
            Some(ArtifactKind::Firmware),
        ),
        (None, None, None) => {
            die("gen-manifest requires --record <path>, --title-id <id>, or --firmware <version>")
        }
    }
}

/// The refusal when a selector's records directory holds a record of
/// another kind, or `None` when the record is the one asked for.
///
/// The two kinds render different manifests, so the kind a record
/// declares decides which manifest a run writes.
fn selector_mismatch(
    asked: Option<ArtifactKind>,
    record: &InstallRecord,
    record_path: &Path,
) -> Option<String> {
    let asked = asked?;
    let found = record.artifact.kind;
    (found != asked).then(|| {
        format!(
            "{} declares a {} entry where a {} record is looked up; generating from it \
             would write the manifest for something else. Pass --record <path> to \
             generate from this record as it declares itself",
            record_path.display(),
            found.as_str(),
            asked.as_str(),
        )
    })
}

/// Which manifest a record generates, and what it fills in.
enum Generated {
    /// A title manifest, from the identity the install recorded.
    Title(TitleFields),
    /// The system software's manifest, whose generated fields are the
    /// same in every firmware tree.
    Firmware,
}

impl Generated {
    fn from_record(record: &InstallRecord, record_path: &Path) -> Self {
        match (record.artifact.kind, record.title.as_ref()) {
            (ArtifactKind::Firmware, _) => Self::Firmware,
            // An update record names a title, but its `distribution`
            // is the update's own tag, which no title manifest holds.
            (ArtifactKind::TitleUpdate, _) => die(&format!(
                "{} describes a {} entry; a manifest is generated from the title's base record",
                record_path.display(),
                record.artifact.kind.as_str()
            )),
            (ArtifactKind::TitleBase, Some(title)) => {
                Self::Title(TitleFields::from_record(record, title))
            }
            (ArtifactKind::TitleBase, None) => die(&format!(
                "{} describes a {} entry, which names no title",
                record_path.display(),
                record.artifact.kind.as_str()
            )),
        }
    }

    /// The registry key, which names the file the stub is written to.
    fn content_id(&self) -> &str {
        match self {
            Self::Title(t) => &t.content_id,
            Self::Firmware => VSH_CONTENT_ID,
        }
    }

    fn curated_fields(&self) -> &'static str {
        match self {
            Self::Title(_) => "short_name, year, developer, engine, rsx",
            Self::Firmware => "display_name, year, developer, engine",
        }
    }

    fn print_identity(&self) {
        match self {
            Self::Title(t) => t.print_identity(),
            Self::Firmware => {
                println!("    content_id   = {VSH_CONTENT_ID}");
                println!("    distribution = {VSH_DISTRIBUTION}");
                println!("    eboot        = {VSH_SELF}");
                println!("    source path  = {}", vsh_source_path());
            }
        }
    }

    fn render_stub(&self, record_path: &Path) -> String {
        match self {
            Self::Title(t) => t.render_stub(record_path),
            Self::Firmware => render_firmware_stub(),
        }
    }
}

/// The PARAM.SFO / install-derived fields of a title manifest.
struct TitleFields {
    content_id: String,
    display_name: String,
    distribution: String,
    eboot_candidate: String,
    rap_filename: Option<String>,
}

impl TitleFields {
    fn from_record(record: &InstallRecord, title: &TitleRecord) -> Self {
        // The manifest's `content_id` directory key holds the title-id
        // value (a pre-existing field-name misnomer); the RAP, by
        // contrast, is keyed by the full NPD content id.
        let dir_key = title.title_id.clone();
        let eboot_candidate = record
            .files
            .keys()
            .find(|p| p.ends_with("EBOOT.BIN"))
            .and_then(|p| p.rsplit('/').next())
            .map_or_else(
                || {
                    // A v3 record may carry no `[files]` at all, so an
                    // EBOOT-less record reaches here and the stub still
                    // needs a candidate.
                    eprintln!(
                        "gen-manifest: record for {} lists no EBOOT.BIN; \
                         eboot_candidates falls back to the conventional name",
                        title.title_id,
                    );
                    "EBOOT.BIN".to_string()
                },
                |p| p.to_string(),
            );
        // The record names the RAP the install committed; a psn-hdd
        // record carrying none is a title that consumes none, since only
        // network/local licenses do (`game_install::rap_consumed`).
        let rap_filename = record.rap.as_ref().map(|r| r.filename.clone());
        Self {
            content_id: dir_key,
            display_name: title.title.clone(),
            distribution: title.distribution.clone(),
            eboot_candidate,
            rap_filename,
        }
    }

    fn print_identity(&self) {
        println!("    content_id   = {}", self.content_id);
        println!("    display_name = {}", self.display_name);
        println!("    distribution = {}", self.distribution);
        println!("    eboot        = {}", self.eboot_candidate);
        println!(
            "    rap_filename = {}",
            self.rap_filename.as_deref().unwrap_or("(none)")
        );
    }

    fn render_stub(&self, record_path: &Path) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "# Generated by `cellgov dev gen-manifest` from {}.\n",
            record_path.display()
        ));
        s.push_str(
            "# Curated fields below are placeholders -- fill them in. The\n\
             # generated fields (content_id, display_name, eboot_candidates,\n\
             # distribution, rap_filename) come from the install / PARAM.SFO.\n\n",
        );
        s.push_str("[title]\n");
        s.push_str(&format!(
            "content_id = \"{}\"\n",
            toml_escape(&self.content_id)
        ));
        s.push_str(&format!(
            "short_name = \"{}\"\n",
            self.content_id.to_lowercase()
        ));
        s.push_str(&format!(
            "display_name = \"{}\"\n",
            toml_escape(&self.display_name)
        ));
        s.push_str(&format!(
            "eboot_candidates = [\"{}\"]\n",
            toml_escape(&self.eboot_candidate)
        ));
        s.push_str("year = 0\n");
        s.push_str("developer = \"FILLME\"\n");
        s.push_str("engine = \"FILLME\"\n");
        s.push_str(&format!(
            "distribution = \"{}\"\n",
            toml_escape(&self.distribution)
        ));
        if let Some(rap) = &self.rap_filename {
            s.push_str(&format!("rap_filename = \"{}\"\n", toml_escape(rap)));
        }
        // The loader's `[source]` default is hdd; a disc title left on
        // that default resolves under dev_hdd0 and never finds its EBOOT.
        if self.distribution == "disc-iso" {
            s.push_str("\n[source]\nkind = \"disc\"\n");
        }
        s.push_str("\n[checkpoint]\nkind = \"process-exit\"\n");
        s.push_str("\n[rsx]\nmirror = false\nconsume = false\n");
        s
    }
}

/// The system software's stub, which names no firmware version.
///
/// The record's filename carries the version, so the header names the
/// record's kind in place of its path.
fn render_firmware_stub() -> String {
    format!(
        "# Generated by `cellgov dev gen-manifest` from a firmware install record.\n\
         # Curated fields below are placeholders -- fill them in. Of the\n\
         # generated fields, content_id and short_name key the registry;\n\
         # eboot_candidates, distribution and source are where a firmware\n\
         # tree puts the system software, the same in every version.\n\
         \n\
         [title]\n\
         content_id = \"{VSH_CONTENT_ID}\"\n\
         short_name = \"{short_name}\"\n\
         display_name = \"FILLME\"\n\
         eboot_candidates = [\"{VSH_SELF}\"]\n\
         year = 0\n\
         developer = \"FILLME\"\n\
         engine = \"FILLME\"\n\
         distribution = \"{VSH_DISTRIBUTION}\"\n\
         \n\
         [source]\n\
         kind = \"firmware-exec\"\n\
         path = \"{source_path}\"\n\
         \n\
         [checkpoint]\n\
         kind = \"process-exit\"\n\
         \n\
         [rsx]\n\
         mirror = false\n\
         consume = false\n",
        short_name = VSH_CONTENT_ID.to_lowercase(),
        source_path = vsh_source_path(),
    )
}

#[cfg(test)]
#[path = "tests/gen_manifest_tests.rs"]
mod tests;
