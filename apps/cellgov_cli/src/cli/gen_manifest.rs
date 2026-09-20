//! `dev gen-manifest`: turn an install record into a title-manifest
//! stub, closing the loop from "installed a PKG/ISO" to "registered,
//! bootable title".
//!
//! For a title, the install record fills the generated fields
//! (`content_id`, `display_name`, `eboot_candidates`, `distribution`,
//! `rap_filename`), and the `PS3_SYSTEM_VER` in the installed tree's
//! own PARAM.SFO fills `system_ver`. The curated fields (`short_name`,
//! `year`, `developer`, `engine`, `rsx`, `content`, `mounts`) are
//! placeholders for an author to fill. For a firmware entry the
//! generated fields are the position the system software holds inside
//! a firmware tree. That stub names no version: the store holds the
//! versions, and `--fw` selects the one a boot resolves against.
//!
//! `gen-manifest` never overwrites an existing manifest, so its curated
//! fields survive. It writes a stub only where no manifest exists, and
//! otherwise reports the generated identity for the user to reconcile.

use std::path::{Path, PathBuf};

use cellgov_install::manifest::{sha256_of, Sha256};
use cellgov_install::param_sfo;
use cellgov_install::store::{
    preflight, record_rel_path, Artifact, ArtifactKind, InstallRecord, StoreLayout, TitleId,
    TitleRecord, VersionKey,
};
use cellgov_install::system_ver::firmware_version_key;
use cellgov_ps3_abi::format::dev_flash::{FLASH_MOUNT, VSH_MODULE_DIR, VSH_SELF};
use cellgov_ps3_abi::format::param_sfo::{PARAM_SFO_FILE, PS3_SYSTEM_VER_KEY};
use cellgov_ps3_abi::format::title_tree::DISC_GAME_DIR;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::keys::install_root_of;
use crate::cli::parse::GenManifestArgs;
use crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR;
use cellgov_boot::manifest::TitleManifest;

/// The `distribution` tag a disc install records; its PARAM.SFO sits
/// under `PS3_GAME/`.
const DISC_DISTRIBUTION: &str = "disc-iso";

/// Registry key of the system software a firmware image ships.
const VSH_CONTENT_ID: &str = "VSH";

/// The system software's directory, as the manifest's `[source] path`
/// spells it: relative to a firmware entry, `/`-separated.
fn vsh_source_path() -> String {
    format!("{FLASH_MOUNT}/{VSH_MODULE_DIR}")
}

/// The system software's `[title] distribution` tag.
const VSH_DISTRIBUTION: &str = "firmware-exec";

/// The store root `--vfs-root` implies: the install root that encloses
/// the PS3 VFS root, where the read commands look too.
fn store_root_of(vfs_flag: Option<&Path>) -> Result<PathBuf, CommandError> {
    Ok(install_root_of(&super::title::resolve_ps3_vfs_root(
        vfs_flag,
    )?))
}

/// The install-record directory under `store_root`, where the
/// installers write.
///
/// The store preflight runs here because only this lookup resolves a
/// root: `--installs` names a record directory and `--record` names a
/// file.
fn installs_checked(store_root: &Path) -> Result<PathBuf, CommandError> {
    preflight(store_root)
        .map_err(|error| CommandError::failed(format!("gen-manifest failed: {error}")))?;
    Ok(StoreLayout::new(store_root).installs_dir())
}

/// The base record for `title_id` under an `installs/` directory.
///
/// `--installs` names the record directory itself, so the path is built
/// from the same relative arithmetic [`StoreLayout::record_path`] uses
/// below a VFS root.
fn base_record_under(installs: &Path, title_id: &str) -> Result<PathBuf, CommandError> {
    let title_id = TitleId::new(title_id)
        .map_err(|error| CommandError::failed(format!("gen-manifest --title-id: {error}")))?;
    Ok(installs.join(record_rel_path(&Artifact::TitleBase { title_id })))
}

/// The firmware record for `version` under an `installs/` directory.
///
/// See [`base_record_under`] for why the path is built relatively.
fn firmware_record_under(installs: &Path, version: &str) -> Result<PathBuf, CommandError> {
    let version = VersionKey::new(version)
        .map_err(|error| CommandError::failed(format!("gen-manifest --firmware: {error}")))?;
    Ok(installs.join(record_rel_path(&Artifact::Firmware { version })))
}

/// Escape a string for a double-quoted TOML basic string.
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(crate) fn run(
    args: &GenManifestArgs,
    vfs_flag: Option<&Path>,
) -> Result<CommandExitCode, CommandError> {
    // The closure resolves the root only where a read needs it: the
    // record directory a `--title-id` / `--firmware` lookup defaults
    // to, and the tree a title's `system_ver` comes from. One root
    // serves both, so a record and the floor written beside it never
    // come from two stores. A firmware record named by path resolves
    // none.
    let store_root = || store_root_of(vfs_flag);
    let (record_path, asked) = resolve_record_path(args, &store_root)?;
    let registry = args
        .registry
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_TITLE_REGISTRY_DIR));
    let force = args.force;

    let text = std::fs::read_to_string(&record_path).map_err(|error| {
        CommandError::failed(format!("failed to read {}: {error}", record_path.display()))
    })?;
    let record = InstallRecord::parse(&text).map_err(|error| {
        CommandError::failed(format!("parse {}: {error}", record_path.display()))
    })?;
    if let Some(refusal) = selector_mismatch(asked, &record, &record_path) {
        return Err(CommandError::failed(refusal));
    }

    let generated = Generated::from_record(&record, &record_path, store_root)?;
    let manifest_path = registry.join(format!("{}.toml", generated.content_id()));

    let stub = generated.render_stub(&record_path);
    // A record's `distribution` is a free-form string and its `title`
    // is PARAM.SFO text, so a stub can carry a field the manifest
    // loader refuses. The check runs ahead of the identity report as
    // well as the write, so both answer the same for every record.
    if let Err(e) = TitleManifest::load_from_text(&stub, &manifest_path) {
        return Err(CommandError::failed(format!(
            "the stub generated from {} is not a title manifest: {e}",
            record_path.display()
        )));
    }

    if manifest_path.exists() && !force {
        println!(
            "title manifest {} already exists; not overwriting (curated fields preserved).",
            manifest_path.display()
        );
        println!("  generated identity from {}:", record_path.display());
        generated.print_identity();
        println!("  reconcile by hand if any generated field drifted.");
        return Ok(CommandExitCode::SUCCESS);
    }

    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            CommandError::failed(format!("create {}: {error}", parent.display()))
        })?;
    }
    std::fs::write(&manifest_path, stub).map_err(|error| {
        CommandError::failed(format!("write {}: {error}", manifest_path.display()))
    })?;
    println!("wrote title-manifest stub {}", manifest_path.display());
    println!(
        "  fill in the curated fields ({}).",
        generated.curated_fields()
    );
    Ok(CommandExitCode::SUCCESS)
}

/// The record a selector names, and the kind that selector asked for.
///
/// `--record` names a path, so no kind is asked for and the record's
/// own kind is the answer. `--title-id` and `--firmware` each resolve
/// one kind's records directory; clap requires exactly one of the
/// three.
fn resolve_record_path(
    args: &GenManifestArgs,
    store_root: &impl Fn() -> Result<PathBuf, CommandError>,
) -> Result<(PathBuf, Option<ArtifactKind>), CommandError> {
    let lookup = |id: &str,
                  resolve: fn(&Path, &str) -> Result<PathBuf, CommandError>|
     -> Result<PathBuf, CommandError> {
        let installs = match &args.installs {
            Some(installs) => installs.clone(),
            None => installs_checked(&store_root()?)?,
        };
        resolve(&installs, id)
    };
    match (&args.record, &args.title_id, &args.firmware) {
        (Some(p), _, _) => Ok((p.clone(), None)),
        (None, Some(id), _) => Ok((
            lookup(id, base_record_under)?,
            Some(ArtifactKind::TitleBase),
        )),
        (None, None, Some(version)) => Ok((
            lookup(version, firmware_record_under)?,
            Some(ArtifactKind::Firmware),
        )),
        (None, None, None) => Err(CommandError::failed(
            "gen-manifest requires --record <path>, --title-id <id>, or --firmware <version>",
        )),
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
    /// Only a title record calls `store_root`, to find the installed
    /// tree that states its `system_ver`.
    fn from_record(
        record: &InstallRecord,
        record_path: &Path,
        store_root: impl FnOnce() -> Result<PathBuf, CommandError>,
    ) -> Result<Self, CommandError> {
        match (record.artifact.kind, record.title.as_ref()) {
            (ArtifactKind::Firmware, _) => Ok(Self::Firmware),
            // An update record names a title, but its `distribution`
            // is the update's own tag, which no title manifest holds.
            (ArtifactKind::TitleUpdate, _) => Err(CommandError::failed(format!(
                "{} describes a {} entry; a manifest is generated from the title's base record",
                record_path.display(),
                record.artifact.kind.as_str()
            ))),
            (ArtifactKind::TitleBase, Some(title)) => {
                let sfo = param_sfo_path(&store_root()?, record, title);
                let recorded = record.files.get(&param_sfo_rel(title));
                let system_ver = read_system_ver(&sfo, recorded, record_path)?;
                Ok(Self::Title(TitleFields::from_record(
                    record, title, system_ver,
                )))
            }
            (ArtifactKind::TitleBase, None) => Err(CommandError::failed(format!(
                "{} describes a {} entry, which names no title",
                record_path.display(),
                record.artifact.kind.as_str()
            ))),
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

/// The PARAM.SFO's path inside the installed base tree, spelled the way
/// the record's `[files]` keys it.
///
/// A disc tree holds it under `PS3_GAME/`; an HDD tree holds it at the
/// root.
fn param_sfo_rel(title: &TitleRecord) -> String {
    if title.distribution == DISC_DISTRIBUTION {
        format!("{DISC_GAME_DIR}/{PARAM_SFO_FILE}")
    } else {
        PARAM_SFO_FILE.to_string()
    }
}

/// The PARAM.SFO the installed base tree carries, resolved from the
/// record's `store_path` under `store_root`.
fn param_sfo_path(store_root: &Path, record: &InstallRecord, title: &TitleRecord) -> PathBuf {
    let mut path = StoreLayout::new(store_root).resolve_store_path(&record.artifact.store_path);
    path.extend(param_sfo_rel(title).split('/'));
    path
}

/// The `system_ver` the stub carries: the tree's `PS3_SYSTEM_VER` as a
/// firmware version key.
///
/// The record does not carry the floor, so a tree that cannot answer
/// refuses the generation by name. `recorded` is the digest the record
/// holds for this table, when its `[files]` lists one.
fn read_system_ver(
    sfo: &Path,
    recorded: Option<&Sha256>,
    record_path: &Path,
) -> Result<String, CommandError> {
    let bytes = std::fs::read(sfo).map_err(|error| {
        CommandError::failed(format!(
            "read {}: {e}; the stub's system_ver is the PS3_SYSTEM_VER this table states, so \
             the installed tree must be present under the store root (--vfs-root names it)",
            sfo.display(),
            e = error,
        ))
    })?;
    // The record digests every file it installed, and the uninstall
    // gate holds the tree to those digests. A table that hashes
    // differently belongs to some other install of this title id, so
    // its floor is not this record's.
    if let Some(recorded) = recorded {
        let found = Sha256(sha256_of(&bytes));
        if found.0 != recorded.0 {
            return Err(CommandError::failed(format!(
                "{}: SHA-256 {} is not the {} that {} recorded for it; the tree under the \
                 store root is not the one the record describes, so its {PS3_SYSTEM_VER_KEY} \
                 is not this record's floor (--vfs-root names the store root)",
                sfo.display(),
                found.to_hex(),
                recorded.to_hex(),
                record_path.display()
            )));
        }
    }
    let table = param_sfo::parse(&bytes)
        .map_err(|error| CommandError::failed(format!("{}: {error}", sfo.display())))?;
    let raw = table.get_string(PS3_SYSTEM_VER_KEY).ok_or_else(|| {
        CommandError::failed(format!(
            "{}: no {PS3_SYSTEM_VER_KEY} string; the stub's system_ver has nothing to derive from",
            sfo.display()
        ))
    })?;
    firmware_version_key(raw)
        .map_err(|error| CommandError::failed(format!("{}: {error}", sfo.display())))
}

/// The PARAM.SFO / install-derived fields of a title manifest.
struct TitleFields {
    content_id: String,
    display_name: String,
    distribution: String,
    eboot_candidate: String,
    rap_filename: Option<String>,
    /// The floor as a firmware version key; see [`read_system_ver`].
    system_ver: String,
}

impl TitleFields {
    fn from_record(record: &InstallRecord, title: &TitleRecord, system_ver: String) -> Self {
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
            system_ver,
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
        println!("    system_ver   = {}", self.system_ver);
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
             # distribution, rap_filename, system_ver) come from the install /\n\
             # PARAM.SFO. system_ver is the floor the title's own PARAM.SFO\n\
             # states and derives the one cell the headline row is measured at.\n\n",
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
        s.push_str(&format!(
            "system_ver = \"{}\"\n",
            toml_escape(&self.system_ver)
        ));
        // The loader's `[source]` default is hdd; a disc title left on
        // that default resolves under dev_hdd0 and never finds its EBOOT.
        if self.distribution == DISC_DISTRIBUTION {
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
