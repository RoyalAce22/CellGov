//! The hypercall table: each number's name and its input and output
//! register counts.
//!
//! Source: Linux `arch/powerpc/include/asm/lv1call.h`, the `LV1_CALL`
//! table, for each number that table lists. The OtherOS++ copy of that
//! header gives the numbers the Linux table omits: the
//! `undocumented_function_N` calls and the storage-region calls. The two
//! copies disagree only on the input counts of 90, 92 and 93, and this
//! table takes the Linux counts there.

/// The copy of `lv1call.h` that a row comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallSource {
    /// Linux `arch/powerpc/include/asm/lv1call.h`, the `LV1_CALL` table.
    Linux,
    /// The OtherOS++ copy of that header, for a number the Linux table
    /// omits: github.com/damiandudycz/ps3,
    /// `dev-tools/kernel-patches/linux-files/arch/powerpc/include/asm/lv1call.h`.
    OtherOsPlusPlus,
}

/// One row of the hypercall table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv1Call {
    /// The number the caller puts in r11.
    pub number: u16,
    /// The name the header gives the call.
    pub name: &'static str,
    /// How many input registers the call reads, from r3.
    pub inputs: u8,
    /// How many output registers the call writes, from r4.
    pub outputs: u8,
    /// The header the row comes from.
    pub source: CallSource,
}

impl Lv1Call {
    const fn new(
        number: u16,
        name: &'static str,
        inputs: u8,
        outputs: u8,
        source: CallSource,
    ) -> Self {
        Self {
            number,
            name,
            inputs,
            outputs,
            source,
        }
    }
}

/// The most input registers a call reads: r3 to r10.
pub const MAX_INPUTS: u8 = 8;
/// The most output registers a call writes: r4 to r10.
pub const MAX_OUTPUTS: u8 = 7;

/// The row of hypercall `number`, or `None` when neither header lists it.
pub fn lookup(number: u64) -> Option<&'static Lv1Call> {
    LV1_CALLS
        .binary_search_by_key(&number, |call| u64::from(call.number))
        .ok()
        .map(|at| &LV1_CALLS[at])
}

/// `allocate_memory`: create a memory region in the caller's partition.
///
/// The inputs:
/// - r3 is the region size, a multiple of the page size.
/// - r4 is the page size exponent.
/// - r5 is 0.
/// - r6 holds the flags.
///
/// On success, r4 holds the region's logical partition address and r5
/// its memory unit id. Linux `arch/powerpc/platforms/ps3/mm.c`,
/// `ps3_mm_region_create`, makes the call in this form and keeps r4 as
/// the region base. [PS3DEVWIKI-HV-SYSCALL-REF s:lv1_allocate_memory]
/// gives the same registers.
pub const ALLOCATE_MEMORY: u64 = 0;

/// `write_htab_entry`: write one entry of an address space's page table.
///
/// The inputs:
/// - r3 is the vas id, or 0 for the caller's address space.
/// - r4 is the entry index.
/// - r5 is doubleword 0 of the entry.
/// - r6 is doubleword 1, with the page's logical partition address.
///
/// There is no output. The lv1 handler [OFW-4.93 lv1 a:0x2ED1C0] reads the
/// inputs in this form; see [`super::page_table`].
pub const WRITE_HTAB_ENTRY: u64 = 1;

/// `construct_virtual_address_space`: create a virtual address space
/// with its own hashed page table.
///
/// The inputs:
/// - r3 is the page table size exponent.
/// - r4 is the number of page sizes.
/// - r5 holds the page shifts, 8 bits each from the most significant
///   byte.
///
/// On success, r4 holds the vas id and r5 the page table size in bytes.
/// Linux `arch/powerpc/platforms/ps3/mm.c`, `ps3_mm_vas_create`, makes
/// the call in this form.
/// [PS3DEVWIKI-HV-SYSCALL-REF s:lv1_construct_virtual_address_space]
/// gives the same registers.
pub const CONSTRUCT_VIRTUAL_ADDRESS_SPACE: u64 = 2;

/// `select_virtual_address_space`: the vas id in r3, and no output.
/// Linux `ps3_mm_vas_create` makes the call with the id that
/// [`CONSTRUCT_VIRTUAL_ADDRESS_SPACE`] returned.
pub const SELECT_VIRTUAL_ADDRESS_SPACE: u64 = 7;

/// `configure_irq_state_bitmap`: register the interrupt state bitmap of
/// one hardware thread of a logical PPE.
///
/// The inputs:
/// - r3 is the logical PPE id.
/// - r4 is the hardware thread.
/// - r5 is the bitmap's logical partition address.
///
/// There is no output. Linux `arch/powerpc/platforms/ps3/interrupt.c`,
/// `ps3_init_IRQ`, makes the call in this form. The lv1 handler
/// [OFW-4.93 lv1 a:0x2EA35C] reads the inputs in this form; see
/// [`super::interrupt`].
pub const CONFIGURE_IRQ_STATE_BITMAP: u64 = 11;

/// `connect_irq_plug_ext`: five inputs and no output.
/// - r3 is the logical PPE id.
/// - r4 is the hardware thread.
/// - r5 is the plug.
/// - r6 is the event receive port id.
/// - r7 is 0.
///
/// Linux `arch/powerpc/platforms/ps3/interrupt.c`, `ps3_irq_plug_setup`,
/// makes the call in this form. The lv1 handler [OFW-4.93 lv1 a:0x2E9DA8]
/// reads the inputs in this form; see [`super::irq_plug`].
pub const CONNECT_IRQ_PLUG_EXT: u64 = 12;

/// `construct_event_receive_port`: no input, and the new port's id in
/// r4. Linux `arch/powerpc/platforms/ps3/interrupt.c`,
/// `ps3_event_receive_port_setup`, makes the call in this form and names
/// the output the outlet. The lv1 handler [OFW-4.93 lv1 a:0x2EF424]
/// writes the output in this form; see [`super::event_port`].
pub const CONSTRUCT_EVENT_RECEIVE_PORT: u64 = 18;

/// `get_logical_ppe_id`: no input, and the id of the caller's logical
/// PPE in r4. Linux `arch/powerpc/platforms/ps3/repository.c`,
/// `ps3_repository_read_mm_info`, makes the call in this form. The lv1
/// handler [OFW-4.93 lv1 a:0x2E8F38] writes the outputs in this form; see
/// [`super::ppe`].
pub const GET_LOGICAL_PPE_ID: u64 = 69;

/// `get_logical_partition_id`: no input, and the caller's partition id
/// in r4.
pub const GET_LOGICAL_PARTITION_ID: u64 = 74;

/// `read_repository_node`: the partition id in r3 and the key in r4 to
/// r7. On success, r4 and r5 hold the node's two values.
pub const READ_REPOSITORY_NODE: u64 = 91;

/// `read_htab_entries`: read four entries of an address space's page
/// table.
///
/// The inputs are the vas id in r3, or 0 for the caller's address space,
/// and the index of the first entry in r4. On success, r4 to r7 hold
/// doubleword 0 of each entry, and r8 holds the low halfword of
/// doubleword 1 of each entry. The lv1 handler [OFW-4.93 lv1 a:0x2ECAF8]
/// reads the inputs and writes the outputs in this form; see
/// [`super::page_table`].
pub const READ_HTAB_ENTRIES: u64 = 95;

/// `map_htab`: map an address space's page table into the partition's
/// logical address space.
///
/// The input is the vas id in r3, or 0 for the caller's address space.
/// On success, r4 holds the mapping's logical partition address. The lv1
/// worker [OFW-4.93 lv1 a:0x2DDB20] reads the input and writes the output
/// in this form; see [`super::page_table::MAP_HTAB_RESOURCE_SHORTAGE_R3`].
pub const MAP_HTAB: u64 = 122;

/// `lv1_comm_dev_logical_console_construct`, by the name psdevwiki
/// gives 105 [PS3DEVWIKI-HV-SYSCALLS s:HV_Syscalls]. The headers call it
/// `undocumented_function_105`.
///
/// The kernel's wrapper [OFW-4.93 lv2_kernel a:0x800000000000FC54] passes
/// the console id in r3, and reads r3 alone after the call.
pub const LOGICAL_CONSOLE_CONSTRUCT: u64 = 105;

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

/// Every row, ascending by number.
pub const LV1_CALLS: &[Lv1Call] = &[
    Lv1Call::new(0, "allocate_memory", 4, 2, CallSource::Linux),
    Lv1Call::new(1, "write_htab_entry", 4, 0, CallSource::Linux),
    Lv1Call::new(
        2,
        "construct_virtual_address_space",
        3,
        2,
        CallSource::Linux,
    ),
    Lv1Call::new(3, "invalidate_htab_entries", 5, 0, CallSource::Linux),
    Lv1Call::new(
        4,
        "get_virtual_address_space_id_of_ppe",
        0,
        1,
        CallSource::Linux,
    ),
    Lv1Call::new(
        6,
        "query_logical_partition_address_region_info",
        1,
        5,
        CallSource::Linux,
    ),
    Lv1Call::new(7, "select_virtual_address_space", 1, 0, CallSource::Linux),
    Lv1Call::new(9, "pause", 1, 0, CallSource::Linux),
    Lv1Call::new(
        10,
        "destruct_virtual_address_space",
        1,
        0,
        CallSource::Linux,
    ),
    Lv1Call::new(11, "configure_irq_state_bitmap", 3, 0, CallSource::Linux),
    Lv1Call::new(12, "connect_irq_plug_ext", 5, 0, CallSource::Linux),
    Lv1Call::new(13, "release_memory", 1, 0, CallSource::Linux),
    Lv1Call::new(15, "put_iopte", 5, 0, CallSource::Linux),
    Lv1Call::new(17, "disconnect_irq_plug_ext", 3, 0, CallSource::Linux),
    Lv1Call::new(18, "construct_event_receive_port", 0, 1, CallSource::Linux),
    Lv1Call::new(19, "destruct_event_receive_port", 1, 0, CallSource::Linux),
    Lv1Call::new(24, "send_event_locally", 1, 0, CallSource::Linux),
    Lv1Call::new(27, "end_of_interrupt", 1, 0, CallSource::Linux),
    Lv1Call::new(28, "connect_irq_plug", 2, 0, CallSource::Linux),
    Lv1Call::new(29, "disconnect_irq_plug", 1, 0, CallSource::Linux),
    Lv1Call::new(30, "end_of_interrupt_ext", 3, 0, CallSource::Linux),
    Lv1Call::new(31, "did_update_interrupt_mask", 2, 0, CallSource::Linux),
    Lv1Call::new(44, "shutdown_logical_partition", 1, 0, CallSource::Linux),
    Lv1Call::new(54, "destruct_logical_spe", 1, 0, CallSource::Linux),
    Lv1Call::new(57, "construct_logical_spe", 7, 6, CallSource::Linux),
    Lv1Call::new(61, "set_spe_interrupt_mask", 3, 0, CallSource::Linux),
    Lv1Call::new(
        62,
        "undocumented_function_62",
        5,
        0,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(64, "set_spe_transition_notifier", 3, 0, CallSource::Linux),
    Lv1Call::new(65, "disable_logical_spe", 2, 0, CallSource::Linux),
    Lv1Call::new(66, "clear_spe_interrupt_status", 4, 0, CallSource::Linux),
    Lv1Call::new(67, "get_spe_interrupt_status", 2, 1, CallSource::Linux),
    Lv1Call::new(69, "get_logical_ppe_id", 0, 1, CallSource::Linux),
    Lv1Call::new(73, "set_interrupt_mask", 5, 0, CallSource::Linux),
    Lv1Call::new(74, "get_logical_partition_id", 0, 1, CallSource::Linux),
    Lv1Call::new(
        77,
        "configure_execution_time_variable",
        1,
        0,
        CallSource::Linux,
    ),
    Lv1Call::new(78, "get_spe_irq_outlet", 2, 1, CallSource::Linux),
    Lv1Call::new(
        79,
        "set_spe_privilege_state_area_1_register",
        3,
        0,
        CallSource::Linux,
    ),
    Lv1Call::new(90, "create_repository_node", 6, 0, CallSource::Linux),
    Lv1Call::new(91, "read_repository_node", 5, 2, CallSource::Linux),
    Lv1Call::new(92, "write_repository_node", 6, 0, CallSource::Linux),
    Lv1Call::new(93, "delete_repository_node", 4, 0, CallSource::Linux),
    Lv1Call::new(95, "read_htab_entries", 2, 5, CallSource::Linux),
    Lv1Call::new(96, "set_dabr", 2, 0, CallSource::Linux),
    Lv1Call::new(
        99,
        "undocumented_function_99",
        2,
        0,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(103, "get_total_execution_time", 2, 1, CallSource::Linux),
    Lv1Call::new(
        105,
        "undocumented_function_105",
        7,
        0,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(
        106,
        "undocumented_function_106",
        1,
        0,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(
        107,
        "undocumented_function_107",
        6,
        1,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(
        108,
        "undocumented_function_108",
        1,
        5,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(
        109,
        "undocumented_function_109",
        1,
        0,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(
        114,
        "undocumented_function_114",
        3,
        1,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(
        115,
        "undocumented_function_115",
        1,
        0,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(116, "allocate_io_segment", 3, 1, CallSource::Linux),
    Lv1Call::new(117, "release_io_segment", 2, 0, CallSource::Linux),
    Lv1Call::new(120, "construct_io_irq_outlet", 1, 1, CallSource::Linux),
    Lv1Call::new(121, "destruct_io_irq_outlet", 1, 0, CallSource::Linux),
    Lv1Call::new(122, "map_htab", 1, 1, CallSource::Linux),
    Lv1Call::new(123, "unmap_htab", 1, 0, CallSource::Linux),
    Lv1Call::new(127, "get_version_info", 0, 2, CallSource::Linux),
    Lv1Call::new(
        138,
        "undocumented_function_138",
        2,
        0,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(140, "construct_lpm", 6, 3, CallSource::Linux),
    Lv1Call::new(141, "destruct_lpm", 1, 0, CallSource::Linux),
    Lv1Call::new(142, "start_lpm", 1, 0, CallSource::Linux),
    Lv1Call::new(143, "stop_lpm", 1, 1, CallSource::Linux),
    Lv1Call::new(144, "copy_lpm_trace_buffer", 3, 1, CallSource::Linux),
    Lv1Call::new(145, "add_lpm_event_bookmark", 5, 0, CallSource::Linux),
    Lv1Call::new(146, "delete_lpm_event_bookmark", 3, 0, CallSource::Linux),
    Lv1Call::new(147, "set_lpm_interrupt_mask", 3, 1, CallSource::Linux),
    Lv1Call::new(148, "get_lpm_interrupt_status", 1, 1, CallSource::Linux),
    Lv1Call::new(149, "set_lpm_general_control", 5, 2, CallSource::Linux),
    Lv1Call::new(150, "set_lpm_interval", 3, 1, CallSource::Linux),
    Lv1Call::new(151, "set_lpm_trigger_control", 3, 1, CallSource::Linux),
    Lv1Call::new(152, "set_lpm_counter_control", 4, 1, CallSource::Linux),
    Lv1Call::new(153, "set_lpm_group_control", 3, 1, CallSource::Linux),
    Lv1Call::new(154, "set_lpm_debug_bus_control", 3, 1, CallSource::Linux),
    Lv1Call::new(155, "set_lpm_counter", 5, 2, CallSource::Linux),
    Lv1Call::new(156, "set_lpm_signal", 7, 0, CallSource::Linux),
    Lv1Call::new(157, "set_lpm_spr_trigger", 2, 0, CallSource::Linux),
    Lv1Call::new(158, "insert_htab_entry", 6, 3, CallSource::Linux),
    Lv1Call::new(162, "read_virtual_uart", 3, 1, CallSource::Linux),
    Lv1Call::new(163, "write_virtual_uart", 3, 1, CallSource::Linux),
    Lv1Call::new(164, "set_virtual_uart_param", 3, 0, CallSource::Linux),
    Lv1Call::new(165, "get_virtual_uart_param", 2, 1, CallSource::Linux),
    Lv1Call::new(166, "configure_virtual_uart_irq", 1, 1, CallSource::Linux),
    Lv1Call::new(
        167,
        "undocumented_function_167",
        2,
        1,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(
        168,
        "undocumented_function_168",
        3,
        0,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(170, "open_device", 3, 0, CallSource::Linux),
    Lv1Call::new(171, "close_device", 2, 0, CallSource::Linux),
    Lv1Call::new(172, "map_device_mmio_region", 5, 1, CallSource::Linux),
    Lv1Call::new(173, "unmap_device_mmio_region", 3, 0, CallSource::Linux),
    Lv1Call::new(174, "allocate_device_dma_region", 5, 1, CallSource::Linux),
    Lv1Call::new(175, "free_device_dma_region", 3, 0, CallSource::Linux),
    Lv1Call::new(176, "map_device_dma_region", 6, 0, CallSource::Linux),
    Lv1Call::new(177, "unmap_device_dma_region", 4, 0, CallSource::Linux),
    Lv1Call::new(185, "net_add_multicast_address", 4, 0, CallSource::Linux),
    Lv1Call::new(186, "net_remove_multicast_address", 4, 0, CallSource::Linux),
    Lv1Call::new(187, "net_start_tx_dma", 4, 0, CallSource::Linux),
    Lv1Call::new(188, "net_stop_tx_dma", 2, 0, CallSource::Linux),
    Lv1Call::new(189, "net_start_rx_dma", 4, 0, CallSource::Linux),
    Lv1Call::new(190, "net_stop_rx_dma", 2, 0, CallSource::Linux),
    Lv1Call::new(
        191,
        "net_set_interrupt_status_indicator",
        4,
        0,
        CallSource::Linux,
    ),
    Lv1Call::new(193, "net_set_interrupt_mask", 4, 0, CallSource::Linux),
    Lv1Call::new(194, "net_control", 6, 2, CallSource::Linux),
    Lv1Call::new(
        197,
        "connect_interrupt_event_receive_port",
        4,
        0,
        CallSource::Linux,
    ),
    Lv1Call::new(
        198,
        "disconnect_interrupt_event_receive_port",
        4,
        0,
        CallSource::Linux,
    ),
    Lv1Call::new(
        199,
        "get_spe_all_interrupt_statuses",
        1,
        1,
        CallSource::Linux,
    ),
    Lv1Call::new(
        200,
        "undocumented_function_200",
        1,
        0,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(
        201,
        "undocumented_function_201",
        1,
        0,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(202, "deconfigure_virtual_uart_irq", 0, 0, CallSource::Linux),
    Lv1Call::new(207, "enable_logical_spe", 2, 0, CallSource::Linux),
    Lv1Call::new(
        209,
        "undocumented_function_209",
        8,
        0,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(210, "gpu_open", 1, 0, CallSource::Linux),
    Lv1Call::new(211, "gpu_close", 0, 0, CallSource::Linux),
    Lv1Call::new(212, "gpu_device_map", 1, 2, CallSource::Linux),
    Lv1Call::new(213, "gpu_device_unmap", 1, 0, CallSource::Linux),
    Lv1Call::new(214, "gpu_memory_allocate", 5, 2, CallSource::Linux),
    Lv1Call::new(216, "gpu_memory_free", 1, 0, CallSource::Linux),
    Lv1Call::new(217, "gpu_context_allocate", 2, 5, CallSource::Linux),
    Lv1Call::new(218, "gpu_context_free", 1, 0, CallSource::Linux),
    Lv1Call::new(221, "gpu_context_iomap", 5, 0, CallSource::Linux),
    Lv1Call::new(225, "gpu_context_attribute", 6, 0, CallSource::Linux),
    Lv1Call::new(227, "gpu_context_intr", 1, 1, CallSource::Linux),
    Lv1Call::new(228, "gpu_attribute", 3, 0, CallSource::Linux),
    Lv1Call::new(232, "get_rtc", 0, 2, CallSource::Linux),
    Lv1Call::new(
        240,
        "set_ppe_periodic_tracer_frequency",
        1,
        0,
        CallSource::Linux,
    ),
    Lv1Call::new(241, "start_ppe_periodic_tracer", 5, 0, CallSource::Linux),
    Lv1Call::new(242, "stop_ppe_periodic_tracer", 1, 1, CallSource::Linux),
    Lv1Call::new(245, "storage_read", 6, 1, CallSource::Linux),
    Lv1Call::new(246, "storage_write", 6, 1, CallSource::Linux),
    Lv1Call::new(248, "storage_send_device_command", 6, 1, CallSource::Linux),
    Lv1Call::new(249, "storage_get_async_status", 1, 2, CallSource::Linux),
    Lv1Call::new(
        250,
        "storage_create_region",
        5,
        2,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(
        251,
        "storage_delete_region",
        2,
        1,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(
        252,
        "storage_set_region_acl",
        4,
        1,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(
        253,
        "storage_get_region_acl",
        3,
        2,
        CallSource::OtherOsPlusPlus,
    ),
    Lv1Call::new(254, "storage_check_async_status", 2, 1, CallSource::Linux),
    Lv1Call::new(255, "panic", 1, 0, CallSource::Linux),
];

#[cfg(test)]
#[path = "tests/call_tests.rs"]
mod tests;
