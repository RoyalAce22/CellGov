//! BP_VR held to the console capture of the 4.93 boot repository record.

use super::CELL_BP_VR;
use crate::lv1::boot_record_capture::captured_doubleword;
use crate::lv1::repository::BOOT_RECORD_4_93;

/// The offset in the 4.93 boot record of the word that the kernel's
/// `stw` of its `mfspr` of BP_VR writes.
const BP_VR_OFFSET: u64 = 0xb8;

#[test]
fn the_bp_vr_value_is_the_word_the_4_93_kernel_stored_in_its_boot_record() {
    let doubleword = captured_doubleword(BOOT_RECORD_4_93 + BP_VR_OFFSET);
    // The kernel's stw writes the high word. The low word is zero fill.
    assert_eq!(doubleword, u64::from(CELL_BP_VR) << 32);
}
