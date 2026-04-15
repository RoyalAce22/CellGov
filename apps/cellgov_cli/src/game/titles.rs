//! Named PS3 titles the CLI's game harness has specific configuration for.
//!
//! Every title-specific decision the CLI makes (EBOOT path, boot
//! checkpoint definition, VFS layout, firmware requirements) hangs
//! off [`Title`]. Title metadata lives only in `cellgov_cli`; no
//! library crate below knows titles exist. That boundary is what
//! keeps the oracle generalizable -- a downstream importer that
//! pulls `cellgov_ppu` or `cellgov_compare` does not transitively
//! pull a list of named games.

use std::path::{Path, PathBuf};

/// A named PS3 title for which the harness carries dedicated
/// configuration. New titles are added here and nowhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Title {
    /// thatgamecompany flOw (PSN, 2007). The reference title the
    /// oracle was bootstrapped against.
    Flow,
    /// Housemarque Super Stardust HD (PSN, 2007). The second title
    /// CellGov brings up; chosen for its standard SPURS usage and
    /// RPCS3 "Playable" compatibility status.
    Sshd,
}

/// Reasons the `--title` argument can fail to resolve to a [`Title`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleError {
    /// No `--title` flag was supplied on the command line.
    Missing,
    /// `--title` was supplied but its value does not match any known
    /// title. The payload is the offending value.
    Unknown(String),
}

/// Content-manifest metadata for a title: its PSN content id and the
/// candidate main-executable filenames inside `USRDIR/` to try in
/// priority order. Decrypted ELFs come first so encrypted SELFs are
/// only considered when no decrypted form is available.
#[derive(Debug, Clone, Copy)]
pub struct TitleContent {
    /// PSN content id (also the directory name under
    /// `/dev_hdd0/game/`). Stable per title.
    pub content_id: &'static str,
    /// Main executable filenames to try inside the title's
    /// `USRDIR/`, in priority order. The loader picks the first
    /// one that exists on disk.
    pub eboot_candidates: &'static [&'static str],
}

/// Per-title boot checkpoint definition. The harness stops at this
/// point to produce the observation that the cross-runner
/// comparator diffs against RPCS3.
///
/// flOw terminates on its own at `sys_process_exit`; titles whose
/// attract-mode loops never exit instead pick the first PPU write
/// to the RSX command region (`0xC0000000+`), which both runners
/// reach exactly once, deterministically, per boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointTrigger {
    /// Stop when the guest calls `sys_process_exit`.
    ProcessExit,
    /// Stop when the PPU attempts its first write into the RSX
    /// region. In CellGov that write faults with
    /// `MemError::ReservedWrite { region: "rsx", .. }`; the harness
    /// treats the fault as "checkpoint reached", not "boot broken".
    FirstRsxWrite,
}

impl Title {
    /// Every title the CLI knows about, in enumeration order. Stable
    /// so help text and error diagnostics list titles consistently.
    pub const ALL: &'static [Title] = &[Title::Flow, Title::Sshd];

    /// Parse the short CLI name (e.g. `"flow"`, `"sshd"`) into a
    /// [`Title`]. Returns `None` for unknown names. Names are
    /// case-sensitive; the CLI does not try to be clever.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.name() == name)
    }

    /// Short CLI name for this title. Used by `--title <name>` and
    /// by error diagnostics. Must be stable across releases.
    pub fn name(self) -> &'static str {
        match self {
            Title::Flow => "flow",
            Title::Sshd => "sshd",
        }
    }

    /// Human-readable label for diagnostics and log lines. Safe to
    /// change; not part of any contract.
    pub fn display_name(self) -> &'static str {
        match self {
            Title::Flow => "flOw (thatgamecompany, 2007)",
            Title::Sshd => "Super Stardust HD (Housemarque, 2007)",
        }
    }

    /// Content-manifest metadata for this title. See [`TitleContent`].
    pub fn content(self) -> TitleContent {
        // PS3 retail EBOOTs are SELFs; CellGov can only load the
        // decrypted ELF form, which must be produced once per title
        // via `rpcs3.exe --decrypt <path>/EBOOT.BIN`. Both names are
        // listed so that a VFS with only the encrypted form still
        // resolves (and then fails at load time with a clearer
        // diagnostic than "file not found").
        match self {
            Title::Flow => TitleContent {
                content_id: "NPUA80001",
                eboot_candidates: &["EBOOT.elf", "EBOOT.BIN"],
            },
            Title::Sshd => TitleContent {
                content_id: "NPUA80068",
                eboot_candidates: &["EBOOT.elf", "EBOOT.BIN"],
            },
        }
    }

    /// Boot checkpoint definition for this title. The step loop
    /// uses this to decide whether a particular halt condition is a
    /// "checkpoint reached" signal or a "boot broken" fault.
    pub fn checkpoint_trigger(self) -> CheckpointTrigger {
        match self {
            // flOw's boot reaches sys_process_exit deterministically
            // under CellGov, so its checkpoint is that syscall.
            Title::Flow => CheckpointTrigger::ProcessExit,
            // SSHD's attract-mode loop never exits on its own; the
            // first PPU write to the RSX region is the earliest
            // deterministic stopping point both runners reach.
            Title::Sshd => CheckpointTrigger::FirstRsxWrite,
        }
    }

    /// Build the conventional PS3 `USRDIR` path for this title under
    /// a VFS root (typically pointing at a `dev_hdd0` directory) and
    /// return the first candidate executable that exists on disk, or
    /// `None` if neither is present.
    pub fn resolve_eboot(self, vfs_root: &Path) -> Option<PathBuf> {
        let usrdir = vfs_root
            .join("game")
            .join(self.content().content_id)
            .join("USRDIR");
        for name in self.content().eboot_candidates {
            let p = usrdir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }

    /// Look up the `--title <name>` value in a raw args vector and
    /// resolve it to a [`Title`]. Returns [`TitleError::Missing`] if
    /// no `--title` was supplied and [`TitleError::Unknown`] if the
    /// supplied value is not a known title name.
    pub fn parse_from_args(args: &[String]) -> Result<Title, TitleError> {
        for i in 0..args.len() {
            if args[i] == "--title" {
                let value = args.get(i + 1).ok_or(TitleError::Missing)?.as_str();
                return Self::from_name(value)
                    .ok_or_else(|| TitleError::Unknown(value.to_string()));
            }
        }
        Err(TitleError::Missing)
    }

    /// Comma-separated list of all known title names, for use in
    /// error diagnostics. Stable ordering comes from [`Title::ALL`].
    pub fn known_names_csv() -> String {
        Self::ALL
            .iter()
            .map(|t| t.name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_titles_have_distinct_names() {
        let names: Vec<&str> = Title::ALL.iter().map(|t| t.name()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "title names must be unique");
    }

    #[test]
    fn from_name_round_trips_every_title() {
        for &t in Title::ALL {
            assert_eq!(Title::from_name(t.name()), Some(t));
        }
    }

    #[test]
    fn from_name_rejects_unknown() {
        assert_eq!(Title::from_name("nonesuch"), None);
        assert_eq!(Title::from_name(""), None);
    }

    #[test]
    fn from_name_is_case_sensitive() {
        assert_eq!(Title::from_name("FLOW"), None);
        assert_eq!(Title::from_name("Flow"), None);
        assert_eq!(Title::from_name("flow"), Some(Title::Flow));
    }

    #[test]
    fn parse_from_args_finds_title() {
        let args = vec![
            "cli".to_string(),
            "run-game".to_string(),
            "--title".to_string(),
            "flow".to_string(),
        ];
        assert_eq!(Title::parse_from_args(&args), Ok(Title::Flow));
    }

    #[test]
    fn parse_from_args_finds_title_after_other_flags() {
        let args = vec![
            "cli".to_string(),
            "run-game".to_string(),
            "EBOOT.elf".to_string(),
            "--max-steps".to_string(),
            "100".to_string(),
            "--title".to_string(),
            "sshd".to_string(),
        ];
        assert_eq!(Title::parse_from_args(&args), Ok(Title::Sshd));
    }

    #[test]
    fn parse_from_args_missing_title_returns_missing() {
        let args = vec!["cli".to_string(), "run-game".to_string()];
        assert_eq!(Title::parse_from_args(&args), Err(TitleError::Missing));
    }

    #[test]
    fn parse_from_args_missing_value_returns_missing() {
        let args = vec![
            "cli".to_string(),
            "run-game".to_string(),
            "--title".to_string(),
        ];
        assert_eq!(Title::parse_from_args(&args), Err(TitleError::Missing));
    }

    #[test]
    fn parse_from_args_unknown_title_returns_unknown() {
        let args = vec![
            "cli".to_string(),
            "run-game".to_string(),
            "--title".to_string(),
            "xyz".to_string(),
        ];
        assert_eq!(
            Title::parse_from_args(&args),
            Err(TitleError::Unknown("xyz".to_string()))
        );
    }

    #[test]
    fn known_names_csv_lists_all_titles_in_enum_order() {
        let csv = Title::known_names_csv();
        assert_eq!(csv, "flow, sshd");
    }

    #[test]
    fn display_name_differs_from_short_name() {
        for &t in Title::ALL {
            assert_ne!(t.name(), t.display_name());
        }
    }

    #[test]
    fn every_title_has_a_content_manifest() {
        for &t in Title::ALL {
            let c = t.content();
            assert!(!c.content_id.is_empty(), "{} content_id empty", t.name());
            assert!(
                !c.eboot_candidates.is_empty(),
                "{} has no eboot candidates",
                t.name()
            );
        }
    }

    #[test]
    fn flow_content_id_is_npua80001() {
        assert_eq!(Title::Flow.content().content_id, "NPUA80001");
    }

    #[test]
    fn sshd_content_id_is_npua80068() {
        assert_eq!(Title::Sshd.content().content_id, "NPUA80068");
    }

    #[test]
    fn resolve_eboot_returns_none_when_vfs_empty() {
        let tmp = std::env::temp_dir().join("cellgov_titles_resolve_empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        assert_eq!(Title::Flow.resolve_eboot(&tmp), None);
        assert_eq!(Title::Sshd.resolve_eboot(&tmp), None);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_eboot_finds_first_candidate() {
        // Build a fake VFS root with only SSHD's EBOOT.BIN present
        // (no decrypted ELF); resolver should still return the BIN
        // because both names are in the candidate list.
        let tmp = std::env::temp_dir().join("cellgov_titles_resolve_bin");
        let _ = std::fs::remove_dir_all(&tmp);
        let usrdir = tmp.join("game").join("NPUA80068").join("USRDIR");
        std::fs::create_dir_all(&usrdir).unwrap();
        let bin = usrdir.join("EBOOT.BIN");
        std::fs::write(&bin, b"fake").unwrap();
        assert_eq!(Title::Sshd.resolve_eboot(&tmp), Some(bin));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn flow_checkpoint_is_process_exit() {
        assert_eq!(
            Title::Flow.checkpoint_trigger(),
            CheckpointTrigger::ProcessExit
        );
    }

    #[test]
    fn sshd_checkpoint_is_first_rsx_write() {
        assert_eq!(
            Title::Sshd.checkpoint_trigger(),
            CheckpointTrigger::FirstRsxWrite
        );
    }

    #[test]
    fn every_title_declares_a_checkpoint_trigger() {
        // Non-exhaustive guard: iterating Title::ALL and calling
        // checkpoint_trigger() forces every variant to have a mapping
        // without the match in checkpoint_trigger needing a wildcard.
        for &t in Title::ALL {
            let _ = t.checkpoint_trigger();
        }
    }

    #[test]
    fn resolve_eboot_prefers_earlier_candidate() {
        // When both EBOOT.elf and EBOOT.BIN exist, the decrypted ELF
        // wins because it is listed first.
        let tmp = std::env::temp_dir().join("cellgov_titles_resolve_prefer");
        let _ = std::fs::remove_dir_all(&tmp);
        let usrdir = tmp.join("game").join("NPUA80068").join("USRDIR");
        std::fs::create_dir_all(&usrdir).unwrap();
        let elf = usrdir.join("EBOOT.elf");
        let bin = usrdir.join("EBOOT.BIN");
        std::fs::write(&elf, b"fake-elf").unwrap();
        std::fs::write(&bin, b"fake-bin").unwrap();
        assert_eq!(Title::Sshd.resolve_eboot(&tmp), Some(elf));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
