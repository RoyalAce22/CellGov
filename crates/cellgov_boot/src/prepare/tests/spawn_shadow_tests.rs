//! The shadow withdrawal of the spawn loader.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_core::{AddressSpaceId, ProcessSpawnLoadError};
use cellgov_install::keys::{KeyVault, KeyVaultError};
use cellgov_mem::GuestMemory;
use cellgov_ppu::shadow::PredecodedShadow;

use super::thread_shadow_tests::created_thread;
use super::{SpawnFirmware, SpawnLoader, ThreadShadow};
use crate::child_init::ChildInitPlans;
use crate::{KeyVaultSource, NoTaps, NullSink};

/// `li r3,1 ; blr`.
const TEXT: [u8; 8] = [0x38, 0x60, 0x00, 0x01, 0x4E, 0x80, 0x00, 0x20];

/// A key source that answers every image with an empty vault.
struct EmptyVault(KeyVault);

impl KeyVaultSource for EmptyVault {
    fn vault_for(&self, _bytes: &[u8]) -> Result<&KeyVault, &KeyVaultError> {
        Ok(&self.0)
    }
}

fn spawn_loader(thread_shadow: ThreadShadow) -> SpawnLoader {
    SpawnLoader {
        thread_shadow,
        firmware: SpawnFirmware {
            dir: None,
            candidates: RefCell::new(None),
            prx_base: None,
        },
        plans: ChildInitPlans::default(),
        run_hle_stubbed: false,
        profile_mode: false,
        sink: Rc::new(NullSink),
        keys: Rc::new(EmptyVault(KeyVault::empty())),
        taps: Rc::new(NoTaps),
    }
}

#[test]
fn a_refused_spawn_leaves_a_later_thread_without_the_shadow() {
    let shadow = PredecodedShadow::build(0, &TEXT);
    let thread_shadow = ThreadShadow::default();
    thread_shadow.offer(Some(&shadow));
    let loader = spawn_loader(thread_shadow.clone());
    let mut child_mem = GuestMemory::from_regions(Vec::new()).expect("no regions to overlap");

    let refused = loader.load(b"not an ELF image", &mut child_mem, AddressSpaceId::new(1));

    assert!(
        matches!(refused, Err(ProcessSpawnLoadError::ImageParse { .. })),
        "bytes that are no ELF image refuse as a parse failure, got {refused:?}",
    );
    assert!(created_thread(&thread_shadow)
        .instruction_shadow()
        .is_none());
}
