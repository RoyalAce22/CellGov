//! Boots the `spurs_initialize` probe against the installed firmware and
//! compares its result frame with the committed console capture.
//!
//! The probe links the firmware's cellSpurs library, so the boot needs
//! the firmware PRX set: the target requires `microtests` (the probe ELF
//! is built) and `installed-firmware-tests` (firmware 4.93 is
//! installed). A missing ELF or firmware is a hard error.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use std::path::PathBuf;
use std::process::Command;

use cellgov_compare::Observation;

#[path = "common/installed_firmware.rs"]
mod installed_firmware;

const NAME: &str = "spurs_initialize";

/// Size of the probe's result frame: 32 big-endian u32 words.
const FRAME_BYTES: usize = 128;

/// The SPU thread group id: the console numbers groups system-wide, so
/// two captures differ here.
const W_GROUP_ID: usize = 8;

/// Loading (w3) and unloading (w18) the SPURS system module succeed on
/// the console. CellGov refuses the load with 0x800120ff, so the unload
/// finds nothing loaded; the probe goes on past both.
const KNOWN_DIVERGENT: [usize; 2] = [3, 18];

fn probe_dir() -> PathBuf {
    installed_firmware::workspace_root()
        .join("tests")
        .join("micro")
        .join(NAME)
}

/// The payload after the first `CGOV` magic and its big-endian length.
fn frame(bytes: &[u8]) -> Vec<u32> {
    let at = bytes
        .windows(4)
        .position(|w| w == b"CGOV")
        .expect("a CGOV frame");
    let len = u32::from_be_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
    assert_eq!(len, FRAME_BYTES, "frame length");
    bytes[at + 8..at + 8 + len]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().unwrap()))
        .collect()
}

fn cellgov_frame() -> Vec<u32> {
    let elf = probe_dir()
        .join("build")
        .join("ps3")
        .join(format!("{NAME}.elf"));
    assert!(
        elf.is_file(),
        "{}: the microtests feature declares the probe built; run its build.sh",
        elf.display()
    );
    let scratch = installed_firmware::workspace_root()
        .join("target")
        .join("spurs_microtest_scratch")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&scratch).unwrap();
    let manifest = scratch.join("manifest.toml");
    std::fs::write(
        &manifest,
        format!(
            r#"[title]
content_id = "MT_SPURS_INITIALIZE"
short_name = "mt_spurs_initialize"
display_name = "microtest {NAME}"
eboot_candidates = ["{NAME}.elf"]
year = 2007
developer = "cellgov"
engine = "microtest"
distribution = "psn-hdd"
system_ver = "{ver}"

[checkpoint]
kind = "process-exit"
"#,
            ver = installed_firmware::TEST_SYSTEM_VERSION,
        ),
    )
    .unwrap();
    let observation = scratch.join("observation.json");
    let output = Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["boot", "run", "--title-manifest"])
        .arg(&manifest)
        .args(["--max-steps", "1000000", "--save-observation"])
        .arg(&observation)
        .arg("--firmware-dir")
        .arg(installed_firmware::firmware_external_dir())
        .arg(&elf)
        .current_dir(installed_firmware::workspace_root())
        .output()
        .expect("spawn cellgov boot run");
    assert!(
        output.status.success(),
        "boot run exited {:?}:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let observation: Observation =
        serde_json::from_str(&std::fs::read_to_string(&observation).unwrap()).unwrap();
    frame(&observation.tty_log)
}

#[test]
fn spurs_initialize_starts_both_handler_threads_as_on_the_console() {
    let console = frame(
        &std::fs::read(
            probe_dir()
                .join("ps3")
                .join("cech20-cex-493")
                .join("cgov_frame.bin"),
        )
        .unwrap(),
    );
    let cellgov = cellgov_frame();
    assert_eq!(
        cellgov[0], 0,
        "failed step {} code {:#010x}",
        cellgov[0], cellgov[1]
    );
    for (w, (&cg, &ps3)) in cellgov.iter().zip(&console).enumerate() {
        if w == W_GROUP_ID {
            assert_ne!(cg, 0, "CellGov names an SPU thread group");
        } else if !KNOWN_DIVERGENT.contains(&w) {
            assert_eq!(cg, ps3, "w{w}: CellGov {cg:#010x}, console {ps3:#010x}");
        }
    }
    assert_eq!([cellgov[14], cellgov[15]], [1, 1], "both handler threads");
}
