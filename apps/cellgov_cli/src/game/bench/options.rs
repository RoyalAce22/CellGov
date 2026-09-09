//! Inputs every `boot bench` entry point takes, and the `boot
//! bench-once` argv a run set forwards to each child.

use cellgov_time::Budget;

use crate::game::manifest::{self, CellKey, TitleManifest};

/// Subprocess measurements one `boot bench` invocation takes.
///
/// The determinism gate is exact at any count above one. The
/// throughput half fixes the count: min-of-N is the estimator there,
/// and two samples cannot outvote a single descheduling event.
pub const BENCH_DEFAULT_RUNS: usize = 3;

/// The selection flags a run resolved its composition from.
///
/// The parent of a run set forwards these flags to every child, so
/// each process composes from the same store.
#[derive(Debug, Clone, Copy, Default)]
pub struct SelectionArgs<'a> {
    pub fw: Option<&'a str>,
    pub game_ver: Option<&'a str>,
    /// An unmanaged firmware tree, which bypasses the store.
    pub firmware_dir: Option<&'a str>,
    /// The root the CLI reads the store from.
    pub vfs_root: Option<&'a str>,
}

/// The cell a run is held against, and the two parameters the registry
/// fixes for it.
#[derive(Debug, Clone, Copy)]
pub struct AnchorPlan<'a> {
    /// `None` when the composition names no cell:
    ///
    /// - an unmanaged firmware tree carries no version;
    /// - a title the store does not hold has no game-version axis.
    pub cell: Option<&'a CellKey>,
    /// Instruction cap the cell's anchor is recorded under.
    pub max_steps: u64,
    /// Checkpoint the cell's anchor is recorded under.
    pub checkpoint: manifest::CheckpointTrigger,
}

/// Inputs common to every `boot bench` entry point.
#[derive(Debug, Clone, Copy)]
pub struct BenchOptions<'a> {
    pub title: &'a TitleManifest,
    pub elf_path: &'a str,
    pub max_steps: usize,
    /// What the registry declares for the cell this run composes.
    pub plan: AnchorPlan<'a>,
    /// The `sys/external` directory the firmware loader reads.
    pub firmware_dir: Option<&'a str>,
    pub composed_mounts: &'a [crate::composition::ComposedMount],
    /// The triple this run is measured against.
    pub identity: &'a cellgov_compare::RunIdentity,
    /// What the child re-resolves its own composition from.
    pub selection: SelectionArgs<'a>,
    pub strict_reserved: bool,
    pub checkpoint_override: Option<manifest::CheckpointTrigger>,
    pub budget_override: Option<Budget>,
    /// When true, scan the title ELF for unimplemented PPU
    /// encodings before execution and print the gap report.
    pub prescan: bool,
    /// Guest argv for the primary thread, `argv[0]` included. Empty
    /// keeps the no-args entry state (r3..r6 = 0).
    pub guest_args: &'a [String],
    /// Compare the run against the cell's committed anchor. Cleared
    /// by `--no-anchor-check` for the re-record workflow, where the
    /// anchor is expected to disagree.
    pub check_anchor: bool,
    /// Travels to the child, which reports it back on its
    /// `BENCH_RESULT` line.
    pub run_index: usize,
}

impl BenchOptions<'_> {
    /// Whether an override moves this run off the trajectory its
    /// cell's anchor recorded.
    ///
    /// The cap is a ceiling the run may hit short of the anchor, so it
    /// is no retarget.
    pub(super) fn retargets_trajectory(&self) -> bool {
        self.checkpoint_override
            .is_some_and(|cp| cp != self.plan.checkpoint)
            || self.budget_override.is_some()
            || self.strict_reserved
            || !self.guest_args.is_empty()
    }

    /// Append the `boot bench-once` CLI form of this struct onto `cmd`.
    pub(super) fn encode_to_command(&self, cmd: &mut std::process::Command) {
        cmd.arg("boot")
            .arg("bench-once")
            // The parent captures the child's streams and replays them
            // after exit, so a child bar would render into a pipe.
            .arg("--no-progress")
            .arg("--title")
            .arg(self.title.name())
            .arg("--max-steps")
            .arg(self.max_steps.to_string())
            .arg("--run-index")
            .arg(self.run_index.to_string());
        for (flag, value) in [
            ("--vfs-root", self.selection.vfs_root),
            ("--fw", self.selection.fw),
            ("--game-ver", self.selection.game_ver),
            ("--firmware-dir", self.selection.firmware_dir),
        ] {
            if let Some(v) = value {
                cmd.arg(flag).arg(v);
            }
        }
        if self.strict_reserved {
            cmd.arg("--strict-reserved");
        }
        if let Some(cp) = self.checkpoint_override {
            cmd.arg("--checkpoint").arg(cp.as_cli_str());
        }
        if let Some(b) = self.budget_override {
            cmd.arg("--budget").arg(b.raw().to_string());
        }
        if self.prescan {
            cmd.arg("--prescan");
        }
        for arg in self.guest_args {
            cmd.arg("--guest-arg").arg(arg);
        }
    }
}

#[cfg(test)]
#[path = "tests/options_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/retarget_tests.rs"]
mod retarget_tests;
