//! `gen-manifest`: turn a game-install record into a title-manifest
//! stub, closing the loop from "installed a PKG/ISO" to "registered,
//! bootable title".
//!
//! The PARAM.SFO-derived fields (`content_id`, `display_name`,
//! `eboot_candidates`, `distribution`, `rap_filename`) are filled from
//! the install record; the curated fields (`short_name`, `year`,
//! `developer`, `engine`, `rsx`, `content`, `mounts`) are written as
//! placeholders for an author to fill. An existing manifest is never
//! overwritten -- its curated fields are preserved -- so this only
//! writes a stub for a brand-new title and otherwise reports the
//! generated identity for the user to reconcile.

use std::path::{Path, PathBuf};

use cellgov_install::store::{
    record_rel_path, Artifact, InstallRecord, StoreLayout, TitleId, TitleRecord, DEFAULT_VFS_ROOT,
};

use crate::cli::args::{find_flag_value, has_bool_flag, reject_flag_here, require_at_most_one};
use crate::cli::exit::die;

const DEFAULT_REGISTRY: &str = "titles";

/// Records for the default VFS root, matching where `cellgov_install`
/// writes them. `--installs` names the directory directly, for a VFS
/// rooted elsewhere.
fn default_installs() -> PathBuf {
    StoreLayout::new(DEFAULT_VFS_ROOT).installs_dir()
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

/// Escape a string for a double-quoted TOML basic string.
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(crate) fn run(args: &[String]) {
    // The shared parsers, not a local scan: they refuse `--flag=value`
    // and a duplicate, both of which a bare `position(|a| a == name)`
    // reads as absent -- a `--force=1` that read as absent left the
    // existing manifest in place and still exited 0.
    require_at_most_one(args, &["--record", "--title-id"]);
    let record_flag = find_flag_value(args, "--record");
    let title_id = find_flag_value(args, "--title-id");
    let record_path = match (record_flag, title_id) {
        (Some(p), _) => {
            // --installs only names the directory a --title-id is
            // resolved under; an explicit record path is already whole.
            reject_flag_here(args, "--installs", "a --title-id lookup");
            PathBuf::from(p)
        }
        (None, Some(id)) => {
            let installs =
                find_flag_value(args, "--installs").map_or_else(default_installs, PathBuf::from);
            base_record_under(&installs, &id)
        }
        (None, None) => die("gen-manifest requires --record <path> or --title-id <id>"),
    };
    let registry = PathBuf::from(
        find_flag_value(args, "--registry").unwrap_or_else(|| DEFAULT_REGISTRY.to_string()),
    );
    let force = has_bool_flag(args, "--force");

    let text = std::fs::read_to_string(&record_path)
        .unwrap_or_else(|e| die(&format!("failed to read {}: {e}", record_path.display())));
    let record = InstallRecord::parse(&text)
        .unwrap_or_else(|e| die(&format!("parse {}: {e}", record_path.display())));

    let title = record.title.as_ref().unwrap_or_else(|| {
        die(&format!(
            "{} describes a {} entry, which names no title",
            record_path.display(),
            record.artifact.kind.as_str()
        ))
    });
    let gen = GeneratedFields::from_record(&record, title);
    let manifest_path = registry.join(format!("{}.toml", gen.content_id));

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

    let stub = gen.render_stub(&record_path);
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| die(&format!("create {}: {e}", parent.display())));
    }
    std::fs::write(&manifest_path, stub)
        .unwrap_or_else(|e| die(&format!("write {}: {e}", manifest_path.display())));
    println!("wrote title-manifest stub {}", manifest_path.display());
    println!("  fill in the curated fields (short_name, year, developer, engine, rsx).");
}

/// The PARAM.SFO / install-derived fields of a title manifest.
struct GeneratedFields {
    content_id: String,
    display_name: String,
    distribution: String,
    eboot_candidate: String,
    rap_filename: Option<String>,
}

impl GeneratedFields {
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
            "# Generated by `cellgov_cli gen-manifest` from {}.\n",
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

#[cfg(test)]
#[path = "tests/gen_manifest_tests.rs"]
mod tests;
