//! The retain knob, in its own binary.
//!
//! `CELLGOV_RETAIN_SCRATCH` is process-wide, so a test that sets it
//! races every other test in the same binary.

#![cfg(feature = "scratch")]

use cellgov_testkit::scratch::scratch_labeled;

#[test]
fn the_retain_knob_keeps_the_tree_and_names_where_it_went() {
    let kept = {
        // `Drop` reads the variable, so set it before the guard exists.
        std::env::set_var("CELLGOV_RETAIN_SCRATCH", "1");
        let s = scratch_labeled("retain_probe");
        std::fs::write(s.join("evidence"), b"x").expect("write into the scratch tree");
        s.to_path_buf()
    };
    std::env::remove_var("CELLGOV_RETAIN_SCRATCH");

    assert!(
        kept.join("evidence").is_file(),
        "{} was removed while the retain knob was set",
        kept.display()
    );
    // The knob is for reading a failed run's tree by hand, so nothing
    // else reclaims it.
    std::fs::remove_dir_all(&kept).expect("remove the retained tree");

    let swept = {
        let s = scratch_labeled("retain_probe");
        s.to_path_buf()
    };
    assert!(
        !swept.exists(),
        "{} survived with the knob unset",
        swept.display()
    );
}
