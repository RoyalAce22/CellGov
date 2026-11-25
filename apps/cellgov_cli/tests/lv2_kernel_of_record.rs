//! The installed 4.93 firmware's LV2 kernel, decrypted from the store
//! and mapped at its link address.
//!
//! Requires:
//!
//! - the `installed-firmware-tests` feature
//! - a 4.93 entry installed by `cellgov firmware install`
//! - the operator's LV2 keys

use cellgov_boot::compose::FirmwareChoice;
use cellgov_boot::kernel::kernel_of_record;
use cellgov_boot::KeyVaultSource;
use cellgov_install::keys::{KeyVault, KeyVaultError};
use cellgov_install::store::inventory::StoreInventory;
use cellgov_install::store::select::select_firmware;
use cellgov_install::store::DEFAULT_VFS_ROOT;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::format::core_os::LV2_KERNEL_LINK_BASE;

#[path = "common/installed_firmware.rs"]
mod installed_firmware;

struct OperatorVault(KeyVault);

impl KeyVaultSource for OperatorVault {
    fn vault_for(&self, _bytes: &[u8]) -> Result<&KeyVault, &KeyVaultError> {
        Ok(&self.0)
    }
}

#[test]
fn the_4_93_kernel_maps_at_its_link_base_and_enters_by_saving_r3_to_r5() {
    let store = installed_firmware::workspace_root().join(DEFAULT_VFS_ROOT);
    let inventory =
        StoreInventory::read(&store).unwrap_or_else(|e| panic!("installed-firmware-tests: {e}"));
    let managed = select_firmware(
        &inventory,
        Some(installed_firmware::TEST_FIRMWARE_VERSION),
        None,
    )
    .unwrap_or_else(|e| panic!("installed-firmware-tests: {e}"));
    let vault =
        KeyVault::load_for_vfs(&store).unwrap_or_else(|e| panic!("installed-firmware-tests: {e}"));

    let kernel = kernel_of_record(
        &FirmwareChoice::Managed(Box::new(managed)),
        &OperatorVault(vault),
    )
    .unwrap_or_else(|e| panic!("installed-firmware-tests: {e}"));

    assert_eq!(kernel.firmware, installed_firmware::TEST_FIRMWARE_VERSION);
    assert_eq!(kernel.image.entry, LV2_KERNEL_LINK_BASE + 0x100);
    let words = kernel
        .image
        .mem
        .read(ByteRange::new(GuestAddr::new(kernel.image.entry), 12).expect("entry range"))
        .expect("the entry lies in the kernel region");
    // mtspr SPRG1,r3; mtspr SPRG2,r4; mtspr SPRG3,r5.
    assert_eq!(
        &*words,
        &[0x7c, 0x71, 0x43, 0xa6, 0x7c, 0x92, 0x43, 0xa6, 0x7c, 0xb3, 0x43, 0xa6]
    );
    let code = kernel.image.mem.code_spans();
    assert_eq!(code.first().map(|span| span.0), Some(LV2_KERNEL_LINK_BASE));
}
