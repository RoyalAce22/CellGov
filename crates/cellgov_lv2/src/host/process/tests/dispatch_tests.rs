//! `sys_process_get_sdk_version` dispatch tests: sentinel default and plumb-through of the version set at process load.

use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::elf::SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN;

fn captured_version(host: &Lv2Host) -> u32 {
    match host.dispatch_process_get_sdk_version(0x1000, UnitId::new(0)) {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0, "sc 25 must return code 0");
            assert_eq!(effects.len(), 1, "sc 25 emits one shared write");
            let Effect::SharedWriteIntent { bytes, .. } = &effects[0] else {
                panic!("expected SharedWriteIntent, got {:?}", effects[0]);
            };
            let payload = bytes.bytes();
            assert_eq!(payload.len(), 4, "SDK version is u32");
            u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn default_is_psl1ght_sentinel() {
    let host = Lv2Host::new();
    assert_eq!(
        captured_version(&host),
        SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN
    );
}

#[test]
fn set_sdk_version_propagates_to_dispatch() {
    let mut host = Lv2Host::new();
    host.set_sdk_version(0x0019_0004);
    assert_eq!(captured_version(&host), 0x0019_0004);
}

/// Non-vacuous proof: this test FAILS if the dispatch arm
/// regresses to the unconditional sentinel. It is the
/// adversarial-revert tripwire for the sc-25 fix.
#[test]
fn dispatch_does_not_hardcode_the_sentinel() {
    let mut host = Lv2Host::new();
    host.set_sdk_version(0x0016_0008);
    let got = captured_version(&host);
    assert_ne!(
        got, SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN,
        "dispatch_process_get_sdk_version regressed to hardcoded \
         0xFFFFFFFF instead of plumbing through the field set by \
         Lv2Host::set_sdk_version. See \
         docs/dev/bug_investigations/cellsysutil_allblocked_43.md"
    );
    assert_eq!(got, 0x0016_0008);
}
