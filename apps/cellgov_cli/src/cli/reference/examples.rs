//! The invocations every leaf command's help leads with.
//!
//! The same table feeds `--help` and `docs/cli.md`.

/// One command's example block.
pub(crate) struct Examples {
    /// Space-separated path below `cellgov`; empty for the root.
    pub path: &'static str,
    /// Invocations, each a complete command line.
    pub lines: &'static [&'static str],
}

/// Upper bound on one command's example block: a leaf leads with 1-3
/// invocations.
#[cfg(test)]
pub(crate) const MAX_EXAMPLES: usize = 3;

/// Every command that leads its help with invocations: the root, and
/// each leaf. A noun's help is its verb list, so nouns have no entry.
pub(crate) const EXAMPLES: &[Examples] = &[
    Examples {
        path: "",
        lines: &[
            "cellgov title install dumps/NPUA80001/flow.pkg --rap dumps/NPUA80001/flow.rap",
            "cellgov boot bench --title flow --fw 4.93",
            "cellgov diff observations cellgov.json rpcs3.json",
        ],
    },
    Examples {
        path: "status",
        lines: &["cellgov status", "cellgov status --format json"],
    },
    Examples {
        path: "firmware install",
        lines: &[
            "cellgov firmware install dumps/firmware/PS3UPDAT.PUP",
            "cellgov firmware install dumps/firmware/PS3UPDAT.PUP --force --verbose",
            "cellgov firmware install dumps/firmware/PS3UPDAT.PUP --kernel-only",
        ],
    },
    Examples {
        path: "firmware list",
        lines: &[
            "cellgov firmware list",
            "cellgov firmware list --format json",
        ],
    },
    Examples {
        path: "firmware show",
        lines: &["cellgov firmware show 4.93"],
    },
    Examples {
        path: "firmware verify",
        lines: &[
            "cellgov firmware verify 4.93",
            "cellgov firmware verify 4.93 --format json",
        ],
    },
    Examples {
        path: "firmware verify-pups",
        lines: &["cellgov firmware verify-pups dumps/firmware"],
    },
    Examples {
        path: "firmware kernels",
        lines: &[
            "cellgov firmware kernels",
            "cellgov firmware kernels --format json",
        ],
    },
    Examples {
        path: "firmware uninstall",
        lines: &[
            "cellgov firmware uninstall 4.93 --dry-run",
            "cellgov firmware uninstall 4.93 --verify",
        ],
    },
    Examples {
        path: "title install",
        lines: &[
            "cellgov title install dumps/NPUA80001/flow.pkg --rap dumps/NPUA80001/flow.rap",
            "cellgov title install dumps/BCES00664/wipeout.iso",
        ],
    },
    Examples {
        path: "title install-update",
        lines: &["cellgov title install-update dumps/NPUA80068/sshd-1.02.pkg"],
    },
    Examples {
        path: "title list",
        lines: &["cellgov title list", "cellgov title list --format json"],
    },
    Examples {
        path: "title show",
        lines: &["cellgov title show NPUA80001"],
    },
    Examples {
        path: "title verify",
        lines: &[
            "cellgov title verify NPUA80001",
            "cellgov title verify NPUA80068 --ver 1.02",
        ],
    },
    Examples {
        path: "title uninstall",
        lines: &[
            "cellgov title uninstall NPUA80001 --all --dry-run",
            "cellgov title uninstall NPUA80068 --ver 1.02",
        ],
    },
    Examples {
        path: "keys show",
        lines: &["cellgov keys show"],
    },
    Examples {
        path: "keys import",
        lines: &[
            "cellgov keys import dumps/keys/",
            "cellgov keys import dumps/keys/keys.toml --replace",
        ],
    },
    Examples {
        path: "keys remove",
        lines: &["cellgov keys remove"],
    },
    Examples {
        path: "self decrypt",
        lines: &["cellgov self decrypt vfs/dev_flash/sys/external/liblv2.sprx --output liblv2.elf"],
    },
    Examples {
        path: "boot run",
        lines: &[
            "cellgov boot run --title flow --fw 4.93",
            "cellgov boot run --title sshd --fw 4.93 --max-steps 200000 --prescan",
            "cellgov boot run --title wipeout --fw 4.93 --trace --save-state-trace wipeout.state",
        ],
    },
    Examples {
        path: "boot bench",
        lines: &[
            "cellgov boot bench --title flow --fw 4.93",
            "cellgov boot bench --title wipeout --fw 4.93 --runs 5 --no-anchor-check",
            "cellgov boot bench --all --runs 1",
        ],
    },
    Examples {
        path: "boot bench-once",
        lines: &["cellgov boot bench-once --title flow --fw 4.93 --max-steps 50000"],
    },
    Examples {
        path: "diff compare",
        lines: &[
            "cellgov diff compare fairness",
            "cellgov diff compare fairness --save-baseline fairness.baseline.json",
            "cellgov diff compare fairness --against-baseline fairness.baseline.json --mode strict",
        ],
    },
    Examples {
        path: "diff observations",
        lines: &["cellgov diff observations cellgov.json rpcs3.json"],
    },
    Examples {
        path: "diff diverge",
        lines: &["cellgov diff diverge cellgov.state rpcs3.state"],
    },
    Examples {
        path: "diff zoom",
        lines: &["cellgov diff zoom cellgov.zoom.state rpcs3.zoom.state 0x1f4"],
    },
    Examples {
        path: "explore",
        lines: &[
            "cellgov explore fairness",
            "cellgov explore dma --format json",
        ],
    },
    Examples {
        path: "explore micro",
        lines: &[
            "cellgov explore micro barrier_wakeup",
            "cellgov explore micro mailbox_roundtrip --observations-dir observations/",
        ],
    },
    Examples {
        path: "explore title",
        lines: &[
            "cellgov explore title --title flow --fw 1.50",
            "cellgov explore title --title sshd --fw 4.93 --start-step 20000",
            "cellgov explore title --title wipeout --fw 4.93 --max-schedules 32 --format json",
        ],
    },
    Examples {
        path: "scenario list",
        lines: &["cellgov scenario list"],
    },
    Examples {
        path: "scenario run",
        lines: &["cellgov scenario run fairness"],
    },
    Examples {
        path: "scenario dump",
        lines: &["cellgov scenario dump dma"],
    },
    Examples {
        path: "dev disasm",
        lines: &[
            "cellgov dev disasm vfs/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN --vaddr 0x10381ce8",
            "cellgov dev disasm dumps/flow.elf --vaddr 0x10000 --count 64 --symbolize",
        ],
    },
    Examples {
        path: "dev prx-imports",
        lines: &[
            "cellgov dev prx-imports vfs/dev_flash/sys/external/libsysmodule.sprx",
            "cellgov dev prx-imports dumps/EBOOT.BIN --module cellGcmSys",
        ],
    },
    Examples {
        path: "dev funcs",
        lines: &[
            "cellgov dev funcs vfs/dev_flash/sys/external/liblv2.sprx",
            "cellgov dev funcs dumps/EBOOT.BIN --json",
        ],
    },
    Examples {
        path: "dev lv2-discover",
        lines: &[
            "cellgov dev lv2-discover ../cellgov-output/lv2_kernel-3.55.elf",
            "cellgov dev lv2-discover ../cellgov-output/lv2_kernel-3.55.elf --format json",
        ],
    },
    Examples {
        path: "dev lv2-census",
        lines: &[
            "cellgov dev lv2-census ../cellgov-output/lv2_kernel-3.55.elf --fw 3.55 --pup-sha256 334e60a4ef5843a688c1c6aebf0951c3259429233c7a5aca5a24f0edad78a192",
            "cellgov dev lv2-census ../cellgov-output/lv2_kernel-3.55.elf --fw 3.55 --pup-sha256 334e60a4ef5843a688c1c6aebf0951c3259429233c7a5aca5a24f0edad78a192 --replace-version",
        ],
    },
    #[cfg(feature = "decrypt")]
    Examples {
        path: "dev lv2-extract",
        lines: &[
            "cellgov dev lv2-extract --fw 4.93 --output-dir ../cellgov-output",
            "cellgov dev lv2-extract --output-dir ../cellgov-output --format json",
        ],
    },
    #[cfg(feature = "decrypt")]
    Examples {
        path: "dev caller-census",
        lines: &[
            "cellgov dev caller-census --fw 4.93 --output-dir ../caller-census",
            "cellgov dev caller-census --all --output-dir docs/lv2",
        ],
    },
    Examples {
        path: "dev rpcs3-attribute",
        lines: &[
            "cellgov dev rpcs3-attribute --trace hle.trace --addr 0x10000000 --len 0x40",
            "cellgov dev rpcs3-attribute --trace hle.trace --ranked",
        ],
    },
    Examples {
        path: "dev fixture-gen",
        lines: &[
            "cellgov dev fixture-gen --manifest title_manifests/NPUA80001.toml --cellgov cellgov.json \
             --rpcs3 rpcs3.json --fw 4.93 --game-ver base",
        ],
    },
    Examples {
        path: "dev titles-gen",
        lines: &["cellgov dev titles-gen"],
    },
    Examples {
        path: "dev cli-gen",
        lines: &["cellgov dev cli-gen"],
    },
    Examples {
        path: "dev workspace-gen",
        lines: &["cellgov dev workspace-gen"],
    },
    Examples {
        path: "dev completions",
        lines: &[
            "cellgov dev completions bash",
            "cellgov dev completions pwsh",
        ],
    },
    Examples {
        path: "dev gen-manifest",
        lines: &[
            "cellgov dev gen-manifest --title-id NPUA80001",
            "cellgov dev gen-manifest --firmware 4.93",
            "cellgov dev gen-manifest --record \
             vfs/.cellgov/installs/titles/NPUA80001/base.install.toml --force",
        ],
    },
    Examples {
        path: "dev record-anchors",
        lines: &[
            "cellgov dev record-anchors --title flow",
            "cellgov dev record-anchors --all --fw 4.93",
        ],
    },
    Examples {
        path: "dev oracle-gap",
        lines: &["cellgov dev oracle-gap"],
    },
    Examples {
        path: "dev fuzz ppu-instruction",
        lines: &[
            "cellgov dev fuzz ppu-instruction --seed 7 --count 1000",
            "cellgov dev fuzz ppu-instruction --quiet --progress --count 100",
        ],
    },
    Examples {
        path: "dev fuzz ppu-sequence",
        lines: &["cellgov dev fuzz ppu-sequence --seed 7 --count 100 --sequence-words 16"],
    },
    Examples {
        path: "dev fuzz spu-instruction",
        lines: &["cellgov dev fuzz spu-instruction --seed 7 --count 1000"],
    },
    Examples {
        path: "dev fuzz spu-sequence",
        lines: &["cellgov dev fuzz spu-sequence --seed 7 --count 100 --sequence-words 16"],
    },
    Examples {
        path: "dev fuzz semantic",
        lines: &["cellgov dev fuzz semantic both"],
    },
    Examples {
        path: "dev fuzz raw",
        lines: &[
            "cellgov dev fuzz raw ppu --count 65536 --output ppu-bounded.json",
            "cellgov dev fuzz raw spu --full --shard 0 --shards 16 --output spu-shard0.json",
        ],
    },
];

/// The block `path`'s help leads with, or `None` when it declares none.
///
/// # Panics
///
/// Debug builds panic when [`EXAMPLES`] holds:
///
/// - a second entry for `path`
/// - an entry for `path` with no line to show
///
/// The lookup answers with the first entry, so neither the second entry
/// nor an empty line reaches a reader.
pub(crate) fn lines_for(path: &str) -> Option<&'static [&'static str]> {
    let mut hits = EXAMPLES.iter().filter(|e| e.path == path);
    let first = hits.next()?;
    debug_assert!(
        hits.next().is_none(),
        "EXAMPLES holds a second entry for {path:?} that no lookup reaches"
    );
    debug_assert!(
        !first.lines.is_empty() && first.lines.iter().all(|l| !l.trim().is_empty()),
        "EXAMPLES entry {path:?} holds nothing to show"
    );
    Some(first.lines)
}

/// The examples block as help text: a heading and one indented line per
/// invocation.
///
/// The block ends with no trailing newline. Clap's `{before-help}` slot
/// supplies the blank line that follows.
pub(crate) fn block(lines: &[&str]) -> String {
    let mut out = String::from("Examples:");
    for line in lines {
        out.push_str("\n  ");
        out.push_str(line);
    }
    out
}
