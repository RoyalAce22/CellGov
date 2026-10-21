//! The bootable microtests' expectation table: one entry per microtest,
//! one expectation per `u32` of its CGOV payload, each derived from the
//! test's documented output layout. Shared by the boot suite and the
//! hardware-capture checks.

use cellgov_ps3_abi::hw::rsx::{
    CELL_GCM_DISPLAY_FLIP_STATUS_DONE, CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
};
use cellgov_ps3_abi::hw::spu::{
    event, MFC_ATOMIC_STAT_G, MFC_ATOMIC_STAT_S, MFC_ATOMIC_STAT_U, SPU_IN_MBOX_DEPTH,
    SPU_OUT_INTR_MBOX_DEPTH, SPU_STATUS_P, SPU_STATUS_R, SPU_STATUS_W,
};
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::spu::{
    event as group_event, group_join_cause, raw_spu, IMAGE_PATH_MAX, IMAGE_PATH_NAME_MAX,
};

/// What a payload word must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// Fixed by the test's design (a derived sum, a sentinel, a count).
    Exact(u32),
    /// Must have happened at least once, but the count is not fixed.
    NonZero,
    /// Varies with scheduler interleaving; asserting it would pin the
    /// scheduler rather than the behaviour under test.
    Any,
}

pub struct Case {
    pub name: &'static str,
    /// Scheduler-step cap. Generous: the outcome assertion catches a
    /// wedge, so a tight cap would only convert a real hang into a
    /// confusing `Timeout`.
    pub max_steps: usize,
    /// One entry per `u32` in the CGOV payload, in wire order. The
    /// length also pins the payload size, so a struct that grows or
    /// shrinks a field fails here rather than being silently ignored.
    pub fields: &'static [(&'static str, Expect)],
}

pub use Expect::{Any, Exact, NonZero};

/// `MESSAGES` for both producer-consumer tests; the reported sum is
/// the triangular number `N*(N-1)/2`.
const PRODCONS_MESSAGES: u32 = 32;
const PRODCONS_SUM: u32 = PRODCONS_MESSAGES * (PRODCONS_MESSAGES - 1) / 2;

/// `MESSAGES` for the event-queue test.
const PUBSUB_MESSAGES: u32 = 16;
const PUBSUB_SUM: u32 = PUBSUB_MESSAGES * (PUBSUB_MESSAGES - 1) / 2;

/// `INCREMENTS_PER_THREAD` for the two PPU counter tests; two threads
/// each do this many, so a counter short of `2 * N` means a lost
/// update.
const PPU_INCREMENTS: u32 = 64;

/// `INCREMENTS_PER_THREAD` for the two-SPU atomic test.
const SPU_INCREMENTS: u32 = 32;

/// `SIGNAL_WORD` the SPU thread window test's sender signals.
const SPU_ALIAS_SIGNAL_WORD: u32 = 0xC0DE_0001;

/// spu_intr_mbox_to_event_queue's payload, in order:
/// - the setup;
/// - the SPU's answers and mailbox counts;
/// - queue 1's two events and its empty receive;
/// - queue 2's event and its empty receive.
const INTR_MBOX_EVENT_FIELDS: [(&str, Expect); 36] = intr_mbox_event_fields();

const fn intr_mbox_event_fields() -> [(&'static str, Expect); 36] {
    const SOURCE_HIGH: u32 = (group_event::THREAD_USER_KEY >> 32) as u32;
    const SOURCE_LOW: u32 = group_event::THREAD_USER_KEY as u32;
    /// spu/main.c's CONNECTED_PORT.
    const CONNECTED_PORT: u32 = 3;
    /// ppu/main.c's GROUP_PORT_ASKED.
    const GROUP_PORT: u32 = 10;
    [
        ("step", Exact(0)),
        ("rc", Exact(0)),
        ("group_port", Exact(GROUP_PORT)),
        ("setup_pad", Exact(0)),
        // LV2 empties the interrupt mailbox it reads.
        ("intr_count_before", Exact(SPU_OUT_INTR_MBOX_DEPTH)),
        ("send_answer", Exact(0)),
        ("send_full_answer", Exact(errno::CELL_EBUSY.code)),
        ("send_unconnected_answer", Exact(errno::CELL_ENOTCONN.code)),
        ("send_group_port_answer", Exact(0)),
        // Neither throw left an answer in the inbound mailbox.
        ("in_count_after", Exact(0)),
        ("intr_count_after", Exact(SPU_OUT_INTR_MBOX_DEPTH)),
        ("spu_pad", Exact(0)),
        // The send: data2 holds the port and the word's data0, and data3
        // holds the `SPU_WrOutMbox` word.
        ("first_receive", Exact(0)),
        ("first_source_high", Exact(SOURCE_HIGH)),
        ("first_source_low", Exact(SOURCE_LOW)),
        ("first_data1_is_thread", Exact(1)),
        ("first_port", Exact(CONNECTED_PORT)),
        ("first_data0", Exact(0x12_3456)),
        ("first_data3_high", Exact(0)),
        ("first_data3", Exact(0xCAFE_BABE)),
        // The throw that filled the queue.
        ("second_receive", Exact(0)),
        ("second_source_high", Exact(SOURCE_HIGH)),
        ("second_source_low", Exact(SOURCE_LOW)),
        ("second_data1_is_thread", Exact(1)),
        ("second_port", Exact(CONNECTED_PORT)),
        ("second_data0", Exact(0x65_4321)),
        ("second_data3_high", Exact(0)),
        ("second_data3", Exact(0xDEAD_BEEF)),
        // The send and the throw to the full queue left no event.
        ("third_receive", Exact(errno::CELL_ETIMEDOUT.code)),
        ("group_receive", Exact(0)),
        ("group_data1_is_thread", Exact(1)),
        ("group_port_word", Exact(GROUP_PORT)),
        ("group_data0", Exact(0x99_9999)),
        ("group_data3", Exact(0xAAAA_AAAA)),
        ("group_second_receive", Exact(errno::CELL_ETIMEDOUT.code)),
        ("pad", Exact(0)),
    ]
}

/// spu_thread_window_slots's payload, in order:
/// - the failed step and its code;
/// - the join cause and status of its three groups;
/// - what slot 0 got through each slot's local-store alias;
/// - each slot's signal record;
/// - the yield marker.
const WINDOW_SLOTS_FIELDS: [(&str, Expect); 44] = window_slots_fields();

const fn window_slots_fields() -> [(&'static str, Expect); 44] {
    let mut fields = [("pad", Exact(0)); 44];
    fields[0] = ("step", Exact(0));
    fields[1] = ("rc", Exact(0));
    fields[2] = ("window_cause", Exact(group_join_cause::ALL_THREADS_EXIT));
    fields[3] = ("window_status", Exact(0));
    fields[4] = ("group_exit_cause", Exact(group_join_cause::GROUP_EXIT));
    // spu/main.c's GROUP_EXIT_STATUS.
    fields[5] = ("group_exit_status", Exact(0x1234));
    fields[6] = ("yield_cause", Exact(group_join_cause::ALL_THREADS_EXIT));
    fields[7] = ("yield_status", Exact(0));
    let mut slot = 0;
    while slot < 4 {
        let mut word = 0;
        while word < 4 {
            let pattern = 0x5A00_0000 | (slot as u32) << 16 | word as u32;
            fields[8 + slot * 4 + word] = ("window", Exact(pattern));
            word += 1;
        }
        // Slot 0 collects one bit from each other slot; slot n gets
        // the release word 0xB0 + n.
        let signal = if slot == 0 { 0xE } else { 0xB0 + slot as u32 };
        fields[24 + slot * 4] = ("slot", Exact(slot as u32));
        fields[25 + slot * 4] = ("signal", Exact(signal));
        slot += 1;
    }
    fields[40] = ("yield_marker", Exact(0x5949_4C44));
    fields
}

/// spu_image_segments's payload, in order:
/// - the failed step and its code;
/// - the four initialize answers;
/// - the sixteen open answers;
/// - the local store at the COPY range, at the two FILL ranges, and at
///   a range in no segment.
const IMAGE_SEGMENTS_FIELDS: [(&str, Expect); 40] = image_segments_fields();

/// The probe's open cases: short names or one long name, and the path
/// length without its NUL.
const IMAGE_OPEN_CASES: [(bool, usize); 16] = [
    (false, 255),
    (false, 256),
    (false, 269),
    (false, 270),
    (false, 271),
    (false, 1022),
    (false, 1023),
    (false, 1024),
    (false, 1025),
    (false, 2048),
    (true, 1022),
    (true, 1023),
    (true, 1024),
    (true, 1025),
    (true, 2048),
    (false, 0),
];

/// What `sys_spu_image_open` answers for a path of `length` bytes under
/// `/dev_hdd0/tmp/` that names no file.
const fn image_open_answer(short_names: bool, length: usize) -> u32 {
    const PREFIX: usize = "/dev_hdd0/tmp/".len();
    let longest = if short_names || length <= PREFIX {
        1
    } else {
        length - PREFIX
    };
    if length < IMAGE_PATH_MAX && longest > IMAGE_PATH_NAME_MAX {
        errno::CELL_ENAMETOOLONG.code
    } else {
        errno::CELL_ENOENT.code
    }
}

const fn image_segments_fields() -> [(&'static str, Expect); 40] {
    let mut fields = [("pad", Exact(0)); 40];
    fields[3] = ("type_3", Exact(errno::CELL_EINVAL.code));
    fields[4] = ("entry_3fffc", Exact(0));
    fields[5] = ("entry_40000", Exact(errno::CELL_EINVAL.code));
    let mut i = 0;
    while i < 16 {
        let (short_names, length) = IMAGE_OPEN_CASES[i];
        fields[8 + i] = ("open", Exact(image_open_answer(short_names, length)));
        i += 1;
    }
    // ppu/main.c's COPY_SOURCE; both FILL ranges read zero.
    let mut word = 0;
    while word < 4 {
        fields[24 + word] = ("copied", Exact(0xC0C0_C0C0 + 0x0101_0101 * word as u32));
        fields[28 + word] = ("filled", Exact(0));
        fields[32 + word] = ("filled_everywhere", Exact(0));
        fields[36 + word] = ("untouched", Exact(0));
        word += 1;
    }
    fields[0] = ("step", Exact(0));
    fields[1] = ("rc", Exact(0));
    fields[2] = ("initialize", Exact(0));
    fields
}

/// spu_raw_lifecycle's payload, in the order ppu/main.c lists. These
/// words take any value:
/// - a Raw SPU id: the console hands out the lowest id that no other
///   part of the system holds;
/// - an interrupt tag: its low bits count kernel objects;
/// - the status poll count: it follows the SPU's progress;
/// - an untouched PU interrupt mailbox register: it holds the physical
///   SPU's last word.
const RAW_LIFECYCLE_FIELDS: [(&str, Expect); 80] = raw_lifecycle_fields();

const fn raw_lifecycle_fields() -> [(&'static str, Expect); 80] {
    const EAGAIN: Expect = Exact(errno::CELL_EAGAIN.code);
    const EINVAL: Expect = Exact(errno::CELL_EINVAL.code);
    const ESRCH: Expect = Exact(errno::CELL_ESRCH.code);
    const EFAULT: Expect = Exact(errno::CELL_EFAULT.code);
    const EBUSY: Expect = Exact(errno::CELL_EBUSY.code);
    const OK: Expect = Exact(0);
    // spu/main.S reads 0x41 from the inbound mailbox, writes 0x42 to the
    // outbound one and 0x1EEF to the interrupt one, then stops with
    // 0x100 at local store 0x14.
    const STOPPED: u32 = 0x100 << 16 | SPU_STATUS_P;
    const STOP_NPC: u32 = 0x18;
    // SPU_Mbox_Stat: outbound count, free inbound entries << 8,
    // interrupt mailbox count << 16.
    const IN_FREE: u32 = SPU_IN_MBOX_DEPTH << 8;
    const INTR: u32 = 1 << 16;
    // Class 2 status bits B, S and M.
    const B: u32 = raw_spu::INT_STAT_MAILBOX_THRESHOLD as u32;
    const SM: u32 = (raw_spu::INT_STAT_STOP_AND_SIGNAL | raw_spu::INT_STAT_MAILBOX) as u32;
    [
        ("step", OK),
        ("rc", OK),
        ("create_before_initialize", EAGAIN),
        ("its_id", Exact(0xEEEE_EEEE)),
        ("initialize", OK),
        ("create_a", OK),
        ("id_a", Any),
        ("create_b", OK),
        ("id_b", Any),
        ("create_past_the_pool", EAGAIN),
        ("initialize_below_the_live_count", EBUSY),
        ("destroy_b", OK),
        ("destroy_b_again", ESRCH),
        ("destroy_no_such", ESRCH),
        ("create_null_id", EFAULT),
        ("tag_class_0", OK),
        ("tag_0", Any),
        ("tag_class_0_again", EAGAIN),
        ("tag_class_1", EINVAL),
        ("tag_class_2", OK),
        ("tag_2", Any),
        ("tag_no_such", ESRCH),
        ("set_mask_0", OK),
        ("get_mask_0", OK),
        ("mask_0", Exact(0x7)),
        ("set_mask_2", OK),
        ("mask_2", Exact(0xF)),
        ("set_mask_class_1", EINVAL),
        ("set_mask_no_such", ESRCH),
        ("get_stat_0", OK),
        ("stat_0", OK),
        ("set_cfg_3", OK),
        ("get_cfg", OK),
        ("cfg_3", Exact(3)),
        ("set_cfg_4", OK),
        ("cfg_4", Exact(4 & raw_spu::CFG_MASK)),
        ("recover", EINVAL),
        ("recover_no_such", ESRCH),
        ("status_before", Exact(0)),
        ("mbox_stat_before", Exact(IN_FREE)),
        ("local_store_word", Exact(0xCAFE_F00D)),
        ("mbox_stat_after_in", Exact(IN_FREE - (1 << 8))),
        ("polls", Any),
        ("status_stopped", Exact(STOPPED)),
        ("npc", Exact(STOP_NPC)),
        ("mbox_stat_stopped", Exact(INTR | IN_FREE | 1)),
        ("out_mbox", Exact(0x42)),
        ("mbox_stat_after_out", Exact(INTR | IN_FREE)),
        ("get_stat_2", OK),
        ("stat_2", Exact(B | SM)),
        ("read_puint_mb", OK),
        ("puint_mb", Exact(0x1EEF)),
        ("mbox_stat_after_puint", Exact(IN_FREE)),
        ("stat_2_after_puint", Exact(B | SM)),
        ("clear_stat_2", OK),
        ("stat_2_cleared", Exact(B)),
        ("destroy_a", OK),
        ("read_puint_mb_untouched", OK),
        ("puint_mb_untouched", Any),
        ("read_puint_mb_again", OK),
        ("puint_mb_again", Exact(0x1EEF)),
        ("create_null_id_pool_full", EAGAIN),
        ("create_after_null_id", OK),
        ("its_id", Any),
        ("set_mask_no_such_class_1", EINVAL),
        ("get_mask_no_such_class_1", EINVAL),
        ("tag_no_such_class_1", EINVAL),
        ("get_mask_null", EFAULT),
        ("create_unmapped_id", EFAULT),
        ("create_after_unmapped_id", OK),
        ("its_id", Any),
        ("read_puint_mb_unmapped", EFAULT),
        ("get_cfg_null", EFAULT),
        ("set_mask_class_3", EINVAL),
        ("tag_null", EFAULT),
        ("tag_after_null", OK),
        ("get_stat_2_c", OK),
        ("stat_2_c", Exact(B)),
        ("pad", Exact(0)),
        ("pad", Exact(0)),
    ]
}

/// spu_raw_window_modes's payload, in the order ppu/main.c lists. These
/// words take any value:
/// - the Raw SPU id: the console hands out the lowest id that no other
///   part of the system holds;
/// - the two poll counts: they follow the SPU's progress.
const RAW_WINDOW_MODES_FIELDS: [(&str, Expect); 32] = raw_window_modes_fields();

const fn raw_window_modes_fields() -> [(&'static str, Expect); 32] {
    const OK: Expect = Exact(0);
    // spu/main.S starts at local store 0x80, writes 0x11 and 0x22 to the
    // interrupt mailbox, and stops with 0x101 at 0xA4.
    const STOPPED: u32 = 0x101 << 16 | SPU_STATUS_P;
    const STOP_NPC: u32 = 0xA8;
    const IN_FREE: u32 = SPU_IN_MBOX_DEPTH << 8;
    const INTR: u32 = 1 << 16;
    const B: u32 = raw_spu::INT_STAT_MAILBOX_THRESHOLD as u32;
    const SM: u32 = (raw_spu::INT_STAT_STOP_AND_SIGNAL | raw_spu::INT_STAT_MAILBOX) as u32;
    let mut fields = [("pad", OK); 32];
    fields[0] = ("step", OK);
    fields[1] = ("rc", OK);
    fields[2] = ("initialize", OK);
    fields[3] = ("create", OK);
    fields[4] = ("id", Any);
    fields[5] = ("set_cfg_1", OK);
    fields[6] = ("get_cfg", OK);
    fields[7] = ("cfg", Exact(1));
    fields[8] = ("mbox_stat_first_message", Exact(INTR | IN_FREE));
    fields[9] = ("polls_first_message", Any);
    fields[10] = ("status_second_waits", Exact(SPU_STATUS_R | SPU_STATUS_W));
    fields[11] = ("read_puint_mb", OK);
    fields[12] = ("puint_mb", Exact(0x11));
    fields[13] = ("status_stopped", Exact(STOPPED));
    fields[14] = ("polls_stopped", Any);
    fields[15] = ("npc", Exact(STOP_NPC));
    fields[16] = ("mbox_stat_stopped", Exact(INTR | IN_FREE));
    fields[17] = ("get_stat_2", OK);
    fields[18] = ("stat_2", Exact(B | SM));
    fields[19] = ("read_puint_mb_second", OK);
    fields[20] = ("puint_mb_second", Exact(0x22));
    // SPU_Cfg 1: register 1 ORs 0x1 and 0x2; register 2 keeps the last
    // of 0x10 and 0x20.
    fields[21] = ("signal_1_or", Exact(0x3));
    fields[22] = ("signal_2_overwrite", Exact(0x20));
    fields[23] = ("destroy", OK);
    fields
}

/// The flip status once a flip lands.
const FLIP_STATUS_DONE: u32 = CELL_GCM_DISPLAY_FLIP_STATUS_DONE as u32;

/// The flip status a reset leaves.
const FLIP_STATUS_WAITING: u32 = CELL_GCM_DISPLAY_FLIP_STATUS_WAITING as u32;

/// `sys_ppu_thread_exit` value both thread microtests hand to `join`.
const THREAD_EXIT_RETVAL: u32 = 0xCAFE_F00D;

pub const CASES: &[Case] = &[
    Case {
        name: "ppu_atomic_spinlock",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("counter", Exact(2 * PPU_INCREMENTS)),
            // Retry counts are 0 whenever the scheduler does not
            // interleave the two threads inside a lwarx/stwcx window.
            ("parent_retries", Any),
            ("child_retries", Any),
        ],
    },
    Case {
        name: "ppu_cond_prodcons",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("sum", Exact(PRODCONS_SUM)),
            ("producer_errs", Exact(0)),
            ("consumer_errs", Exact(0)),
        ],
    },
    Case {
        name: "ppu_lwcond_prodcons",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("sum", Exact(PRODCONS_SUM)),
            ("producer_errs", Exact(0)),
            ("consumer_errs", Exact(0)),
            ("create_unknown_lwmutex", Exact(errno::CELL_ESRCH.code)),
            ("signal_owned_no_waiter", Exact(errno::CELL_EPERM.code)),
            ("signal_not_owned_no_waiter", Exact(0)),
            ("signal_forced_no_waiter", Exact(errno::CELL_ENOENT.code)),
            ("destroy_unknown", Exact(errno::CELL_ESRCH.code)),
            ("signal_unknown_thread", Exact(errno::CELL_ESRCH.code)),
            ("signal_all_owned_no_waiter", Exact(0)),
            ("signal_all_not_owned_no_waiter", Exact(0)),
            ("timed_wait", Exact(errno::CELL_ETIMEDOUT.code)),
            ("lock_after_timeout", Exact(0)),
            ("destroy", Exact(0)),
            ("destroy_again", Exact(errno::CELL_ESRCH.code)),
        ],
    },
    Case {
        name: "ppu_event_flag_wakeall",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            // Both waiters wake on the same set call and must observe
            // the same pattern, bits 0 and 1.
            ("waker_a", Exact(0b0011)),
            ("waker_b", Exact(0b0011)),
            ("wake_cnt", Exact(2)),
        ],
    },
    Case {
        name: "ppu_event_queue_pubsub",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("sum", Exact(PUBSUB_SUM)),
            ("errors", Exact(0)),
            ("last_data1", Exact(PUBSUB_MESSAGES - 1)),
        ],
    },
    Case {
        name: "ppu_lwmutex_counter",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("counter", Exact(2 * PPU_INCREMENTS)),
            ("lock_errors", Exact(0)),
            ("unlock_errors", Exact(0)),
        ],
    },
    Case {
        name: "ppu_semaphore_prodcons",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("sum", Exact(PRODCONS_SUM)),
            ("producer_errs", Exact(0)),
            ("consumer_errs", Exact(0)),
        ],
    },
    Case {
        name: "ppu_two_threads_disjoint_writes",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("child", Exact(0xAAAA_AAAA)),
            ("parent", Exact(0xBBBB_BBBB)),
            ("join_retval", Exact(THREAD_EXIT_RETVAL)),
        ],
    },
    Case {
        name: "process_spawn_wait",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("pid", Exact(0x0100_0600)),
            ("spawn_rc", Exact(0)),
            // The child's exit must be seen by polling, not instantly.
            ("polls", NonZero),
        ],
    },
    Case {
        name: "rsx_flip_status_transition",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            // The reset leaves the status at WAITING until the flip lands.
            ("after_reset", Exact(FLIP_STATUS_WAITING)),
            ("done_iters", Any),
            ("last_status", Exact(FLIP_STATUS_DONE)),
        ],
    },
    Case {
        name: "rsx_label_write_poll",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("label_value", Exact(0xCAFE_BABE)),
            ("expected", Exact(0xCAFE_BABE)),
            ("spin_iters", Any),
        ],
    },
    Case {
        name: "rsx_semaphore_post",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            // The report timer's low word moves with the clock. A timer
            // that stayed zero sets status bit 1, which `status` pins.
            ("timer_low", Any),
            ("spin_iters", Any),
            // The Z-pass pixel count, with nothing drawn.
            ("value", Exact(0)),
        ],
    },
    Case {
        name: "spu_atomic_cross_spu",
        max_steps: 1_000_000,
        fields: &[
            // Slot 0: SPU index 0's view.
            ("spu0_status", Exact(0)),
            ("spu0_counter_seen", Any),
            ("spu0_retries", Any),
            ("spu0_index", Exact(0)),
            // Slot 1: SPU index 1's view.
            ("spu1_status", Exact(0)),
            ("spu1_counter_seen", Any),
            ("spu1_retries", Any),
            ("spu1_index", Exact(1)),
            // Slot 2: the settled shared counter.
            ("final_pad0", Exact(0)),
            ("final_counter", Exact(2 * SPU_INCREMENTS)),
            ("final_pad2", Exact(0)),
            ("final_pad3", Exact(0)),
        ],
    },
    Case {
        name: "spu_ls_alias",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("signal_word", Exact(SPU_ALIAS_SIGNAL_WORD)),
            ("receiver_slot", Exact(1)),
            ("pad", Exact(0)),
            // The sender's buffer holds 0xA0 + i at byte i.
            ("received0", Exact(0xA0A1_A2A3)),
            ("received1", Exact(0xA4A5_A6A7)),
            ("received2", Exact(0xA8A9_AAAB)),
            ("received3", Exact(0xACAD_AEAF)),
        ],
    },
    Case {
        name: "spu_thread_window_slots",
        max_steps: 1_000_000,
        fields: &WINDOW_SLOTS_FIELDS,
    },
    Case {
        name: "spu_raw_lifecycle",
        max_steps: 1_000_000,
        fields: &RAW_LIFECYCLE_FIELDS,
    },
    Case {
        name: "spu_raw_window_modes",
        max_steps: 1_000_000,
        fields: &RAW_WINDOW_MODES_FIELDS,
    },
    Case {
        name: "spu_image_segments",
        max_steps: 1_000_000,
        fields: &IMAGE_SEGMENTS_FIELDS,
    },
    Case {
        name: "spu_lluc_publish",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            ("getllar_status", Exact(MFC_ATOMIC_STAT_G)),
            ("putllc_status", Exact(MFC_ATOMIC_STAT_S)),
            ("pad", Exact(0)),
            // The publisher's line holds PUBLISHED (0x77) at every byte.
            ("line0", Exact(0x7777_7777)),
            ("line1", Exact(0x7777_7777)),
            ("line2", Exact(0x7777_7777)),
            ("line3", Exact(0x7777_7777)),
            ("putlluc_status", Exact(MFC_ATOMIC_STAT_U)),
            ("publisher_pad0", Exact(0)),
            ("publisher_pad1", Exact(0)),
            ("publisher_pad2", Exact(0)),
        ],
    },
    Case {
        name: "spu_in_mbox_overrun",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            // Five writes into four entries leave the mailbox full, and
            // the four reads empty it.
            ("count_before", Exact(4)),
            ("count_after", Exact(0)),
            ("pad", Exact(0)),
            // The fifth write (0x55) overwrote the newest entry (0x44).
            ("message0", Exact(0x11)),
            ("message1", Exact(0x22)),
            ("message2", Exact(0x33)),
            ("message3", Exact(0x55)),
        ],
    },
    Case {
        name: "spu_interrupt_mbox",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            // The handler took the PPU's message.
            ("message", Exact(0xC0DE)),
            ("handler_event_status", Exact(event::MB)),
            // An interrupt turns interrupts off; irete turns them on.
            ("handler_mach_stat", Exact(0)),
            ("mach_stat_after", Exact(1)),
            ("in_mbox_count_after", Exact(0)),
            ("srr0_set", Exact(1)),
            ("pad", Exact(0)),
        ],
    },
    Case {
        name: "spu_intr_mbox_edges",
        max_steps: 1_000_000,
        fields: &[
            ("step", Exact(0)),
            ("rc", Exact(0)),
            // A queue connected to an SPU port destroys without the force
            // flag.
            ("destroy", Exact(0)),
            ("forced_destroy", Exact(0)),
            ("in_count_before", Exact(1)),
            // The waiting message reads first; the send's answer queues
            // behind it.
            ("first_read", Exact(0x5A5A_5A5A)),
            ("count_after_spin", Exact(1)),
            ("second_read", Exact(0)),
            ("destroyed_port_answer", Exact(errno::CELL_ENOTCONN.code)),
            ("in_count_after_throw", Exact(0)),
            ("intr_count", Exact(SPU_OUT_INTR_MBOX_DEPTH)),
            ("spu_pad", Exact(0)),
            // The send past the occupied mailbox delivered its event.
            ("receive", Exact(0)),
            ("data1_is_thread", Exact(1)),
            ("port", Exact(5)),
            ("data0", Exact(0x77_7777)),
            ("data3", Exact(0x8888_8888)),
            ("second_receive", Exact(errno::CELL_ETIMEDOUT.code)),
            ("pad0", Exact(0)),
            ("pad1", Exact(0)),
        ],
    },
    Case {
        name: "spu_intr_mbox_to_event_queue",
        max_steps: 1_000_000,
        fields: &INTR_MBOX_EVENT_FIELDS,
    },
    Case {
        name: "spu_mailbox_events",
        max_steps: 1_000_000,
        fields: &[
            ("step", Exact(0)),
            ("rc", Exact(0)),
            ("send_receive", Exact(0)),
            ("throw_receive", Exact(0)),
            ("start_mask", Exact(0)),
            // A thread starts with Le pending and Me clear. The count
            // under a Me-only mask is the Le count that the mask write
            // left behind.
            ("start_le_count", Exact(1)),
            ("start_me_count", Exact(1)),
            ("start_status", Exact(event::LE)),
            // A full outbound mailbox raises nothing until LV2 reads it.
            ("status_mbox_full", Exact(0)),
            ("send_answer", Exact(0)),
            ("status_after_send", Exact(event::LE | event::ME)),
            ("out_counts", Exact(1 << 16 | 1)),
            ("status_after_throw", Exact(event::ME)),
            // Le rose while the mask excluded it, and stayed pending.
            ("masked_le_count", Exact(1)),
            ("masked_le_status", Exact(event::LE)),
            ("out_mbox_count", Exact(1)),
            ("start_a_count", Exact(0)),
            ("start_a_status", Exact(0)),
            ("end_a_count", Exact(0)),
            ("end_a_status", Exact(0)),
            ("send_data3", Exact(0x1111_1111)),
            ("throw_data3", Exact(0x3333_3333)),
            ("throw_data2_low", Exact(0x44_4444)),
            ("pad", Exact(0)),
        ],
    },
    Case {
        name: "spu_lr_event",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            // A putllc, a getllar to another line and a putlluc reset
            // the reservation by a local action.
            ("count_after_putllc", Exact(0)),
            ("count_after_getllar", Exact(0)),
            ("count_after_putlluc", Exact(0)),
            ("putllc_status", Exact(0)),
            // The PPU's store into the reserved line.
            ("event_status", Exact(event::LR)),
            ("count_after_ack", Exact(0)),
            ("line_word", Exact(0x5A5A_5A5A)),
        ],
    },
    Case {
        name: "spu_signal_notify",
        max_steps: 1_000_000,
        fields: &[
            ("status", Exact(0)),
            // SPU_SIGNAL1_OVERWRITE | SPU_SIGNAL2_OR, read back.
            ("config", Exact(0b10)),
            ("register_number_2", Exact(errno::CELL_EINVAL.code)),
            ("unknown_thread", Exact(errno::CELL_ESRCH.code)),
            ("count1", Exact(1)),
            ("count2", Exact(1)),
            // Register 1 overwrites: 0x10 then 0x20 leaves 0x20.
            ("register1", Exact(0x20)),
            // Register 2 ORs: 0x01 then 0x02 leaves 0x03.
            ("register2", Exact(0x03)),
            ("count1_after", Exact(0)),
            ("count2_after", Exact(0)),
            ("config_of_4", Exact(errno::CELL_EINVAL.code)),
            ("config_of_unknown_thread", Exact(errno::CELL_ESRCH.code)),
        ],
    },
    Case {
        name: "spu_atomic_misaligned_lsa",
        max_steps: 1_000_000,
        fields: &MISALIGNED_LSA_FIELDS,
    },
    Case {
        name: "spu_list_bad_size",
        max_steps: 1_000_000,
        fields: &LIST_BAD_ELEMENT_FIELDS,
    },
    Case {
        name: "spu_list_bad_ea",
        max_steps: 1_000_000,
        fields: &LIST_BAD_ELEMENT_FIELDS,
    },
    Case {
        name: "spu_list_ordered_read",
        max_steps: 1_000_000,
        fields: &[
            ("failed_step", Exact(0)),
            ("failed_rc", Exact(0)),
            ("join", Exact(0)),
            ("join_cause", Exact(group_join_cause::ALL_THREADS_EXIT)),
            ("join_status", Exact(0)),
            ("exit_status", Exact(0)),
            ("block", NonZero),
            ("pad0", Exact(0)),
            // The getlb waits for the get ahead of it in its tag group,
            // so it reads the element the get brought in.
            ("barrier_list", Exact(0xF1F1_F1F1)),
            // No fence or barrier orders the getl after the get, so the
            // MFC may read its element before or after the get lands it.
            ("plain_list", Any),
            ("pad1", Exact(0)),
            ("pad2", Exact(0)),
        ],
    },
    Case {
        name: "spu_list_ordering_forms",
        max_steps: 1_000_000,
        fields: &[
            ("failed_step", Exact(0)),
            ("failed_rc", Exact(0)),
            ("join", Exact(0)),
            ("join_cause", Exact(group_join_cause::ALL_THREADS_EXIT)),
            ("join_status", Exact(0)),
            ("exit_status", Exact(0)),
            ("block", NonZero),
            ("pad0", Exact(0)),
            // Each of these waits for the get before it, so it reads the
            // element the get brought in.
            ("fence_list", Exact(0xA1A1_A1A1)),
            ("list_behind_barrier_command", Exact(0xB2B2_B2B2)),
            ("barrier_list", Exact(0xD1D1_D1D1)),
            ("plain_list_behind_barrier_list", Exact(0xD2D2_D2D2)),
            ("putb_behind_barrier_list", Exact(0xC3C3_C3C3)),
            // Nothing orders the plain put behind the barrier list get,
            // so it may read its buffer before or after the gather.
            ("plain_put_behind_barrier_list", Any),
            ("pad1", Exact(0)),
            ("pad2", Exact(0)),
        ],
    },
];

/// The payload of spu_list_bad_size and spu_list_bad_ea, in the order
/// ppu/main.c lists. The list's second element raises a DMA alignment
/// fault, so the group's exception queue receives the exception event,
/// the SPU never passes its tag wait, and the PPU terminates the group.
/// Only the first element reaches the buffer.
const LIST_BAD_ELEMENT_FIELDS: [(&str, Expect); 28] = [
    ("failed_step", Exact(0)),
    ("failed_rc", Exact(0)),
    ("receive", Exact(0)),
    ("pad0", Exact(0)),
    (
        "source_hi",
        Exact((group_event::GROUP_EXCEPTION_KEY >> 32) as u32),
    ),
    ("source_lo", Exact(group_event::GROUP_EXCEPTION_KEY as u32)),
    // The group and thread ids carry a system-wide serial on the console.
    ("data1_group", Any),
    ("data1_thread", Any),
    // The tag wait's SPU_NPC, as the console capture holds it.
    ("data2_npc", Exact(0x238)),
    ("data2_cause", Exact(group_event::EXCEPTION_DMA_ALIGNMENT)),
    // An alignment fault reports no address.
    ("data3_hi", Exact(0)),
    ("data3_lo", Exact(0)),
    ("terminate", Exact(0)),
    ("join", Exact(0)),
    ("join_cause", Exact(group_join_cause::TERMINATED)),
    // TERMINATE_STATUS.
    ("join_status", Exact(0x7E57)),
    ("disconnect", Exact(0)),
    ("group_destroy", Exact(0)),
    ("queue_destroy", Exact(0)),
    ("pad1", Exact(0)),
    ("group_id", Any),
    ("thread_id", Any),
    ("buffer", NonZero),
    ("pad2", Exact(0)),
    // Element 1's first word: the LS address of `src`.
    ("element1", NonZero),
    ("element2", Exact(0)),
    ("element2_plus_4", Exact(0)),
    ("element3", Exact(0)),
];

/// spu_atomic_misaligned_lsa's payload: a status and the three atomic
/// statuses, the SPU's 256-byte buffer after its getllar, then the two
/// 128-byte lines after the putllc and the putlluc.
const MISALIGNED_LSA_FIELDS: [(&str, Expect); 132] = misaligned_lsa_fields();

/// The payload byte at `offset` within the three data regions.
///
/// The buffer starts as `i ^ 0x55`; the getllar replaces its first line
/// with the main-storage line `0x80 + i`. The SPU then adds one to every
/// byte, and the putllc and putlluc each store one buffer line.
const fn misaligned_lsa_byte(offset: usize) -> u8 {
    /// The buffer byte at `i` after the getllar.
    const fn after_getllar(i: usize) -> u8 {
        if i < 128 {
            (0x80 + i) as u8
        } else {
            (i ^ 0x55) as u8
        }
    }
    if offset < 256 {
        after_getllar(offset)
    } else if offset < 384 {
        after_getllar(offset - 256).wrapping_add(1)
    } else {
        after_getllar(offset - 384 + 128).wrapping_add(1)
    }
}

const fn misaligned_lsa_fields() -> [(&'static str, Expect); 132] {
    let mut fields = [("", Any); 132];
    fields[0] = ("status", Exact(0));
    fields[1] = ("getllar_status", Exact(MFC_ATOMIC_STAT_G));
    // A putllc that holds its reservation reports success, 0.
    fields[2] = ("putllc_status", Exact(0));
    fields[3] = ("putlluc_status", Exact(MFC_ATOMIC_STAT_U));
    let mut word = 0;
    while word < 128 {
        let at = word * 4;
        let value = u32::from_be_bytes([
            misaligned_lsa_byte(at),
            misaligned_lsa_byte(at + 1),
            misaligned_lsa_byte(at + 2),
            misaligned_lsa_byte(at + 3),
        ]);
        let name = if word < 64 {
            "buffer"
        } else if word < 96 {
            "line1"
        } else {
            "line2"
        };
        fields[4 + word] = (name, Exact(value));
        word += 1;
    }
    fields
}
