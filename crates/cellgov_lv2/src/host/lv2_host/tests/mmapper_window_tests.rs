//! Where a game process's 256 MiB areas land before and after the RSX
//! context window opens, held to the `lv2_mmapper_rsx_window` capture.

use super::*;
use crate::host::rsx::test_helpers::context_allocate_request;
use crate::host::test_support::{console_payload, FakeRuntime};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;

const AREA: u32 = 0x1000_0000;
const BEFORE_BASES: usize = 2;
const BEFORE_RCS: usize = 10;
const AFTER_BASES: usize = 18;
const TRIES: usize = 8;

fn words() -> Vec<u64> {
    let payload = console_payload("lv2_mmapper_rsx_window");
    assert_eq!(payload.len(), 34 * 8);
    payload
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// Every base `mmapper_alloc` grants until it refuses.
fn grants(host: &mut Lv2Host) -> Vec<u64> {
    let mut bases = Vec::new();
    while let Some(base) = host.mmapper_alloc(BOOT_PROCESS_PID, AREA, 0x10_0000) {
        bases.push(u64::from(base));
        assert!(bases.len() <= TRIES, "more grants than the probe tried");
    }
    bases
}

#[test]
fn without_a_context_the_grants_are_the_console_s() {
    let w = words();
    let console: Vec<u64> = w[BEFORE_BASES..BEFORE_BASES + TRIES]
        .iter()
        .copied()
        .take_while(|&b| b != 0)
        .collect();
    assert_eq!(
        w[BEFORE_RCS + console.len()],
        u64::from(errno::CELL_ENOMEM),
        "the console refuses the next one"
    );
    assert_eq!(grants(&mut Lv2Host::new()), console);
}

/// After rsxInit the console skips the window's granule, and also the
/// lowest one, which the 1 MiB-page host area libgcm maps holds there.
/// CellGov places that host area elsewhere, so its grants keep the
/// lowest granule.
#[test]
fn with_a_context_the_window_s_granule_is_skipped_as_on_the_console() {
    let w = words();
    assert_eq!(w[0], 0, "rsxInit succeeded on the console");
    let console: Vec<u64> = w[AFTER_BASES..AFTER_BASES + TRIES]
        .iter()
        .copied()
        .take_while(|&b| b != 0)
        .collect();
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1_0000);
    let _ = host.dispatch(
        context_allocate_request(0x1000, 0x1008, 0x1010, 0x1018, 1),
        UnitId::new(0),
        &rt,
    );
    let cellgov = grants(&mut host);
    assert_eq!(cellgov[0], u64::from(Lv2Host::MMAPPER_REGION_START));
    assert!(!console.contains(&cellgov[0]), "the console's host area");
    assert_eq!(cellgov[1..], console[..]);
}
