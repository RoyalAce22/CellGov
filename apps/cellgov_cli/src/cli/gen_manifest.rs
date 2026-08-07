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

use cellgov_install::game_install::InstallRecord;

use crate::cli::exit::die;

const DEFAULT_REGISTRY: &str = "docs/title_manifests";
const DEFAULT_INSTALLS: &str = "installs";

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// Escape a string for a double-quoted TOML basic string.
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(crate) fn run(args: &[String]) {
    let record_path = match (flag(args, "--record"), flag(args, "--title-id")) {
        (Some(p), _) => PathBuf::from(p),
        (None, Some(id)) => PathBuf::from(flag(args, "--installs").unwrap_or(DEFAULT_INSTALLS))
            .join(format!("{id}.install.toml")),
        (None, None) => die("gen-manifest requires --record <path> or --title-id <id>"),
    };
    let registry = PathBuf::from(flag(args, "--registry").unwrap_or(DEFAULT_REGISTRY));
    let force = has_flag(args, "--force");

    let text = std::fs::read_to_string(&record_path)
        .unwrap_or_else(|e| die(&format!("failed to read {}: {e}", record_path.display())));
    let record: InstallRecord = toml::from_str(&text)
        .unwrap_or_else(|e| die(&format!("parse {}: {e}", record_path.display())));

    let gen = GeneratedFields::from_record(&record);
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
    fn from_record(record: &InstallRecord) -> Self {
        // The manifest's `content_id` directory key holds the title-id
        // value (a pre-existing field-name misnomer); the RAP, by
        // contrast, is keyed by the full NPD content id.
        let dir_key = record.title.title_id.clone();
        let eboot_candidate = record
            .files
            .keys()
            .find(|p| p.ends_with("EBOOT.BIN"))
            .and_then(|p| p.rsplit('/').next())
            .unwrap_or("EBOOT.BIN")
            .to_string();
        // An NPDRM HDD title carries a full NPD content id distinct from
        // the title-id; its RAP is named by that content id. Disc and
        // bare-title-id records have no RAP.
        let rap_filename = (record.title.distribution == "psn-hdd"
            && record.title.content_id != dir_key)
            .then(|| format!("{}.rap", record.title.content_id));
        Self {
            content_id: dir_key,
            display_name: record.title.title.clone(),
            distribution: record.title.distribution.clone(),
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
        s.push_str("\n[checkpoint]\nkind = \"process-exit\"\n");
        s.push_str("\n[rsx]\nmirror = false\nconsume = false\n");
        s
    }
}

#[cfg(test)]
#[path = "tests/gen_manifest_tests.rs"]
mod tests;
