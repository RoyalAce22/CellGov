//! A cross-runner fixture never files a capture taken under a boot
//! override.

use super::overridden_capture_refusal;
use cellgov_compare::BootOverrides;

#[test]
fn a_capture_under_no_override_is_accepted() {
    assert_eq!(
        overridden_capture_refusal("cg.json", &BootOverrides::default()),
        None
    );
}

#[test]
fn a_capture_under_an_override_is_refused_by_name() {
    let refusal = overridden_capture_refusal(
        "cg.json",
        &BootOverrides {
            prx_base: Some(0x3000_0000),
            ..BootOverrides::default()
        },
    )
    .expect("an overridden capture is refused");
    assert!(refusal.contains("cg.json"), "{refusal}");
    assert!(refusal.contains("prx_base=0x30000000"), "{refusal}");
}
