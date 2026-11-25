//! Logical partition ids.

/// The id a repository read passes to name the caller's own partition.
///
/// Source: Linux `arch/powerpc/platforms/ps3/repository.c`,
/// `enum ps3_lpar_id`, `PS3_LPAR_ID_CURRENT`. Linux's `read_node`
/// replaces it with the result of `get_logical_partition_id` before the
/// hypercall.
pub const LPAR_ID_CURRENT: u64 = 0;

/// The partition of the policy module, which holds the system nodes.
///
/// Source: Linux `arch/powerpc/platforms/ps3/repository.c`,
/// `enum ps3_lpar_id`, `PS3_LPAR_ID_PME`.
pub const LPAR_ID_PME: u64 = 1;

/// The partition the game OS, and so the LV2 kernel, runs in.
///
/// Source: [PS3DEVWIKI-HV-RE s:Enabling_WLAN_Gelic_On_FAT]. The page
/// says that the system manager enables the network control interface
/// for LPAR 2 while GameOS boots. It sets the bit whose position is the
/// LPAR id. The repository dump [PS3DEVWIKI-REPO s:3.41_GameOS] agrees:
/// it holds the `ss.laid` node with LPAR 2 as its third key word.
pub const LPAR_ID_GAME_OS: u64 = 2;
