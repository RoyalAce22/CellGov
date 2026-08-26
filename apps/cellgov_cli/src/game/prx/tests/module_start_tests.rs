use super::*;

const WAIT_SITE: &str =
    "\n  last 2 syscalls:\n    LV2 #102 at 0x01968a50\n    LV2 #107 at 0x01968a90";

#[test]
fn a_stalled_module_start_renders_its_rings_after_the_reason() {
    let e = ModuleStartError::Stalled {
        module: "cellSysutil_Library".into(),
        steps: 65,
        reason: "NoRunnableUnit/AllBlocked".into(),
        detail: WAIT_SITE.into(),
    };
    let text = e.to_string();
    let head = text
        .find("(NoRunnableUnit/AllBlocked)")
        .expect("reason present");
    let site = text
        .find("LV2 #107 at 0x01968a90")
        .expect("wait site present");
    assert!(head < site, "rings must follow the reason: {text}");
}

#[test]
fn an_incomplete_module_start_renders_its_rings_after_the_last_pc() {
    let e = ModuleStartError::Incomplete {
        module: "cellSysutil_Library".into(),
        budget: PER_MODULE_STEP_BUDGET,
        last_pc: 0x0196_8a90,
        detail: WAIT_SITE.into(),
    };
    let text = e.to_string();
    let head = text
        .find("last pc=0x0000000001968a90")
        .expect("last pc present");
    let site = text
        .find("LV2 #107 at 0x01968a90")
        .expect("wait site present");
    assert!(head < site, "rings must follow the last pc: {text}");
}

#[test]
fn an_empty_detail_adds_nothing_to_the_message() {
    let e = ModuleStartError::Stalled {
        module: "m".into(),
        steps: 0,
        reason: "r".into(),
        detail: String::new(),
    };
    assert!(e.to_string().ends_with("fail-fast"), "got {e}");
}
