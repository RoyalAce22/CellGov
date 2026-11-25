//! lv1's hypercall table:
//! - its entry count;
//! - the function that lv1 puts in an entry with no handler;
//! - the live 4.93 table, read from the reference console.
//!
//! lv1 builds the table at run time, so the stock image holds zeros
//! there. The count and the function come from each firmware's own
//! `lv1.self`, decrypted offline and read with `cellgov dev disasm`. The
//! live table comes from the console capture [`LIVE_TABLE_CAPTURE`]. No
//! byte of lv1 is committed; these facts are.

/// The entry count of lv1's hypercall table. A number at or past it gets
/// [`super::dispatch::UNKNOWN_HYPERCALL_R3`].
///
/// `set_lv1_hvcall_table` stores this count (`li r0, 256`) at HSPRG0 -
/// 0x6FD0, and the table base at HSPRG0 - 0x6FC8. The `sc` vector reads
/// both, at [OFW-4.93 lv1 a:0x0c24] and [OFW-4.93 lv1 a:0x0c30]. The count
/// is a constant in each image of
/// [`HYPERCALL_TABLE_SITES`].
///
/// [PS3DEVWIKI-HV-RE s:HV_call] The size of the HV call table is stored at -0x6FD0(HSPRG0).
pub const HYPERCALL_TABLE_ENTRIES: u64 = 256;

/// The r3 that the Invalid HV Call function returns:
/// `LV1_NOT_IMPLEMENTED` (-20), zero-extended.
///
/// The function is four instructions: `li r3, 0`, `oris r3, r3, 0xffff`,
/// `ori r3, r3, 0xffec` and `blr`. It writes no other register. Its
/// entry's low bit is clear, so a problem-state call to an invalid
/// number gets [`super::dispatch::PROBLEM_STATE_DENIED_R3`] before the
/// function runs.
///
/// [PS3DEVWIKI-HV-RE s:set_lv1_hvcall_table_entry_invalid] This function puts the Invalid HV Call function in an entry of the HV call table.
pub const INVALID_HYPERCALL_R3: u64 = 0x0000_0000_FFFF_FFEC;

/// Where one firmware's lv1 sets up its hypercall table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HypercallTableSite {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of `set_lv1_hvcall_table`, which stores
    /// [`HYPERCALL_TABLE_ENTRIES`]. `set_lv1_hvcall_table_entry_invalid`
    /// follows it at +0x24.
    pub set_table: u32,
    /// The address of the Invalid HV Call function, at `set_table` +
    /// 0x1F0.
    pub invalid_function: u32,
    /// The address of the table, or `None` where it was not read. The
    /// memory call table follows the table at +0x800.
    pub table_base: Option<u32>,
}

/// Each firmware whose lv1 image shows the entry count and the Invalid
/// HV Call function, ascending.
///
/// The 3.15 and 3.41 rows match the addresses that psdevwiki gives for
/// both functions. The 1.50 image does not hold the code these images
/// share, and no further reading of it was made.
///
/// [PS3DEVWIKI-HV-RE s:set_lv1_hvcall_table] set_lv1_hvcall_table is at 0x2C1F04 at 3.15 and at 0x2C02B4 at 3.41.
pub const HYPERCALL_TABLE_SITES: &[HypercallTableSite] = &[
    HypercallTableSite {
        firmware: "1.94",
        set_table: 0x2D8DE8,
        invalid_function: 0x2D8FD8,
        table_base: None,
    },
    HypercallTableSite {
        firmware: "3.15",
        set_table: 0x2C1F04,
        invalid_function: 0x2C20F4,
        table_base: Some(0x363A08),
    },
    HypercallTableSite {
        firmware: "3.41",
        set_table: 0x2C02B4,
        invalid_function: 0x2C04A4,
        table_base: Some(0x361B08),
    },
    HypercallTableSite {
        firmware: "3.70",
        set_table: 0x2C75C8,
        invalid_function: 0x2C77B8,
        table_base: Some(0x371708),
    },
    HypercallTableSite {
        firmware: "4.93",
        set_table: 0x2C8990,
        invalid_function: 0x2C8B80,
        table_base: Some(0x372D08),
    },
];

/// The site row of the firmware version `firmware`, or `None` when no
/// image read gives it.
pub fn table_site(firmware: &str) -> Option<&'static HypercallTableSite> {
    HYPERCALL_TABLE_SITES
        .iter()
        .find(|site| site.firmware == firmware)
}

/// The firmware version of [`LIVE_TABLE_HANDLERS`].
pub const LIVE_TABLE_FIRMWARE: &str = "4.93";

/// The console capture of the live 4.93 table, relative to the
/// workspace root. Its README gives the method.
pub const LIVE_TABLE_CAPTURE: &str = "tests/micro/lv1_hypercall_table";

/// Each number that has a handler in lv1's live 4.93 table, ascending,
/// with the table's doubleword for it.
///
/// The doubleword is the handler's code address, and its low bit is the
/// problem-state-callable flag. No entry sets the flag. Each number
/// below [`HYPERCALL_TABLE_ENTRIES`] that is absent here holds the
/// Invalid HV Call function. The memory calls share the thunk at
/// [OFW-4.93 lv1 a:0x2C89CC].
pub const LIVE_TABLE_HANDLERS: &[(u16, u32)] = &[
    (0, 0x2C89CC),
    (1, 0x2ED1C0),
    (2, 0x2EC554),
    (3, 0x2ECDE8),
    (4, 0x2EB2A4),
    (5, 0x2EB3E0),
    (6, 0x2C89CC),
    (7, 0x2EAFE4),
    (8, 0x2EB798),
    (9, 0x2EB618),
    (10, 0x2EC7CC),
    (11, 0x2EA35C),
    (12, 0x2E9DA8),
    (13, 0x2C89CC),
    (15, 0x2C89CC),
    (17, 0x2E9A74),
    (18, 0x2EF424),
    (19, 0x2EF31C),
    (24, 0x2EF530),
    (26, 0x2EAC98),
    (27, 0x2EAB90),
    (28, 0x2EAECC),
    (29, 0x2EADC4),
    (30, 0x2E9710),
    (31, 0x2E9354),
    (44, 0x2EBF5C),
    (54, 0x2FD878),
    (57, 0x31AB6C),
    (61, 0x2FE570),
    (62, 0x31AFC0),
    (64, 0x2FCF68),
    (65, 0x2FD090),
    (66, 0x2FE208),
    (67, 0x2FEBA4),
    (69, 0x2E8F38),
    (73, 0x2EAA28),
    (74, 0x2EBD38),
    (75, 0x2E5F20),
    (77, 0x2EB8A4),
    (78, 0x2FC968),
    (79, 0x2FDEA0),
    (89, 0x2FD980),
    (90, 0x2C89CC),
    (91, 0x2C89CC),
    (92, 0x2C89CC),
    (93, 0x2C89CC),
    (95, 0x2ECAF8),
    (96, 0x2EB4C8),
    (97, 0x2E912C),
    (98, 0x2E8DF0),
    (99, 0x31C468),
    (102, 0x2E9034),
    (105, 0x2C89CC),
    (106, 0x2C89CC),
    (107, 0x2C89CC),
    (108, 0x2C89CC),
    (109, 0x2C89CC),
    (110, 0x2C89CC),
    (111, 0x2C89CC),
    (112, 0x2C89CC),
    (114, 0x2DD21C),
    (115, 0x2DCCA8),
    (116, 0x2C89CC),
    (117, 0x2C89CC),
    (118, 0x2C89CC),
    (119, 0x2C89CC),
    (120, 0x2C0604),
    (121, 0x2BFC70),
    (122, 0x2C89CC),
    (123, 0x2C89CC),
    (124, 0x2C89CC),
    (125, 0x2C89CC),
    (126, 0x2C89CC),
    (127, 0x2F7988),
    (134, 0x2E9260),
    (135, 0x2C89CC),
    (136, 0x2C89CC),
    (137, 0x31C0C0),
    (138, 0x31BD54),
    (158, 0x2EC084),
    (162, 0x2F429C),
    (163, 0x2F2AF4),
    (164, 0x2F4100),
    (165, 0x2F3A68),
    (166, 0x2F2810),
    (167, 0x31C80C),
    (168, 0x31AA44),
    (170, 0x305A38),
    (171, 0x306EEC),
    (172, 0x306D64),
    (173, 0x306C10),
    (174, 0x306A6C),
    (175, 0x306918),
    (176, 0x306770),
    (177, 0x30660C),
    (178, 0x3063CC),
    (179, 0x306190),
    (180, 0x305FF0),
    (181, 0x305E54),
    (182, 0x2C89CC),
    (183, 0x2C89CC),
    (185, 0x307238),
    (186, 0x3076DC),
    (187, 0x307498),
    (188, 0x307030),
    (189, 0x3087CC),
    (190, 0x3085C8),
    (191, 0x308204),
    (193, 0x307FE0),
    (194, 0x307D2C),
    (195, 0x307B48),
    (196, 0x30793C),
    (197, 0x305CF0),
    (198, 0x305B8C),
    (199, 0x2FE698),
    (200, 0x31B40C),
    (201, 0x31B108),
    (202, 0x2F2F64),
    (207, 0x2FD394),
    (209, 0x31B710),
    (210, 0x20E908),
    (211, 0x20D4B4),
    (212, 0x20DB58),
    (213, 0x20D774),
    (214, 0x20EC9C),
    (216, 0x20F174),
    (217, 0x20F5BC),
    (218, 0x20FD78),
    (221, 0x20D060),
    (222, 0x20CED4),
    (225, 0x210ACC),
    (227, 0x20CB08),
    (228, 0x21027C),
    (230, 0x20E394),
    (231, 0x2C89CC),
    (232, 0x2F75B8),
    (233, 0x2F7E08),
    (240, 0x314E70),
    (241, 0x314B04),
    (242, 0x315050),
    (244, 0x2F7E4C),
    (245, 0x308B88),
    (246, 0x308A08),
    (248, 0x3094E4),
    (249, 0x3093AC),
    (250, 0x3090F4),
    (251, 0x308FB4),
    (252, 0x308D08),
    (253, 0x308E5C),
    (254, 0x30926C),
    (255, 0x2EBE34),
];

/// What lv1's hypercall table holds for one number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableEntry {
    /// A handler. The doubleword is its code address, and its low bit
    /// is the problem-state-callable flag.
    Handler(u32),
    /// The Invalid HV Call function, which returns
    /// [`INVALID_HYPERCALL_R3`].
    Invalid,
}

impl TableEntry {
    /// Whether a problem-state `sc 1` reaches this entry. The vector
    /// answers [`super::dispatch::PROBLEM_STATE_DENIED_R3`] to every
    /// other problem-state call.
    pub const fn problem_state_callable(self) -> bool {
        matches!(self, Self::Handler(word) if word & 1 != 0)
    }
}

/// lv1's table entry for hypercall `number` at the firmware version
/// `firmware`.
///
/// It is `None` in these cases:
/// - `number` is not below [`HYPERCALL_TABLE_ENTRIES`];
/// - no console capture gives the table of `firmware`.
pub fn table_entry(firmware: &str, number: u64) -> Option<TableEntry> {
    if firmware != LIVE_TABLE_FIRMWARE || number >= HYPERCALL_TABLE_ENTRIES {
        return None;
    }
    let entry = match LIVE_TABLE_HANDLERS.binary_search_by_key(&number, |(n, _)| u64::from(*n)) {
        Ok(at) => TableEntry::Handler(LIVE_TABLE_HANDLERS[at].1),
        Err(_) => TableEntry::Invalid,
    };
    Some(entry)
}

#[cfg(test)]
#[path = "tests/table_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/table_capture_tests.rs"]
mod capture_tests;
