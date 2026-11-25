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
/// Source: psdevwiki "Hypervisor Reverse Engineering" (Wayback snapshot
/// 2026-05-12). The page says that the system manager enables the
/// network control interface for LPAR 2 while GameOS boots. It sets the
/// bit whose position is the LPAR id. psdevwiki's "Repository Nodes" page
/// (Wayback snapshot 2025-12-20) agrees: its 3.41 GameOS dump holds the
/// `ss.laid` node with LPAR 2 as its third key word.
pub const LPAR_ID_GAME_OS: u64 = 2;
