//! The bootable microtests' expectation table: one entry per microtest,
//! one expectation per `u32` of its CGOV payload, each derived from the
//! test's documented output layout. Shared by the boot suite and the
//! hardware-capture checks.

use cellgov_ps3_abi::hw::spu::{event, MFC_ATOMIC_STAT_G, MFC_ATOMIC_STAT_S, MFC_ATOMIC_STAT_U};
use cellgov_ps3_abi::lv2::errno;

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

/// `FLIP_STATUS_DONE` -- the terminal value of the flip-status mirror.
const FLIP_STATUS_DONE: u32 = 0;

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
            ("waiting_iters", Any),
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
            // The guest-ticks clock CellGov wrote; the value moves with
            // step accounting, but a zero means no report ever landed.
            ("report_value", NonZero),
            ("spin_iters", Any),
            ("padding", Exact(0)),
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
