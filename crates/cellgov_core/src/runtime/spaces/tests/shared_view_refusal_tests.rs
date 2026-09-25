//! The runtime refuses a keyed attach to a live mapping with no views and names the invariant break.

use cellgov_mem::GuestMemory;

use super::*;
use crate::runtime::state::Runtime;

#[test]
fn a_keyed_attach_to_a_viewless_mapping_is_refused_and_named() {
    const KEY: u64 = 0x50;
    let mut rt = Runtime::new(GuestMemory::new(0x1000), cellgov_time::Budget::new(4), 100);
    rt.register_shared_mapping(KEY, 0x40, &[]).unwrap();
    let before = rt.sync_state_hash();

    rt.attach_keyed_shm_view(KEY, 0x40, AddressSpaceId::BOOT, 0x100);

    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("spaces.keyed_shm_mapping_viewless"),
        1
    );
    assert_eq!(rt.sync_state_hash(), before, "the mapping gained no view");
    assert_eq!(rt.sync_state_hash(), rt.sync_state_hash_from_scratch());
}
