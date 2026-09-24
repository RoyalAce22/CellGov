//! The selection banner: what this run tests, named before the run.
//!
//! The banner writes to stderr before any other output. Its lines never
//! land on stdout beside the measurements a consumer parses.

use cellgov_boot::compose::{BootComposition, FirmwareChoice, GameChoice, UnderstatedFirmware};
use cellgov_boot::manifest::TitleManifest;
use cellgov_install::store::select::GameVersion;

use super::refusal::selected_by_label;

/// A digest as `head..tail`, enough to tell two installs apart at a
/// glance without carrying 64 characters across three lines.
fn short_digest(hex: &str) -> String {
    if hex.len() <= 8 {
        return hex.to_string();
    }
    format!("{}..{}", &hex[..4], &hex[hex.len() - 4..])
}

/// Render the banner for a composed boot.
///
/// Returns the lines in print order.
pub(crate) fn render(title: &TitleManifest, composition: &BootComposition) -> Vec<String> {
    let mut lines = vec![
        format!(
            "title    {}  {}  {}",
            title.name(),
            title.content_id,
            title.display_name()
        ),
        render_game(composition),
        render_firmware(&composition.firmware),
    ];
    lines.extend(composition.identity.render_lines().into_iter().skip(2));
    lines
}

fn render_game(composition: &BootComposition) -> String {
    match &composition.game {
        GameChoice::Stored(stored) => {
            let (version, base, update) = (&stored.version, &stored.base, &stored.update);
            let detail = match (version, update) {
                (GameVersion::Update(_), Some(u)) => format!(
                    "(base {}, {}; update sha256 {})",
                    base.version,
                    base.distribution,
                    short_digest(&u.source_sha256),
                ),
                _ => format!(
                    "({}; base sha256 {})",
                    base.distribution,
                    short_digest(&base.source_sha256),
                ),
            };
            format!("game     {version}  {detail}")
        }
        GameChoice::Firmware {
            dir,
            unmanaged_path,
        } => {
            let tag = if *unmanaged_path {
                "  (path: unmanaged)"
            } else {
                ""
            };
            format!("game     firmware-exec  {}{tag}", dir.display())
        }
        GameChoice::Unstored => {
            "game     no store entry  (content resolved from the VFS root)".to_string()
        }
    }
}

fn render_firmware(firmware: &FirmwareChoice) -> String {
    match firmware {
        FirmwareChoice::Managed(managed) => format!(
            "firmware {}  ({}; pup sha256 {})",
            managed.entry.version,
            selected_by_label(managed.selected_by),
            short_digest(&managed.entry.pup_sha256)
        ),
        FirmwareChoice::Unmanaged { dir } => {
            format!("firmware unmanaged  {}", dir.display())
        }
        FirmwareChoice::None => "firmware none  (imports answer through trampolines)".to_string(),
    }
}

/// One line per composed entry whose declared minimum firmware the
/// selection does not meet. The caller prints them after the banner,
/// on the same stream.
pub(crate) fn render_firmware_notes(notes: &[UnderstatedFirmware]) -> Vec<String> {
    notes
        .iter()
        .map(|n| {
            if n.incomparable {
                format!(
                    "warning: {} declares system version {:?}, which does not compare with \
                     the selected firmware {:?}",
                    n.entry, n.declared, n.selected,
                )
            } else {
                format!(
                    "warning: {} declares system version {}, and the selected firmware {} is \
                     older",
                    n.entry, n.declared, n.selected,
                )
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/banner_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/banner_override_tests.rs"]
mod override_tests;
