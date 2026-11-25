//! The numbers of the partition, logical PPE, repository, logical
//! console and time hypercalls.

/// `get_logical_ppe_id`: no input, and the id of the caller's logical
/// PPE in r4.
///
/// Linux `arch/powerpc/platforms/ps3/repository.c`,
/// `ps3_repository_read_mm_info`, makes the call in this form. The lv1
/// handler [OFW-4.93 lv1 a:0x2E8F38] writes the outputs in this form; see
/// [`crate::lv1::ppe`].
pub const GET_LOGICAL_PPE_ID: u64 = 69;

/// `get_logical_partition_id`: no input, and the caller's partition id
/// in r4.
pub const GET_LOGICAL_PARTITION_ID: u64 = 74;

/// `read_repository_node`: the partition id in r3 and the key in r4 to
/// r7. On success, r4 and r5 hold the node's two values.
pub const READ_REPOSITORY_NODE: u64 = 91;

/// `lv1_comm_dev_logical_console_construct`, by the name psdevwiki
/// gives 105 [PS3DEVWIKI-HV-SYSCALLS s:HV_Syscalls]. The headers call it
/// `undocumented_function_105`.
///
/// The kernel's wrapper [OFW-4.93 lv2_kernel a:0x800000000000FC54] passes
/// the console id in r3, and reads r3 alone after the call.
pub const LOGICAL_CONSOLE_CONSTRUCT: u64 = 105;

/// `lv1_comm_dev_logical_console_destruct`, by the name psdevwiki gives
/// 106 [PS3DEVWIKI-HV-SYSCALLS s:HV_Syscalls]. The headers call it
/// `undocumented_function_106`.
///
/// The kernel's wrapper [OFW-4.93 lv2_kernel a:0x80000000000104a8]
/// passes the console id in r3, and reads r3 alone after the call; see
/// [`crate::lv1::console`].
pub const LOGICAL_CONSOLE_DESTRUCT: u64 = 106;

/// `lv1_comm_dev_logical_console_send`, by the name psdevwiki gives 107
/// [PS3DEVWIKI-HV-SYSCALLS s:HV_Syscalls]. The headers call it
/// `undocumented_function_107`.
///
/// The kernel's send wrappers [OFW-4.93 lv2_kernel a:0x800000000000FCD8]
/// [OFW-4.93 lv2_kernel a:0x800000000000FDC0-0x800000000000FEB0] put the
/// console id in r3, the byte count in r4 and up to
/// [`LOGICAL_CONSOLE_SEND_MAX`] bytes in r5 to r8. After a call with
/// r3 = 0, the kernel subtracts r4 from its count of unsent bytes, so r4
/// is the count the console took.
pub const LOGICAL_CONSOLE_SEND: u64 = 107;

/// `lv1_comm_dev_logical_console_flush`, by the name psdevwiki gives 109
/// [PS3DEVWIKI-HV-SYSCALLS s:HV_Syscalls]. The headers call it
/// `undocumented_function_109`.
///
/// The kernel's wrapper [OFW-4.93 lv2_kernel a:0x800000000000FD30] passes
/// the console id in r3, and reads r3 alone after the call.
pub const LOGICAL_CONSOLE_FLUSH: u64 = 109;

/// The most bytes one [`LOGICAL_CONSOLE_SEND`] carries: the four
/// doublewords r5 to r8.
pub const LOGICAL_CONSOLE_SEND_MAX: usize = 32;
