//! The PPE's issue timing: the measured terms and the documented ones.
//!
//! The measured values come from the `ppu_issue_timing` and
//! `ppu_issue_table` captures (CECH-2001A, 4.93 CEX,
//! `tests/micro/<probe>/ps3/cech20-cex-493/`). Each probe times each loop
//! with `mftb` five times. A measured value is one of:
//!
//! - the fastest of the five at [`PPU_CYCLES_PER_TB_TICK`], rounded to the
//!   nearest cycle;
//! - a term of the PPU's issue model that the item's doc ties to a loop's
//!   fastest run.
//!
//! The test `crates/cellgov_ppu/tests/it/console_ppu_issue_timing.rs`
//! derives the `ppu_issue_timing` values of the first kind again. It also
//! replays the loops of both probes through the issue model. The
//! documented values carry the page that states them.

use crate::lv2::{errno, memory, syscall};

/// PPE core cycles per time-base tick.
///
/// At this ratio the fastest run of the empty `bdnz` loop reads 8.015
/// cycles an iteration, eight independent `addi` 12.04, and eight
/// dependent L1 loads 40.04: whole cycles to half a percent. A ratio of
/// 39 or 41 puts the empty loop at 7.8 or 8.2. The time base runs at
/// 79.8 MHz, so the core clock is 3.192 GHz.
pub const PPU_CYCLES_PER_TB_TICK: u64 = 40;

/// The cycles from the fetch that holds a predicted-taken branch to the
/// fetch at its target.
///
/// The empty `bdnz` loop measures it: one fetch an iteration.
///
/// [CBE-Handbook p:299 s:10.2.3] starting the fetch of a new sequential
/// stream after a predicted-taken branch requires eight cycles.
pub const PPU_TAKEN_BRANCH_FETCH_CYCLES: u64 = 8;

/// The cycles from the fetch that holds a predicted-not-taken branch to
/// the fetch of the instruction after it.
///
/// Eight compares, each with a `bne` that falls through, and the loop's
/// `bdnz` take 39.1 cycles an iteration: four cycles a branch, and the
/// eight of the loop branch.
pub const PPU_NOT_TAKEN_BRANCH_FETCH_CYCLES: u64 = 4;

/// The cycles from a fixed-point `add` to a dependent one.
pub const PPU_FX_LATENCY_CYCLES: u64 = 2;

/// The cycles from a `mulld` to a dependent one, and the cycles the
/// multiply holds the fixed-point unit.
///
/// The handbook gives 15; the console's dependent `mulld` takes 14.
///
/// [CBE-Handbook p:718 s:A.1.2] `mulld mulhd mulhdu mulldo`: latency 15;
/// the FXU stalls 15 cycles after issue.
pub const PPU_MULLD_LATENCY_CYCLES: u64 = 14;

/// The cycles a dependent instruction waits for a divide beyond the
/// handbook's formula for the divide.
///
/// The probe's dependent `divd` of a 33-bit dividend by 1 takes 48
/// cycles, and the formula gives 38: a dependent instruction refetches
/// while the divide runs.
pub const PPU_DIVIDE_REFETCH_CYCLES: u64 = 10;

/// The cycles from a double-precision `fadd` or `fmadd` to a dependent one.
pub const PPU_FP_LATENCY_CYCLES: u64 = 10;

/// The cycles from a `vaddfp` to a dependent one.
pub const PPU_VMX_FLOAT_LATENCY_CYCLES: u64 = 12;

/// The cycles from a load that hits the L1 data cache to a load that uses
/// its result as an address.
pub const PPU_L1_LOAD_USE_CYCLES: u64 = 5;

/// [`PPU_L1_LOAD_USE_CYCLES`] for a chase over a shuffled 256 KB ring,
/// which the 32 KB L1 cannot hold and the 512 KB L2 can.
///
/// [CBE-Handbook p:135 s:6.1] the L1 data cache is 32 KB.
///
/// [CBE-Handbook p:139 s:6.1.5] the L2 cache is 512 KB.
pub const PPU_L2_LOAD_USE_CYCLES: u64 = 41;

/// [`PPU_L1_LOAD_USE_CYCLES`] for a chase over a shuffled 8 MB ring,
/// which neither cache can hold.
pub const PPU_MEMORY_LOAD_USE_CYCLES: u64 = 443;

/// The fewest cycles between two touches' requests to memory.
///
/// [CBE-Handbook p:146 s:6.1.6.2] the data-prefetch engine requests one
/// cache line at a time.
///
/// In the `ppu_issue_touch_flush` capture a chase through an 8 MB ring
/// that touches each line eight lines ahead takes 279.0 cycles for four
/// loads. With `dcbtst` it takes 285.0: about 70 cycles a line. The chase
/// alone takes 403 cycles a load.
pub const PPU_TOUCH_INTERVAL_CYCLES: u64 = 70;

/// The fewest cycles between a touch of a line the L2 holds and the next
/// touch's request.
///
/// In the `ppu_issue_touch_forms` capture a chase through a 256 KB ring
/// that touches each line eight lines ahead takes 9.2 cycles a load. The
/// chase alone takes [`PPU_L2_LOAD_USE_CYCLES`].
pub const PPU_TOUCH_L2_INTERVAL_CYCLES: u64 = 9;

/// The cycles after the last beat of a reload before its load-miss queue
/// entry can take another request.
///
/// [CBE-Handbook p:137 s:6.1.3.7] a load-miss queue entry takes a new request 7 cycles after the final data beat of its reload comes back from the L2.
pub const PPU_LOAD_MISS_ENTRY_REUSE_CYCLES: u64 = 7;

/// The touches of one L1 data cache set at which a load of a line that
/// one of them still brings stops waiting for the set's later reloads.
///
/// [CBE-Handbook p:134 s:6.1.2.2] The data prefetch engine holds 8 `dcbt` instructions.
///
/// The documents do not say what the PPE does with a load of a line that
/// a touch still brings. The `ppu_issue_touch_queue` capture shows:
/// - a touch of a load's line and then 1 to 6 touches of other lines in
///   its set cost the load about what those touches cost it without the
///   touch of its line;
/// - with 7 or 8 such touches, or 6 and a second touch of the load's
///   line, the load waits for its line's touch alone;
/// - touches in other sets do not count, and the spacing of the touches
///   does not change the count.
pub const PPU_L1_SET_TOUCH_LIMIT: usize = 8;

/// The cycles from a `dcbf` to a load of the same line that can start.
///
/// [CBE-Handbook p:147 s:6.1.6.5] `dcbf` can complete before the
/// operation it causes completes.
///
/// In the `ppu_issue_touch_flush` capture a chase through a 16 KB ring
/// that flushes each line before its load takes 471 cycles a load. A load
/// from memory takes [`PPU_MEMORY_LOAD_USE_CYCLES`].
pub const PPU_DCBF_LINE_BUSY_CYCLES: u64 = 28;

/// The cycles from a `dcbst` to a load of the same line that can start.
///
/// In the `ppu_issue_touch_flush` capture a chase through a 16 KB ring
/// that stores each line back before its load takes 8 cycles a load. The
/// chase alone takes [`PPU_L1_LOAD_USE_CYCLES`].
pub const PPU_DCBST_LINE_BUSY_CYCLES: u64 = 3;

/// The cycles from a `dcbf` of a line a store modified to a load of the
/// same line that can start.
///
/// In the `ppu_issue_touch_forms` capture a chase through a 16 KB ring
/// that stores to each line and then flushes it before its load takes
/// 785 cycles a load.
pub const PPU_DCBF_MODIFIED_LINE_BUSY_CYCLES: u64 = 341;

/// The cycles from a `dcbst` of a line a store modified to a load of the
/// same line that can start.
///
/// In the `ppu_issue_touch_forms` capture a chase through a 16 KB ring
/// that stores to each line and then stores it back before its load
/// takes 58 cycles a load. With no `dcbst` it takes 14.
pub const PPU_DCBST_MODIFIED_LINE_BUSY_CYCLES: u64 = 52;

/// The cycles from a store to the load of the same bytes after it,
/// while the store is still in the store queue.
///
/// Four `std` and `ld` pairs on one doubleword, each `std` storing what
/// the `ld` before it loaded, take 194.5 cycles an iteration.
pub const PPU_LOAD_HIT_STORE_CYCLES: u64 = 49;

/// The cycles from a store to the issue of the instructions after a
/// load of the same bytes, while the store is still in the store queue.
///
/// In the `ppu_issue_load_hit_store` capture, 32 independent `addi` after
/// an `std` and an `ld` of the same doubleword take 63.0 to 64.8 cycles an
/// iteration. Alone they take 33.2 to 34.5, so they issue about 30 cycles
/// after the `std`.
pub const PPU_LOAD_HIT_STORE_RELEASE_CYCLES: u64 = 30;

/// The bytes of one store-queue entry.
///
/// [CBE-Handbook p:134 s:6.1.2.2] the store subunit contains a store
/// queue (STQ, 8 x 16-byte entries).
pub const PPU_STORE_ENTRY_BYTES: u64 = 16;

/// The cycles between two store-queue entries writing their bytes into
/// the cache.
///
/// Fitted to the `ppu_issue_multiple` capture's `stmw` and `lmw` pairs
/// on one line. The pairs of 4, 8 and 16 words take 85.94, 139.34 and
/// 174.53 cycles an iteration, and with this interval the issue model
/// gives 82, 120 and 196. No one interval fits both the 8- and 16-word
/// pairs, and 30 stays within 22 cycles of each.
pub const PPU_STORE_DRAIN_CYCLES: u64 = 30;

/// The cycles between two entries of one store as they leave the store queue.
///
/// Each entry that leaves frees its slot for a younger store.
/// In the `ppu_issue_store_queue_full` capture, 1 to 6 back-to-back
/// 128-byte `stmw r0` (8 entries each) take 72 to 78 cycles a store.
/// With this interval the issue model gives 74.5, which keeps each of
/// those loops within the console's spread. With
/// [`PPU_STORE_DRAIN_CYCLES`] the third store waits for the first and
/// the model gives 151.
pub const PPU_STORE_SLOT_DRAIN_CYCLES: u64 = 8;

/// The 16-byte entries a store can hold past the store queue.
///
/// A store holds them until it writes its bytes into the cache. They are:
/// - the 8 entries of the CIU store queue;
/// - the 8 entries of 64 bytes of the L2 store queue, 4 16-byte entries
///   each.
///
/// [CBE-Handbook p:134 s:6.1.2.2] the store subunit contains a store
/// queue (STQ, 8 x 16-byte entries); the L2 has a store queue (STQ, 8 x
/// 64-byte entries).
pub const PPU_STORE_WRITE_BACK_ENTRIES: usize = 8 + 8 * 4;

/// The cycles from a mispredicted branch's issue to the earliest issue
/// of the instruction at its correct target.
///
/// The probe runs one LCG step a round, then a branch on the LCG's top
/// bit, beside the same loop with a branch that is always taken. The
/// replay of the random loop through the issue model and its branch
/// history table takes the console's 38.7 cycles an iteration at this
/// penalty.
pub const PPU_BRANCH_MISS_CYCLES: u64 = 25;

/// The cycles a `sync` holds the instructions after it once every older
/// instruction completes, with the store queue empty.
///
/// [CBE-Handbook p:722 s:A.1.2] the IU holds a `sync` at issue until all
/// queues and pipelines drain. The `sync` then forces older stores out of
/// the store queues.
///
/// The barrier costs below come from three captures. Each capture times
/// a `bdnz` loop of the barrier alone and of the barrier after a store to
/// one line. The fastest run of the same loop moves between captures, so
/// the replay of each loop through the issue model takes a cost between
/// them.
///
/// | loop | `ppu_issue_fp_store_drain` | `ppu_issue_barrier_forms` | `ppu_issue_barrier_smt` |
/// | --- | --- | --- | --- |
/// | `sync` | 230 | 223 | 232 |
/// | `std; sync` | 265 | 269 | 265 |
/// | `lwsync` | 56 | 52 | 52 |
/// | `std; lwsync` | 70 | 63 | 63 |
/// | `isync` | 52 | | 48 |
/// | `std; isync` | 70 | | |
pub const PPU_SYNC_CYCLES: u64 = 229;

/// The cycles a `sync` holds the instructions after it beyond
/// [`PPU_SYNC_CYCLES`] while an older store is still in the store queue.
pub const PPU_SYNC_STORE_CYCLES: u64 = 34;

/// The cycles an `lwsync` holds the instructions after it once every
/// older instruction completes, with the store queue empty.
///
/// [CBE-Handbook p:722 s:A.1.2] an `lwsync` drains and forces older stores
/// out of the store queues as a `sync` does. Its broadcast onto the EIB
/// does not block.
pub const PPU_LWSYNC_CYCLES: u64 = 53;

/// The cycles an `lwsync` holds the instructions after it beyond
/// [`PPU_LWSYNC_CYCLES`] while an older store is still in the store queue.
pub const PPU_LWSYNC_STORE_CYCLES: u64 = 11;

/// The cycles an `isync` holds the instructions after it once every
/// older instruction completes, before its refetch, with the store queue
/// empty.
///
/// [CBE-Handbook p:723 s:A.1.2] the IU holds an `isync` at issue until
/// all queues and pipelines drain, and flushes when it completes.
pub const PPU_ISYNC_CYCLES: u64 = 17;

/// The cycles an `isync` holds the instructions after it beyond
/// [`PPU_ISYNC_CYCLES`] while an older store is still in the store queue.
pub const PPU_ISYNC_STORE_CYCLES: u64 = 17;

/// The cycles from the last issue cycle of a store to a line the L2 does
/// not hold to the cycle the L2 has that line from memory.
///
/// [CBE-Handbook p:140 s:6.1.5] the L2's read-and-claim queues take its
/// reloads and its store misses.
///
/// [CBE-Handbook p:208 s:8.4.1.1] a store that misses the L2 makes the
/// PPE read the line from memory on the EIB, as a load that misses does.
///
/// CellGov takes the cycles a load's trip to memory adds to its trip to
/// the L2. In the `ppu_issue_page_cross_store` capture, a `sync` after a
/// `stw` to a flushed line takes 312 cycles an iteration more than after
/// a `stw` to a line the L2 holds.
pub const PPU_STORE_MISS_FETCH_CYCLES: u64 = PPU_MEMORY_LOAD_USE_CYCLES - PPU_L2_LOAD_USE_CYCLES;

/// The cycles from an `sc` to the earliest issue of the instruction
/// after it: the LV2 round trip of `sys_time_get_timebase_frequency`,
/// the cheapest call.
///
/// The probe's loop of these calls takes 949 cycles an iteration. The
/// replay of that loop through the issue model takes the console's
/// cycles at this round trip. Each capture of LV2 calls also times a loop
/// of this call. Its other entries exclude the cycles that loop runs past
/// this round trip.
pub const PPU_SYSCALL_ROUND_TRIP_CYCLES: u64 = 929;

/// The round trip of an LV2 call that no capture times.
///
/// The value is the lower median of the success round trips in
/// [`PPU_SYSCALL_ROUND_TRIPS`]: `sys_semaphore_get_value`.
///
/// The calls no capture times include:
///
/// - the PRX module calls other than load, start, stop and unload;
/// - the mmapper calls;
/// - the file system calls;
/// - the waits the table lacks, when they do not block, such as an event
///   queue receive.
///
/// Each of these creates a kernel object, writes a result to guest memory
/// or switches threads. The timed calls that create a kernel object and
/// the timed waits that block take more than this median. Of the timed
/// calls that write a result, `sys_process_get_sdk_version` and
/// `sys_event_flag_get` take less. So an untimed call can cost more than
/// this round trip.
pub const PPU_SYSCALL_UNTIMED_ROUND_TRIP_CYCLES: u64 = 6040;

/// The round trip of each call the `ppu_lv2_call_cost` capture times,
/// as `(syscall number, cycles)`: the cycles from the `sc` to the
/// earliest issue of the instruction after it.
///
/// The probe makes 4096 uncontended calls a loop, five runs each. Each
/// entry is its loop's fastest run, less the 13 cycles the rest of the
/// loop takes beside `sys_time_get_timebase_frequency`'s round trip.
/// A call that writes a result to guest memory takes 5889 to 8956
/// cycles. A call on a semaphore, event flag or mutex that writes no
/// result takes 2161 to 2587, and `sys_ppu_thread_yield` with no other
/// thread runnable takes 1451.
///
/// The `ppu_lv2_call_paths` capture times `sys_process_getpid` and
/// `sys_memory_get_page_attribute` the same way. Each entry is its loop's
/// fastest run, less the 15 cycles the rest of that probe's loop takes
/// beside the cheapest call's round trip.
///
/// The `ppu_lv2_call_untimed` capture times eight more calls on their
/// success path with the same loop body. Each entry is its loop's fastest
/// run, less 41 cycles. A semaphore wait that does not block, a condition
/// signal with no waiter and `sys_process_getppid` take 2437 to 2781
/// cycles. The calls that write a result take 5362 to 9301.
///
/// The `ppu_lv2_call_lifecycle` capture creates and destroys 128 objects
/// of each synchronization class a run. Each entry is its loop's fastest
/// run, less the 18 cycles a create loop takes and the 15 a destroy loop
/// takes beside the cheapest call's round trip. A create takes 9833 to
/// 14097 cycles and a destroy 4736 to 5480.
///
/// The `ppu_lv2_call_heavy` capture times each PRX module, thread and
/// memory call alone, 32 times, between other kernel work. Each entry is
/// its call's lower median sample, less the 2511 cycles the cheapest
/// call's median sample takes over its round trip. A start and a stop
/// each make one call of the first phase and one of the second. Their
/// entries are the mean of the two phases. The module is libadec.sprx,
/// and the allocation is 1 MiB.
///
/// | call | cycles |
/// | --- | --- |
/// | `_sys_prx_start_module`, first phase | 1741649 |
/// | `_sys_prx_start_module`, second phase | 31929 |
/// | `_sys_prx_stop_module`, first phase | 31209 |
/// | `_sys_prx_stop_module`, second phase | 1155449 |
///
/// The `ppu_lv2_call_sizes` capture times the module calls on five
/// modules of 16526 to 973170 bytes, the same way. The calls do not follow
/// the module's size:
///
/// - A load takes 67172649 to 248357809 cycles. The smallest module,
///   libfiber.sprx, takes 116098049. libadec.sprx takes 2.5 times its
///   `ppu_lv2_call_heavy` load. That capture loads no other module
///   between its loads.
/// - A first start phase takes 284209 to 1749529 cycles, and a second
///   stop phase 189049 to 1238849. libadec.sprx is the slowest in both.
///
/// The table keeps the libadec.sprx entries.
///
/// The capture also times `sys_memory_free` on blocks of 64 KiB to
/// 16 MiB: 41129 to 601929 cycles, more as the block has more pages. The
/// call names only the block's address, so its entry is the 1 MiB block's.
/// [`ppu_syscall_success_round_trip_cycles`] times a successful
/// `sys_memory_allocate` by its size. A refused one keeps the 1 MiB entry.
pub const PPU_SYSCALL_ROUND_TRIPS: [(u64, u64); 43] = [
    (syscall::PROCESS_GETPID, 1129),
    (syscall::MEMORY_GET_PAGE_ATTRIBUTE, 8506),
    (syscall::TIME_GET_TIMEBASE_FREQUENCY, 929),
    (syscall::TIME_GET_CURRENT_TIME, 8956),
    (syscall::MEMORY_GET_USER_MEMORY_SIZE, 8298),
    (syscall::PPU_THREAD_YIELD, 1451),
    (syscall::SEMAPHORE_GET_VALUE, 6040),
    (syscall::SEMAPHORE_POST, 2473),
    (syscall::SEMAPHORE_TRY_WAIT, 2587),
    (syscall::EVENT_FLAG_GET, 5889),
    (syscall::EVENT_FLAG_SET, 2545),
    (syscall::EVENT_FLAG_CLEAR, 2474),
    (syscall::MUTEX_LOCK, 2302),
    (syscall::MUTEX_UNLOCK, 2161),
    (syscall::MUTEX_TRYLOCK, 2363),
    (syscall::EVENT_QUEUE_TRY_RECEIVE, 6391),
    (syscall::SEMAPHORE_WAIT, 2530),
    (syscall::EVENT_FLAG_WAIT, 6759),
    (syscall::EVENT_FLAG_TRY_WAIT, 6830),
    (syscall::COND_SIGNAL, 2437),
    (syscall::COND_SIGNAL_ALL, 2606),
    (syscall::PROCESS_GETPPID, 2781),
    (syscall::PROCESS_GET_SDK_VERSION, 5362),
    (syscall::TIME_GET_TIMEZONE, 9301),
    (syscall::LWMUTEX_CREATE, 9833),
    (syscall::LWMUTEX_DESTROY, 5185),
    (syscall::MUTEX_CREATE, 13125),
    (syscall::MUTEX_DESTROY, 5312),
    (syscall::SEMAPHORE_CREATE, 13229),
    (syscall::SEMAPHORE_DESTROY, 4816),
    (syscall::EVENT_FLAG_CREATE, 13048),
    (syscall::EVENT_FLAG_DESTROY, 4736),
    (syscall::COND_CREATE, 14097),
    (syscall::COND_DESTROY, 5480),
    (syscall::SYS_PRX_LOAD_MODULE, 26_665_329),
    (syscall::SYS_PRX_START_MODULE, 886_789),
    (syscall::SYS_PRX_STOP_MODULE, 593_329),
    (syscall::SYS_PRX_UNLOAD_MODULE, 108_609),
    (syscall::PPU_THREAD_CREATE, 255_169),
    (syscall::PPU_THREAD_START, 11809),
    (syscall::PPU_THREAD_JOIN, 9809),
    (syscall::MEMORY_ALLOCATE, 1_601_449),
    (syscall::MEMORY_FREE, 62169),
];

/// The round trip of the call `number` selects: its entry in
/// [`PPU_SYSCALL_ROUND_TRIPS`], or [`PPU_SYSCALL_UNTIMED_ROUND_TRIP_CYCLES`].
pub const fn ppu_syscall_round_trip_cycles(number: u64) -> u64 {
    let mut i = 0;
    while i < PPU_SYSCALL_ROUND_TRIPS.len() {
        if PPU_SYSCALL_ROUND_TRIPS[i].0 == number {
            return PPU_SYSCALL_ROUND_TRIPS[i].1;
        }
        i += 1;
    }
    PPU_SYSCALL_UNTIMED_ROUND_TRIP_CYCLES
}

/// The round trip of `sys_memory_allocate` with 1 MiB pages, as
/// `(cycles for any size, cycles for each MiB)`.
///
/// The `ppu_lv2_call_sizes` capture times allocations of 1, 4 and 16 MiB,
/// 16 rounds each, as the `ppu_lv2_call_heavy` capture times a call. The
/// lower median samples, less the 2431 cycles the cheapest call's median
/// sample takes over its round trip, are 1493689, 6036369 and 23168529
/// cycles. The line runs through the 1 MiB and 16 MiB samples and gives
/// the 4 MiB one 3.4% short.
pub const PPU_MEMORY_ALLOCATE_1M_PAGE_CYCLES: (u64, u64) = (48700, 1_444_989);

/// The round trip of `sys_memory_allocate` with 64 KiB pages, as
/// `(cycles for any size, cycles for each MiB)`.
///
/// The `ppu_lv2_call_sizes` capture times allocations of 64 KiB, 1 MiB and
/// 4 MiB the same way as [`PPU_MEMORY_ALLOCATE_1M_PAGE_CYCLES`]: 196409,
/// 1966809 and 7349929 cycles. The line runs through the 64 KiB and 4 MiB
/// samples and gives the 1 MiB one 3.4% short.
pub const PPU_MEMORY_ALLOCATE_64K_PAGE_CYCLES: (u64, u64) = (82861, 1_816_767);

/// The round trip of the call `number` on its success path, with `r3` and
/// `r4` its first two arguments.
///
/// `sys_memory_allocate` takes the round trip of its size `r3` from
/// [`PPU_MEMORY_ALLOCATE_64K_PAGE_CYCLES`] when `r4` names 64 KiB pages,
/// and from [`PPU_MEMORY_ALLOCATE_1M_PAGE_CYCLES`] otherwise. Every other
/// call takes [`ppu_syscall_round_trip_cycles`].
pub const fn ppu_syscall_success_round_trip_cycles(number: u64, r3: u64, r4: u64) -> u64 {
    if number != syscall::MEMORY_ALLOCATE {
        return ppu_syscall_round_trip_cycles(number);
    }
    // The page size the LV2 allocate handler accepts for these flags.
    let (base, per_mib) =
        if memory::page_size::granule_from_flags(r4) == memory::page_size::GRANULE_64K {
            PPU_MEMORY_ALLOCATE_64K_PAGE_CYCLES
        } else {
            PPU_MEMORY_ALLOCATE_1M_PAGE_CYCLES
        };
    // The size is a 32-bit `size_t`.
    base + per_mib * (r3 & 0xFFFF_FFFF) / (1 << 20)
}

/// The round trip of the call `number` with `r3` and `r4` its first two
/// arguments and `code` its committed return, if any.
///
/// A return other than 0 takes [`ppu_syscall_outcome_round_trip_cycles`].
/// A return of 0, or no committed return, takes
/// [`ppu_syscall_success_round_trip_cycles`]. The check compares the low
/// 32 bits of `code`.
pub fn ppu_syscall_call_round_trip_cycles(number: u64, code: Option<u64>, r3: u64, r4: u64) -> u64 {
    returned_round_trip(
        number,
        code,
        ppu_syscall_success_round_trip_cycles(number, r3, r4),
    )
}

/// The round trip of the call `number` that returned `code`, with
/// `success` its round trip on the success path.
fn returned_round_trip(number: u64, code: Option<u64>, success: u64) -> u64 {
    match code {
        Some(code) if code as u32 != 0 => ppu_syscall_outcome_round_trip_cycles(number, code),
        _ => success,
    }
}

/// The round trip of each timed wait that blocks, as `(syscall number, cycles)`.
///
/// Each entry is the cycles from the wake to the earliest issue of the
/// instruction after the wait's `sc`. The `ppu_lv2_call_contended` and
/// `ppu_lv2_call_woken_forms` captures time them.
///
/// Each capture times a handoff between two threads. The main thread
/// wakes a helper and then waits. The helper wakes the main thread and
/// then waits for the next round. CellGov's run of each probe through the
/// runtime takes the console's round at these trips. Each trip in that
/// run also takes the sibling share that the other thread adds. A round,
/// less the probe's own loop, takes these cycles on the console:
///
/// | wait | wake | round |
/// | --- | --- | --- |
/// | `sys_semaphore_wait` | `sys_semaphore_post` | 49624 |
/// | `sys_event_flag_wait` | `sys_event_flag_set` | 61096 |
/// | `sys_event_queue_receive` | `sys_event_port_send` | 58725 |
/// | `sys_cond_wait` | `sys_cond_signal` | 26076 |
///
/// A condition wait releases its mutex and locks it again before it
/// returns. Its round is the shortest. The second capture repeats the
/// semaphore handoff at 51680 cycles, within that loop's spread of 8619.
pub const PPU_SYSCALL_WOKEN_ROUND_TRIPS: [(u64, u64); 4] = [
    (syscall::SEMAPHORE_WAIT, 13818),
    (syscall::EVENT_FLAG_WAIT, 17015),
    (syscall::EVENT_QUEUE_RECEIVE, 16368),
    (syscall::COND_WAIT, 10483),
];

/// The round trip of a wait that blocked and that no capture times.
///
/// The value is the lower median of [`PPU_SYSCALL_WOKEN_ROUND_TRIPS`]:
/// the woken semaphore wait.
pub const PPU_SYSCALL_WOKEN_ROUND_TRIP_CYCLES: u64 = 13818;

/// The round trip of a `sys_ppu_thread_yield` that lost its hardware thread.
///
/// The trip runs from the cycle the thread gets a hardware thread again
/// to the earliest issue of the instruction after its `sc`.
///
/// The `ppu_lv2_call_contended` capture times a yield among three
/// threads of one priority on the two hardware threads. Each yield thus
/// loses its hardware thread. A yield takes 8887 cycles on the
/// console, less the probe's own loop. CellGov's run of the probe
/// through the runtime takes the console's cycles at this trip.
pub const PPU_SYSCALL_SWITCHED_YIELD_ROUND_TRIP_CYCLES: u64 = 3289;

/// The cycles a preempted thread waits before its next instruction issues.
///
/// A higher-priority thread takes the hardware thread of a preempted
/// thread. The wait runs from the cycle the preempted thread gets a
/// hardware thread again.
///
/// The `ppu_lv2_call_preempted_forms` capture times a loop of the
/// cheapest call beside a priority-500 thread that a spinner wakes every
/// 2000 time-base ticks. Each wake takes the loop's hardware thread and
/// costs the loop 21321 to 24514 cycles. CellGov's run of the probe
/// through the runtime takes the console's cycles at this wait.
pub const PPU_PREEMPTED_RESUME_CYCLES: u64 = 7100;

/// The round trip of each cancelled wait, as `(syscall number, cycles)`.
///
/// A cancelled wait blocks and then returns `CELL_ECANCELED`:
/// - The `ppu_lv2_call_preempted_forms` capture times an event flag
///   handoff that `sys_event_flag_cancel` answers. Its round takes 65854
///   cycles, 4725 over the same handoff that a set answers. CellGov's run
///   of the probe through the runtime takes the console's round at this
///   trip.
/// - The `ppu_lv2_call_wait_ends` capture times an event queue receive
///   that `sys_event_queue_destroy` with the force flag ends. The time
///   from the helper's `mftb` before the destroy to the receiver's `mftb`
///   after its return is 29199 cycles on average. CellGov's run of the
///   probe takes the console's average at this trip.
///
/// No LV2 call cancels a semaphore wait.
pub const PPU_SYSCALL_CANCELLED_ROUND_TRIPS: [(u64, u64); 2] = [
    (syscall::EVENT_FLAG_WAIT, 19692),
    (syscall::EVENT_QUEUE_RECEIVE, 20625),
];

/// The round trip of each timed-out wait, as `(syscall number, cycles)`.
///
/// A timed-out wait blocks and then returns `CELL_ETIMEDOUT`. The trip
/// runs from the timeout's deadline. The `ppu_lv2_call_wait_ends` capture
/// times each wait alone with timeouts of 1 to 1000 us. Two `mftb` reads
/// bracket each call. Each entry is the middle of three values from the
/// wait's lower median samples at the 6, 10 and 100 us timeouts. Each
/// value is a sample less its timeout, and less the cycles the bracket
/// adds beside the cheapest call's round trip.
///
/// A timeout of 2 us or less returns sooner than its timeout plus this
/// trip: a 1 us semaphore wait takes 14240 cycles in all. At 4 us each
/// wait takes more than its timeout, this trip and the bracket, by under
/// 5 percent.
pub const PPU_SYSCALL_TIMED_OUT_ROUND_TRIPS: [(u64, u64); 3] = [
    (syscall::SEMAPHORE_WAIT, 23289),
    (syscall::EVENT_FLAG_WAIT, 29729),
    (syscall::EVENT_QUEUE_RECEIVE, 24969),
];

/// The round trip of the call `number` when it returns after its thread left the PPE.
///
/// `code` is the call's committed return, if any. `preempted` is true
/// when the thread left only for a higher-priority thread. The round
/// trip is the first of these that applies:
/// - [`PPU_SYSCALL_SWITCHED_YIELD_ROUND_TRIP_CYCLES`] for a yield;
/// - for a preempted call, the round trip of a return `code` other than
///   0, or [`ppu_syscall_round_trip_cycles`] otherwise, plus
///   [`PPU_PREEMPTED_RESUME_CYCLES`];
/// - the call's entry in [`PPU_SYSCALL_CANCELLED_ROUND_TRIPS`] when it
///   returns `CELL_ECANCELED`;
/// - the call's entry in [`PPU_SYSCALL_TIMED_OUT_ROUND_TRIPS`] when it
///   returns `CELL_ETIMEDOUT`;
/// - the call's entry in [`PPU_SYSCALL_WOKEN_ROUND_TRIPS`];
/// - [`PPU_SYSCALL_WOKEN_ROUND_TRIP_CYCLES`].
///
/// The lookup compares the low 32 bits of `code`.
pub fn ppu_syscall_resumed_round_trip_cycles(
    number: u64,
    code: Option<u64>,
    preempted: bool,
) -> u64 {
    resumed_round_trip(
        number,
        code,
        preempted,
        ppu_syscall_round_trip_cycles(number),
    )
}

/// [`ppu_syscall_resumed_round_trip_cycles`] for a call with `r3` and `r4`
/// its first two arguments.
///
/// A preempted call that returns 0, or has no committed return, takes
/// [`ppu_syscall_success_round_trip_cycles`] for its arguments as its own
/// round trip.
pub fn ppu_syscall_resumed_call_round_trip_cycles(
    number: u64,
    code: Option<u64>,
    preempted: bool,
    r3: u64,
    r4: u64,
) -> u64 {
    let success = ppu_syscall_success_round_trip_cycles(number, r3, r4);
    resumed_round_trip(number, code, preempted, success)
}

/// The resumed round trip, with `success` the call's round trip on its
/// success path.
fn resumed_round_trip(number: u64, code: Option<u64>, preempted: bool, success: u64) -> u64 {
    if number == syscall::PPU_THREAD_YIELD {
        return PPU_SYSCALL_SWITCHED_YIELD_ROUND_TRIP_CYCLES;
    }
    if preempted {
        return returned_round_trip(number, code, success) + PPU_PREEMPTED_RESUME_CYCLES;
    }
    let ended: &[(u64, u64)] = match code.map(|code| code as u32) {
        Some(c) if c == errno::CELL_ECANCELED.code => &PPU_SYSCALL_CANCELLED_ROUND_TRIPS,
        Some(c) if c == errno::CELL_ETIMEDOUT.code => &PPU_SYSCALL_TIMED_OUT_ROUND_TRIPS,
        _ => &[],
    };
    ended
        .iter()
        .chain(&PPU_SYSCALL_WOKEN_ROUND_TRIPS)
        .find(|&&(n, _)| n == number)
        .map_or(PPU_SYSCALL_WOKEN_ROUND_TRIP_CYCLES, |&(_, cycles)| cycles)
}

/// The round trip of each call the `ppu_lv2_call_paths` and
/// `ppu_lv2_call_refusals` captures time on its error path, as
/// `(syscall number, cycles)`: each call names an object ID the kernel
/// never issued and returns ESRCH.
///
/// Each entry is its loop's fastest run, less the cycles the rest of
/// that probe's loop takes beside the cheapest call's round trip: 15 in
/// the first capture, 13 in the second. A call that takes no result
/// pointer refuses the ID in 1263 to 1473 cycles, about half its success
/// path. `sys_event_flag_get` and `sys_event_flag_trywait` take 5775 and
/// 5269, as many as a success that writes a result.
///
/// The second capture repeats the first's `sys_semaphore_post` loop and
/// times it 114 cycles slower, so its three entries can sit up to that
/// far above the first capture's.
pub const PPU_SYSCALL_ERROR_ROUND_TRIPS: [(u64, u64); 13] = [
    (syscall::SEMAPHORE_POST, 1269),
    (syscall::SEMAPHORE_GET_VALUE, 1276),
    (syscall::MUTEX_LOCK, 1358),
    (syscall::MUTEX_UNLOCK, 1352),
    (syscall::EVENT_FLAG_GET, 5775),
    (syscall::EVENT_FLAG_SET, 1371),
    (syscall::EVENT_FLAG_TRY_WAIT, 5269),
    (syscall::EVENT_QUEUE_TRY_RECEIVE, 1309),
    (syscall::COND_SIGNAL, 1263),
    (syscall::EVENT_PORT_SEND, 1275),
    (syscall::SEMAPHORE_TRY_WAIT, 1429),
    (syscall::EVENT_FLAG_CLEAR, 1415),
    (syscall::MUTEX_TRYLOCK, 1473),
];

/// The round trip of each call that finds its object and then refuses,
/// from the `ppu_lv2_call_live_errors` capture, as `(syscall number,
/// error code, cycles)`.
///
/// Each entry is its loop's fastest run, less the 20 cycles the rest of
/// that probe's loop takes beside the cheapest call's round trip. A
/// semaphore or mutex call refused after the lookup takes 2150 to 2475
/// cycles, 4 to 112 fewer than its success path. A post of a negative
/// count takes 2051, 422 fewer than a post that succeeds.
/// `sys_event_flag_trywait` whose bits do not match takes 6471, as many
/// as a call that writes a result.
pub const PPU_SYSCALL_LIVE_ERROR_ROUND_TRIPS: [(u64, u32, u64); 7] = [
    (syscall::SEMAPHORE_TRY_WAIT, errno::CELL_EBUSY.code, 2475),
    (syscall::SEMAPHORE_POST, errno::CELL_EBUSY.code, 2452),
    (syscall::SEMAPHORE_POST, errno::CELL_EINVAL.code, 2051),
    (syscall::MUTEX_LOCK, errno::CELL_EDEADLK.code, 2298),
    (syscall::MUTEX_TRYLOCK, errno::CELL_EDEADLK.code, 2302),
    (syscall::MUTEX_UNLOCK, errno::CELL_EPERM.code, 2150),
    (syscall::EVENT_FLAG_TRY_WAIT, errno::CELL_EBUSY.code, 6471),
];

/// The round trip of each call the `ppu_lv2_call_arg_refusals` capture
/// times, as `(syscall number, error code, cycles)`: five calls refused
/// for a null result pointer, and a mutex trylock refused because another
/// thread holds the mutex.
///
/// Each entry is its loop's fastest run, less the 17 cycles the rest of
/// that probe's loop takes beside the cheapest call's round trip. A call
/// refused for its result pointer takes 955 to 5425 cycles fewer than its
/// success path. The trylock takes 157 more than a trylock that succeeds,
/// and 218 more than one refused because the caller holds the mutex.
pub const PPU_SYSCALL_REFUSAL_ROUND_TRIPS: [(u64, u32, u64); 6] = [
    (
        syscall::TIME_GET_CURRENT_TIME,
        errno::CELL_EFAULT.code,
        3531,
    ),
    (
        syscall::MEMORY_GET_USER_MEMORY_SIZE,
        errno::CELL_EFAULT.code,
        6433,
    ),
    (syscall::SEMAPHORE_GET_VALUE, errno::CELL_EFAULT.code, 4526),
    (syscall::EVENT_FLAG_GET, errno::CELL_EFAULT.code, 4414),
    (
        syscall::EVENT_QUEUE_TRY_RECEIVE,
        errno::CELL_EFAULT.code,
        5436,
    ),
    (syscall::MUTEX_TRYLOCK, errno::CELL_EBUSY.code, 2520),
];

/// The round trip of the call `number` selects when it returns `code`.
///
/// An error with an entry in [`PPU_SYSCALL_LIVE_ERROR_ROUND_TRIPS`] or
/// [`PPU_SYSCALL_REFUSAL_ROUND_TRIPS`] takes that entry. ESRCH takes the
/// call's entry in [`PPU_SYSCALL_ERROR_ROUND_TRIPS`]. Any other return
/// takes [`ppu_syscall_round_trip_cycles`]. The lookup compares the low
/// 32 bits of `code`, because the kernel sign-extends an error into r3.
pub fn ppu_syscall_outcome_round_trip_cycles(number: u64, code: u64) -> u64 {
    let code = code as u32;
    if let Some(&(_, _, cycles)) = PPU_SYSCALL_LIVE_ERROR_ROUND_TRIPS
        .iter()
        .chain(&PPU_SYSCALL_REFUSAL_ROUND_TRIPS)
        .find(|&&(n, c, _)| n == number && c == code)
    {
        return cycles;
    }
    let refused_id = code == errno::CELL_ESRCH.code;
    PPU_SYSCALL_ERROR_ROUND_TRIPS
        .iter()
        .find(|&&(n, _)| refused_id && n == number)
        .map_or_else(
            || ppu_syscall_round_trip_cycles(number),
            |&(_, cycles)| cycles,
        )
}

/// Eight independent `addi` and their `bdnz`, in cycles an iteration, with
/// the other hardware thread idle and then busy on independent `addi`.
pub const PPU_SMT_INDEPENDENT_ADDI_CYCLES: (u64, u64) = (12, 19);

/// Eight dependent `add` and their `bdnz`, in cycles an iteration, with
/// the other hardware thread idle and then busy.
pub const PPU_SMT_DEPENDENT_ADD_CYCLES: (u64, u64) = (16, 23);

/// Eight dependent L1 loads and their `bdnz`, in cycles an iteration,
/// with the other hardware thread idle and then busy.
pub const PPU_SMT_L1_CHASE_CYCLES: (u64, u64) = (40, 42);

/// The cycles a runnable sibling thread holds issue after each of a
/// thread's dispatch groups.
///
/// A group is the pair of instructions at one 8-byte-aligned address,
/// whether or not the two issue together. An instruction after a
/// microcoded one opens a group of its own: the `ppu_issue_smt`
/// capture's loop of `lha` and `addi` pairs takes 107.66 cycles an
/// iteration beside a busy sibling and 90.26 alone. The 17.4 between
/// is near the nine holds, 18 cycles, that the model takes.
///
/// With this hold, the issue model times the three busy-sibling loops
/// of `ppu_issue_timing` at 19, 23 and 41 cycles an iteration against
/// the console's 19.24, 23.34 and 41.52: each inside the console's
/// spread.
///
/// [CBE-Handbook p:754 s:A.6.1] the threads take dispatch in turn, a
/// thread every other cycle.
pub const PPU_SMT_SIBLING_HOLD_CYCLES: u64 = 2;

/// The most cycles a sibling thread's stalls at the shared issue point
/// add to each of a thread's instructions, past the cycle the
/// instruction is ready: what a sibling of divides, square roots or long
/// operand waits adds. Microcode adds at most
/// [`PPU_SMT_MICROCODE_STALL_CYCLES`].
///
/// The `ppu_issue_shared` capture times five chains beside a sibling
/// running dependent `divd`, independent `divd`, `divw`, `fdiv`, `fsqrt`
/// or microcoded `lha`. Each instruction of the `add`, `addi`, `fadd` and
/// `ld` chains takes about its own latency and 13 cycles more, whichever
/// of the six the sibling runs and whichever unit the chain uses, though
/// the sibling's instructions take between 24 and 94 cycles each. The
/// `fadd` chain beside `lha` takes 7.7 more. A dependent `mulli` takes
/// about 16 to 25 more.
///
/// The `ppu_issue_loop_shape` capture, taken twice, puts the mean wait
/// beside a dependent `divd` sibling that holds its thread at 19 to 30
/// cycles, and within one loop's five runs it spreads from near zero to
/// about 36. A sibling loop in assembly puts it above a loop that returns
/// to C between rounds in five of six cells. A time-base read between
/// rounds puts it in no order. The 13 here is the level the other
/// captures hold.
///
/// [CBE-Handbook p:301 s:10.2.3.1] a stall at IS2 stalls both threads.
///
/// [CBE-Handbook p:753 s:A.5] a nonpipelined or microcoded instruction
/// issues alone.
pub const PPU_SMT_SHARED_STALL_CYCLES: u64 = 13;

/// The interval between two dispatch groups of a thread, as a share of a
/// sibling's divide period, while the sibling's divides stall the shared
/// issue point back to back.
///
/// The `ppu_issue_fdiv_window` capture times loops of K independent
/// `addi` and a `bdnz` beside a lower-priority sibling. The sibling runs
/// back-to-back dependent `fdiv` and never waits for a divide's result.
/// Most runs of each loop take one of these intervals an iteration for
/// each dispatch group. In divide periods, an iteration of most runs
/// takes:
///
/// - two groups: 2.69;
/// - three groups: 4.02;
/// - four groups: 5.36;
/// - five groups: 6.69;
/// - seven groups: 9.36;
/// - nine groups: 12.03.
///
/// A loop of one group takes 1.47. The `ppu_issue_fdiv_chain` capture
/// puts a dependent `add` chain on the same intervals, so each dispatch
/// group waits once. The `ppu_issue_divide_window_forms` capture gives
/// the same periods beside back-to-back `fsqrt`.
///
/// The console's interval is about 4/3 of its divide period of 78 cycles.
/// CellGov's divides run 74 cycles apart. At 5/4 of that, the model times
/// each loop of `ppu_issue_fdiv_window` and `ppu_issue_fdiv_chain` within
/// 11% of the console's median run.
///
/// [CBE-Handbook p:301 s:10.2.3.1] a stall at IS2 stalls both threads.
pub const PPU_SMT_DIVIDE_WINDOW_SHARE: (u64, u64) = (5, 4);

/// The most cycles a sibling's microcoded and other long non-divide
/// instructions add to each of a thread's instructions, past the cycle
/// the instruction is ready.
///
/// Beside a sibling of microcoded `lha` that holds its hardware thread,
/// each `add` of a dependent chain in the `ppu_issue_layout` capture
/// waits 9.3 to 10.2 cycles at seven line offsets of the chain when the
/// sibling's loop sits at +0x00, and 7.4 and 8.7 at the two held offsets
/// when it sits at +0x40. Each instruction of an FXU or load chain waits
/// 7.1 to 9.5 cycles in `ppu_issue_units` and 12.5 to 14.4 in
/// `ppu_issue_shared`. With ten, 6 of the 21 cells beside `lha` that the
/// three captures replay stay outside the replay tolerance: 2 in
/// `ppu_issue_layout`, 3 in `ppu_issue_units` and 1 in
/// `ppu_issue_shared`.
///
/// The `ppu_issue_lha_place` capture moves the sibling's loop through
/// every 16-byte offset of a line and puts its loads on the main
/// thread's line once. Each held cell waits 13.0 to 14.8 cycles, at
/// seven of the eight offsets and on either line; at +0x30 neither chain
/// held. The level moves with the probe and stays level across the
/// sibling's placements.
///
/// No capture times a sibling of `mfcr` or `mfocrf`, the other long
/// instructions this cap covers.
pub const PPU_SMT_MICROCODE_STALL_CYCLES: u64 = 10;

/// The cycles a sibling that spends all its cycles on `mtvscr` adds to
/// each of a thread's instructions, of any class but a store, past the
/// cycle the instruction is ready.
///
/// [CBE-Handbook p:742 s:A.3.2] an `mtvscr` stalls at issue until every
/// older instruction completes. [CBE-Handbook p:301 s:10.2.3.1] a stall
/// at the shared issue point stalls both threads.
///
/// Beside a sibling of back-to-back `mtvscr` a run's cycles move with how
/// the two threads line up, so the figures here are the mean of each
/// chain's five runs. Each instruction of a chain waits these cycles more
/// than beside a sibling of `addi`:
///
/// - independent `vaddubm` in the `ppu_issue_vscr_smt` capture: 22.5;
/// - independent `addi` in the same capture: 19.0;
/// - dependent `fadd` in the same capture: 24.9;
/// - `lwz` in the `ppu_issue_vscr_smt_lsu` capture: 20.3;
/// - `lvx` in the same capture: 23.2.
///
/// With 22, the model times the eight chains of the two captures beside
/// that sibling within 23% of the mean.
pub const PPU_SMT_MTVSCR_STALL_CYCLES: u64 = 22;

/// The share of [`PPU_SMT_MTVSCR_STALL_CYCLES`], as a fraction numerator
/// over denominator, that a sibling's `mtvscr` adds to each store.
///
/// In the `ppu_issue_vscr_smt_lsu` capture each `stw` of a chain beside a
/// sibling of back-to-back `mtvscr` waits 9.1 cycles more than beside a
/// sibling of `addi`. That figure, the mean of five runs, is about half
/// of a load's 20.3.
pub const PPU_SMT_MTVSCR_STORE_STALL_SHARE: (u64, u64) = (1, 2);

/// The share of [`PPU_SMT_MTVSCR_STALL_CYCLES`], as a fraction numerator
/// over denominator, that a sibling's `mtvscr` adds for each of its recent
/// cycles in which it waits for its own held VSU work.
///
/// The `ppu_issue_vscr_smt` capture fixes the share: beside a sibling of an
/// `mtvscr` and seven `vaddubm`, 16 independent `addi` take 98.79 cycles
/// an iteration on the mean of five runs. With a quarter the model gives
/// 102, and with none 47.
///
/// A store waits all of this share. In the `ppu_issue_vscr_smt_drain`
/// capture, 8 `stw` beside that sibling take 47.8 cycles an iteration on
/// the mean of five runs. The model gives 52.8 with all of the share, and
/// 30.7 with [`PPU_SMT_MTVSCR_STORE_STALL_SHARE`] of it.
pub const PPU_SMT_MTVSCR_DRAIN_STALL_SHARE: (u64, u64) = (1, 4);

/// The share of a sibling's mean wait past its in-order floor, for its
/// operands or a busy unit, as a fraction numerator over denominator,
/// that it adds to each of a thread's instructions, up to
/// [`PPU_SMT_SHARED_STALL_CYCLES`].
///
/// A sibling of dependent `mulld`, whose instructions each wait about 13
/// cycles, adds about 6 to 7 cycles to each instruction of the `add` and
/// `addi` chains in the `ppu_issue_shared` capture, and 8 to 9 in
/// `ppu_issue_units`. Half the wait replays the `ppu_issue_shared`
/// capture's `add` chain within the replay tolerance. The `ppu_issue_units`
/// capture's `add` chain beside `mulld` waits more, and its replay keeps
/// that cell as a residual.
pub const PPU_SMT_DEPENDENT_STALL_SHARE: (u64, u64) = (1, 2);

/// The share of a sibling's mean wait past its in-order floor, as a
/// fraction numerator over denominator, that it adds to each of a
/// thread's multiplies, up to [`PPU_SMT_SHARED_STALL_CYCLES`].
///
/// In the `ppu_issue_multiply` capture a sibling of dependent `mulld`,
/// whose instructions each wait about 13 cycles, adds 8.8 to 9.2 cycles
/// to each multiply in the slowest of five runs.
pub const PPU_SMT_MULTIPLY_DEPENDENT_STALL_SHARE: (u64, u64) = (2, 3);

/// The share, as a fraction numerator over denominator, of the stall a
/// sibling's microcode and operand waits add that an instruction issuing
/// to the VSU alone waits. A sibling's divides and square roots stall it
/// in full.
///
/// In the `ppu_issue_units` capture each instruction of an FP chain waits
/// 4 to 7 cycles beside a sibling of microcoded `lha` or dependent
/// `mulld`, and each instruction of an FXU or load chain 6 to 10. Beside
/// a sibling of `divd` or `fdiv` both wait 11 to 17, apart from the `ld`
/// chain beside `divd`, whose runs spread from 1 to 15. In
/// `ppu_issue_shared` a dependent `fadd` waits 7.7 beside `lha` and an
/// `add` 14.4.
///
/// [CBE-Handbook p:301 s:10.2.3.1] a stall at the VSU issue queue stalls
/// both threads only when it also stalls IS2, and the queue's 2-stage
/// buffer accepts up to four more instructions before it does.
pub const PPU_SMT_VSU_STALL_SHARE: (u64, u64) = (1, 2);

/// The cycles between two fetches for one hardware thread.
///
/// [CBE-Handbook p:299 s:10.2.3] instruction fetching strictly
/// alternates between the two threads on successive cycles.
pub const PPU_FETCH_INTERVAL_CYCLES: u64 = 2;

/// The bytes one fetch reads: four instructions.
///
/// [CBE-Handbook p:135 s:6.1.3.2] the PPE can fetch up to four
/// instructions per cycle.
pub const PPU_FETCH_BYTES: u64 = 16;

/// The instructions a thread's fetch can run ahead of its issue.
///
/// The instruction buffer holds four instructions, and each of the seven
/// stages from the buffer's load to issue holds two.
///
/// [CBE-Handbook p:754 s:A.6.1] IB1 loads the instruction buffer and
/// IB2 unloads it.
///
/// [CBE-Handbook p:755 s:A.6.1] decode takes ID1 to ID3, and issue IS1
/// to IS2.
pub const PPU_FETCH_AHEAD_INSTRUCTIONS: u64 = 18;

/// The cycles from a compare to a dependent branch or CR operation.
///
/// [CBE-Handbook p:719 s:A.1.2] `cmpi cmp cmpli cmpl`: latency 1.
pub const PPU_COMPARE_LATENCY_CYCLES: u64 = 1;

/// The cycles from a branch-unit instruction to a dependent one.
///
/// The branch-unit instructions are:
///
/// - a branch's update of CTR or LR;
/// - a CR logical operation;
/// - a move to or from LR or CTR.
///
/// [CBE-Handbook p:715 s:A.1.2] `bc`: latency 1 (lr / ctr).
pub const PPU_BRANCH_UNIT_LATENCY_CYCLES: u64 = 1;

/// The latency of a `mulli`, and the cycles it holds the fixed-point unit.
///
/// The handbook gives 6; eight dependent `mulli` and their `bdnz` take
/// 65.8 cycles an iteration on the console.
///
/// [CBE-Handbook p:718 s:A.1.2] `mulli`: latency 6; the FXU stalls 6
/// cycles after issue.
pub const PPU_MULLI_LATENCY_CYCLES: u64 = 8;

/// The latency of a word multiply, and the cycles it holds the
/// fixed-point unit.
///
/// The handbook gives 9; eight dependent `mullw` and their `bdnz` take
/// 90.8 cycles an iteration on the console.
///
/// [CBE-Handbook p:718 s:A.1.2] `mullw mulhw mulhwu mullwo`: latency 9;
/// the FXU stalls 9 cycles after issue.
pub const PPU_MULLW_LATENCY_CYCLES: u64 = 11;

/// The fewest cycles a divide takes.
///
/// [CBE-Handbook p:718 s:A.1.2] word and doubleword minimum 10; an
/// overflow case completes in 10.
pub const PPU_DIVIDE_MIN_CYCLES: u64 = 10;

/// The most cycles a word divide takes.
///
/// [CBE-Handbook p:718 s:A.1.2] word maximum 38.
pub const PPU_DIVIDE_WORD_MAX_CYCLES: u64 = 38;

/// The most cycles a doubleword divide takes.
///
/// [CBE-Handbook p:718 s:A.1.2] doubleword maximum 70.
pub const PPU_DIVIDE_DOUBLEWORD_MAX_CYCLES: u64 = 70;

/// The cycles from a fixed-point load that hits the L1 to a dependent
/// fixed-point operation.
///
/// [CBE-Handbook p:716 s:A.1.2] 2-cycle latency for a load followed by
/// a dependent FXU operation.
pub const PPU_LOAD_FX_USE_CYCLES: u64 = 2;

/// The cycles from a floating-point load that hits the L1 to a
/// dependent floating-point instruction.
///
/// [CBE-Handbook p:720 s:A.1.2] `lfs lfsx lfd lfdx`: latency 1.
pub const PPU_FP_LOAD_LATENCY_CYCLES: u64 = 1;

/// The cycles from the cycle a floating-point store runs in the FPU to
/// the cycle its data reaches the store queue.
///
/// [CBE-Handbook p:720 s:A.1.2] `stfs stfsx stfd stfdx`: dispatched to
/// both the FPU and the LSU.
///
/// In the `ppu_issue_vscr_mem` and `ppu_issue_vscr_mem_forms` captures,
/// a loop of an `mtvscr` and an `stfd`, `stfs` or `stfiwx` takes 70.62 to
/// 71.34 cycles an iteration. That is 11.1 to 11.8 more than the 59.55
/// with an `stvx`. The `mtvscr` waits for the older FP store's data.
pub const PPU_FP_STORE_DATA_CYCLES: u64 = 13;

/// The cycles from the cycle a floating-point store runs in the FPU to
/// the cycle the store completes, for a drain that does not use the VSU.
///
/// In the `ppu_issue_fp_store_drain` capture, a `sync`, `lwsync`, `isync`
/// or `sc` after an `stfd` takes 19 to 23 cycles an iteration more than
/// after an `std`. An `mfvscr` takes 13 more.
pub const PPU_FP_STORE_COMPLETE_CYCLES: u64 = 24;

/// The own cycles a thread takes for each instruction beside a runnable
/// sibling; the sibling's divides stretch them.
///
/// [CBE-Handbook p:300 s:10.2.3] the threads take turns at dispatch, so a
/// thread beside a busy sibling dispatches every other cycle.
pub const PPU_SMT_OWN_ISSUE_CYCLES: u64 = 2;

/// The stages the VSU issue queue holds past IS2 behind a divide in
/// progress.
///
/// A stage holds one of:
/// - a VSU instruction that waits for the divide;
/// - the bubble of an instruction that does not use the VSU.
///
/// A VSU instruction past that count issues once the oldest stage leaves
/// the queue.
///
/// In the `ppu_issue_viq_depth` capture a sibling runs N dependent `fdiv`
/// and then 256 independent `addi` each round. Beside it a loop of 16
/// independent `addi` takes 34.5 to 35.5 cycles an iteration for N from 1
/// to 7, 40.1 for N = 8, 48.2 for 10 and 75.8 for 16. Past 7 each divide
/// adds about one divide latency to the sibling's round. One divide runs
/// and six wait in the queue. In the `ppu_issue_viq_spaced` capture an
/// `addi` after each `fdiv` takes a stage. The loop then slows from N = 5,
/// at 39.25 cycles an iteration, and by about 4.5 cycles for each divide
/// more.
///
/// [CBE-Handbook p:301 s:10.2.3.1] a stall at the VSU issue queue stalls
/// both threads only when it also stalls IS2. The queue's 2-stage buffer
/// takes up to four more instructions while VQ8 stalls.
///
/// [CBE-Handbook p:757 s:A.6.2] an issue cycle with no VSU instruction
/// sends a bubble down the queue, and the queue does not compress bubbles.
pub const PPU_VIQ_HELD_OPS: usize = 6;

/// The cycles a double-precision divide takes and holds the
/// floating-point unit.
///
/// [CBE-Handbook p:721 s:A.1.2] `fdiv fdivs` (IEEE): latency 74.
///
/// [CBE-Handbook p:754 s:A.5] no FPU instruction, loads and stores
/// included, issues after an `fdiv` or `fsqrt` until it completes.
pub const PPU_FDIV_CYCLES: u64 = 74;

/// The cycles a square root takes and holds the floating-point unit.
///
/// [CBE-Handbook p:721 s:A.1.2] `fsqrt fsqrts` (IEEE): latency 84.
pub const PPU_FSQRT_CYCLES: u64 = 84;

/// The cycles a recording floating-point instruction's CR field takes
/// beyond its result.
///
/// The handbook gives 17. Eight dependent `fadd.`, each of which writes
/// CR1 again, and their `bdnz` take 238.9 cycles an iteration on the
/// console: 30 cycles each.
///
/// [CBE-Handbook p:721 s:A.1.2] +17 (cr).
pub const PPU_FP_RECORD_EXTRA_CYCLES: u64 = 20;

/// The cycles from a move from the FPSCR to a dependent instruction.
///
/// Eight `mffs` to one register and their `bdnz` take 248.2 cycles an
/// iteration on the console.
///
/// [CBE-Handbook p:722 s:A.1.2] `mffs`: 11 to 28, stalled until older
/// vector-scalar operations complete.
pub const PPU_MFFS_LATENCY_CYCLES: u64 = 31;

/// The cycles from the cycle a vector-scalar op other than a load or
/// store, or a floating-point store's data, runs to the cycle it completes
/// for a younger `mffs`.
///
/// [CBE-Handbook p:722 s:A.1.2] `mffs` is stalled at VQ8 until older
/// vector-scalar operations complete.
///
/// In the `ppu_issue_mffs_gap` capture an `mffs`, 16 `li` and an `fadd`
/// take 44.00 cycles an iteration, and 16 `li`, an `fadd` and an `mffs`
/// take 42.00: the `mffs` waits past the `fadd` result. In the
/// `ppu_issue_mffs_stfd` and `ppu_issue_mffs_far` captures an `stfd` in
/// place of the `fadd` takes the same cycles an iteration.
pub const PPU_MFFS_VSU_COMPLETE_CYCLES: u64 = 28;

/// The cycles from the cycle an `mffs` runs to the earliest cycle a
/// younger vector-scalar op runs at, when a taken branch lands on that op
/// at an odd word.
///
/// The `ppu_issue_mffs_odd_target` capture times an `mffs`, one or eight
/// `li`, a `b` and an `fadd`, and also an `mffs`, a `b` and a `vaddubm`.
/// With the target at an odd word each loop takes 40.00 cycles an
/// iteration. With the target at an even word each takes 32.00 to 37.00.
/// The next `mffs` waits [`PPU_MFFS_VSU_COMPLETE_CYCLES`] past the op, so
/// the op runs 12 cycles after the `mffs` before it. An `mfvscr` in place
/// of the `mffs` takes 42.00 at either word.
pub const PPU_MFFS_ODD_TARGET_VSU_CYCLES: u64 = 12;

/// The cycles from the cycle an `mffs` runs to the earliest cycle any
/// younger vector-scalar op other than a load or store, or a younger
/// floating-point store's data, runs at.
///
/// The `ppu_issue_mffs_gap` and `ppu_issue_mffs_odd_target` captures time
/// an `mffs` and an `fadd` or a `vaddubm` two or three instructions after
/// it. Each loop takes 32.00 or 33.00 cycles an iteration. The op then
/// runs 4 or 5 cycles after the `mffs`, and the next `mffs` waits
/// [`PPU_MFFS_VSU_COMPLETE_CYCLES`] past it.
pub const PPU_MFFS_YOUNGER_VSU_CYCLES: u64 = 4;

/// The cycles a move from the CR takes and holds the branch unit.
///
/// The handbook gives about 34; eight `mfcr` to one register and their
/// `bdnz` take 212.6 cycles an iteration on the console.
///
/// [CBE-Handbook p:720 s:A.1.2] `mfcr`, `mfocrf`: about 34, one per 34
/// cycles.
pub const PPU_MFCR_CYCLES: u64 = 27;

/// The cycles a move from the XER takes and holds the fixed-point unit.
///
/// In the `ppu_issue_mfxer` capture, eight `mfxer` and their `bdnz` take
/// 219.44 cycles an iteration. An `mfxer` and eight `li` take 34.77
/// cycles: the `li` wait for the `mfxer`.
///
/// [CBE-Handbook p:720 s:A.1.2] `mfspr` of an SPR other than LR and CTR:
/// handled by the FXU and not pipelined; the pipeline stalls until
/// writeback.
pub const PPU_MFXER_CYCLES: u64 = 27;

/// The cycles from a move from the time base to a dependent instruction.
///
/// The handbook gives about 40; eight `mftb` to one register and their
/// `bdnz` take 365.6 cycles an iteration on the console.
///
/// [CBE-Handbook p:720 s:A.1.2] `mftb`: about 40.
pub const PPU_MFTB_LATENCY_CYCLES: u64 = 46;

/// The cycles a move from the time base holds the fixed-point unit: one
/// time-base tick.
///
/// On the console, eight back-to-back `mftb` to eight registers read
/// values at least one tick apart, most often one (`ppu_timebase_reads`).
/// One read a tick keeps every value distinct.
pub const PPU_MFTB_ISSUE_CYCLES: u64 = PPU_CYCLES_PER_TB_TICK;

/// The microwords of an `lha`.
///
/// The handbook gives 7. Four independent `lha` from one L1 line and
/// their `bdnz` take 86.6 cycles an iteration on the console: 21 cycles
/// each, the microcode ROM's 11 and 10 microwords.
///
/// [CBE-Handbook p:729 s:A.1.3.1] `lha`: 7 microwords.
pub const PPU_LHA_MICROWORDS: u8 = 10;

/// The microwords a store multiple or store string runs before it reads
/// its first source.
///
/// In the `ppu_issue_store_data_wait` capture neither `stmw r24` nor
/// `stswi r24` waits for a `lwz r24` just before it, and a `stw r24`
/// does.
pub const PPU_STORE_SOURCE_LEAD_MICROWORDS: u8 = 2;

/// The cycles a store multiple holds issue beyond its microwords.
///
/// [CBE-Handbook p:729 s:A.1.3.1] gives `stmw` 2 + 1 x words microwords.
/// In the `ppu_issue_stmw_words` capture, a loop of one `stmw` of 1, 2, 4
/// or 8 words takes 18.0, 19.1, 21.7 and 26.1 cycles an iteration. The
/// replay of each loop through the issue model lies within the console's
/// spread at this hold.
pub const PPU_STORE_MULTIPLE_EXTRA_CYCLES: u8 = 3;

/// The bytes of one L1 data-cache sector.
///
/// [CBE-Handbook p:730 s:A.1.3.2] the L1 data cache is physically
/// implemented with 32-byte sectors.
pub const PPU_L1_SECTOR_BYTES: u64 = 32;

/// The cycles a doubleword integer load across an L1 sector costs.
///
/// The cost applies when each word half of the load stays inside one
/// sector. It delays the load's result and the issue of the instructions
/// after it. [CBE-Handbook p:730 s:A.1.3.2] the pipeline is flushed when such a
/// load reaches EX7. The load is then refetched and split into microcode.
///
/// In the `ppu_issue_line_cross` capture, a warm chase of `ld` at offset
/// 92 or 124 of a node takes 54 cycles an iteration. At offset 112 or 116
/// it takes 8. The cost is the same whether or not the second sector is
/// in the next line. The replay of that chase through the issue model
/// takes the console's 54 cycles at this cost.
pub const PPU_SECTOR_CROSS_LOAD_CYCLES: u64 = 49;

/// The cycles a doubleword integer load across an L1 sector adds to
/// [`PPU_SECTOR_CROSS_LOAD_CYCLES`] when one of its word halves also
/// crosses the sector.
///
/// [CBE-Handbook p:730 s:A.1.3.2] when a word of the split load still
/// crosses, the load is flushed again and tried at byte granularity.
///
/// In the `ppu_issue_sector_cross` capture, a warm chase of `ldx` at
/// sector offset 25, 26, 29 or 31 takes 70 cycles an iteration. At
/// sector offset 28 it takes 56. The replay of that chase through the
/// issue model takes the console's 70 cycles at this cost.
pub const PPU_SECTOR_CROSS_BYTE_RETRY_CYCLES: u64 = 16;

/// The cycles a word integer load across an L1 sector costs.
///
/// The cost delays the load's result and the issue of the instructions
/// after it.
///
/// In the `ppu_issue_sector_cross` capture, a warm chase of `lwzx` at
/// sector offset 29, 30 or 31 takes 62 cycles an iteration, whether or
/// not a halfword of the split crosses. The replay of that chase through
/// the issue model takes the console's 62 cycles at this cost.
pub const PPU_SECTOR_CROSS_WORD_LOAD_CYCLES: u64 = 57;

/// The cycles a halfword integer load across an L1 sector costs.
///
/// The cost delays the load's result and the issue of the instructions
/// after it.
///
/// In the `ppu_issue_sector_cross` capture, a warm chase of
/// `lhzx; add` at sector offset 31 takes 58 cycles an iteration. The
/// replay of that chase through the issue model takes the console's 58
/// cycles at this cost.
pub const PPU_SECTOR_CROSS_HALFWORD_LOAD_CYCLES: u64 = 54;

/// The cycles before the first microword of a microcoded instruction
/// issues.
///
/// [CBE-Handbook p:724 s:A.1.2] a minimum of 11 cycles is required
/// before the first instruction is received from the microcode ROM.
pub const PPU_MICROCODE_ROM_CYCLES: u64 = 11;

/// The cycles from a vector load to a dependent instruction.
///
/// [CBE-Handbook p:740 s:A.3.2] vector loads: latency 2.
pub const PPU_VMX_LOAD_LATENCY_CYCLES: u64 = 2;

/// The cycles from a vector permute-class instruction to a dependent
/// instruction.
///
/// The permute class is:
///
/// - permute;
/// - pack and unpack;
/// - merge;
/// - splat.
///
/// [CBE-Handbook p:740 s:A.3.2] permute: latency 4.
pub const PPU_VMX_PERMUTE_LATENCY_CYCLES: u64 = 4;

/// The cycles from a simple vector operation to a dependent instruction.
///
/// The simple operations are:
///
/// - integer add;
/// - logical and select;
/// - rotate and shift;
/// - compare;
/// - minimum and maximum.
///
/// [CBE-Handbook p:741 s:A.3.2] simple: latency 4.
pub const PPU_VMX_SIMPLE_LATENCY_CYCLES: u64 = 4;

/// The share of an LV2 call's round trip a runnable sibling thread adds
/// when it makes no LV2 calls of its own.
///
/// In the `ppu_lv2_call_sibling` capture the cheapest call takes 952
/// cycles alone and 1120 beside a sibling of `addi`. A yield that does not
/// switch threads takes 1444 and 1811.
pub const PPU_SMT_SYSCALL_SIBLING_SHARE: (u64, u64) = (1, 5);

/// The share of an LV2 call's round trip a runnable sibling thread adds
/// when it spends all its recent cycles in LV2 calls of its own.
///
/// In the `ppu_lv2_call_sibling` capture the cheapest call takes 1670
/// cycles beside a sibling that makes the same call, and 1688 beside one
/// that yields. A yield that does not switch threads takes 2689 and 2535.
pub const PPU_SMT_SYSCALL_KERNEL_SIBLING_SHARE: (u64, u64) = (4, 5);

/// The cycles from an `mfvscr` to the cycle it completes, for a younger
/// `mfvscr` or floating-point store.
///
/// [CBE-Handbook p:742 s:A.3.2] Table A-8 gives `mfvscr` no latency.
///
/// In the `ppu_issue_vscr` capture a loop of one `mfvscr` takes 40.25
/// cycles an iteration. The next `mfvscr` waits at issue for the older one
/// to complete.
pub const PPU_MFVSCR_LATENCY_CYCLES: u64 = 40;

/// The cycles from an `mfvscr` to a dependent instruction.
///
/// [CBE-Handbook p:742 s:A.3.2] Table A-8 gives `mfvscr` no latency.
///
/// In the `ppu_issue_vscr` capture an `mfvscr v1` and a dependent
/// `vaddubm v2, v1, v1` take 44.31 cycles an iteration. The `vaddubm`
/// runs 23 cycles after the `mfvscr`, and the next `mfvscr` issues
/// [`PPU_MFVSCR_VSU_COMPLETE_CYCLES`] after it.
pub const PPU_MFVSCR_RESULT_CYCLES: u64 = 23;

/// The cycles from an `mfvscr` to the first younger instruction that can
/// issue.
///
/// [CBE-Handbook p:742 s:A.3.2] `mfvscr` stalls at issue until every
/// older instruction completes.
///
/// In the `ppu_issue_mfvscr_younger` capture an `mfvscr` and 32, 48 and
/// 64 independent `li` take 53.00, 70.92 and 86.17 cycles an iteration.
/// The `li` issue one a cycle, from about 20 cycles after the `mfvscr`.
pub const PPU_MFVSCR_YOUNGER_HOLD_CYCLES: u64 = 20;

/// The cycles from the cycle a vector-scalar op runs to the cycle it
/// completes, for a younger `mfvscr`.
///
/// [CBE-Handbook p:742 s:A.3.2] `mfvscr` stalls at issue until every
/// older instruction completes.
///
/// In the `ppu_issue_mfvscr_gap` capture an `mfvscr` alone takes 40.00
/// cycles an iteration. With an independent `fadd` it takes 41.00, and
/// with an independent `vaddubm` it takes 41.03. The younger op runs
/// [`PPU_MFVSCR_YOUNGER_HOLD_CYCLES`] after the `mfvscr`, and the next
/// `mfvscr` issues 21 cycles after it.
pub const PPU_MFVSCR_VSU_COMPLETE_CYCLES: u64 = 21;

/// The cycles from the cycle a fixed-point store issues to the cycle it
/// completes, for a younger `mfvscr`.
///
/// [CBE-Handbook p:742 s:A.3.2] `mfvscr` stalls at issue until every
/// older instruction completes.
///
/// In the `ppu_issue_store_drain_distance` capture an `mfvscr`, 16 `li`
/// and an `std`, `stw` or `stb` take 48.00 cycles an iteration, 8 more
/// than with no store. With 4 or 8 `li` after the `std` the loop still
/// takes 48.00; with 16 or 32 it costs what the `li` cost.
pub const PPU_MFVSCR_STORE_WAIT_CYCLES: u64 = 12;

/// The cycles from the cycle a vector store issues to the cycle it
/// completes, for a younger `mfvscr`.
///
/// In the `ppu_issue_store_drain_distance` capture an `mfvscr`, 16 `li`
/// and an `stvx` take 56.00 cycles an iteration. The model gives 56.
pub const PPU_MFVSCR_VECTOR_STORE_WAIT_CYCLES: u64 = 21;

/// The cycles from the cycle a floating-point store issues to the cycle
/// it completes, for a younger `mfvscr`.
///
/// In the `ppu_issue_store_drain_distance` capture an `mfvscr`, 16 `li`
/// and an `stfd`, `stfs` or `stfiwx` take 67.00 to 67.23 cycles an
/// iteration; the model gives 67. In the `ppu_issue_fp_store_vscr`
/// capture an `mfvscr`, 8, 16 or 32 `li` and an `stfd` take 59.05, 67.00
/// and 83.33.
pub const PPU_MFVSCR_FP_STORE_WAIT_CYCLES: u64 = 32;

/// The fewest cycles between two stores completing, for a younger
/// `mfvscr`. Stores complete in order.
pub const PPU_MFVSCR_STORE_GAP_CYCLES: u64 = 1;

/// The extra cycles a store takes to complete, for a younger `mfvscr`,
/// when it issues before the older store completes.
///
/// In the `ppu_issue_mfvscr_stores` capture an `mfvscr`, 16 `li` and 1,
/// 2, 3, 4, 5, 6 or 8 `std` to other doublewords take 48.00, 52.00,
/// 53.00, 54.26, 55.00, 56.00 and 58.00 cycles an iteration. The stores
/// issue one a cycle, and the last one completes 15 cycles after it
/// issues, against 12 for a store alone
/// ([`PPU_MFVSCR_STORE_WAIT_CYCLES`]).
pub const PPU_MFVSCR_QUEUED_STORE_CYCLES: u64 = 3;

/// The cycles a store takes to complete, for a younger `mfvscr`, in
/// addition to [`PPU_MFVSCR_QUEUED_STORE_CYCLES`], when it writes a
/// doubleword the older store wrote.
///
/// In the `ppu_issue_mfvscr_stores` capture an `mfvscr`, 16 `li` and two
/// `std` to one doubleword take 56.14 cycles an iteration, and two `stw`
/// to one doubleword 56.06. Two `std` to two doublewords take 52.00.
pub const PPU_MFVSCR_SAME_DOUBLEWORD_CYCLES: u64 = 4;

/// The cycles an `mtvscr` holds its unit, so the next `mtvscr` waits
/// that long.
///
/// [CBE-Handbook p:742 s:A.3.2] Table A-8 gives `mtvscr` no latency.
///
/// In the `ppu_issue_vscr` capture a loop of one `mtvscr` takes 42.54
/// cycles an iteration, and 42.23 with four `addi` after it.
pub const PPU_MTVSCR_HOLD_CYCLES: u64 = 42;

/// The cycles from an `mtvscr` to the first younger vector-scalar
/// instruction that can issue.
///
/// [CBE-Handbook p:742 s:A.3.2] After `mtvscr` issues, every younger vector-scalar instruction stalls until it completes.
///
/// In the `ppu_issue_vscr` capture an `mtvscr` and four independent
/// `vaddubm` take 63.40 cycles an iteration. The last `vaddubm`'s result
/// is ready seven cycles after the first one issues, so the fence takes
/// the other 56.
pub const PPU_MTVSCR_YOUNGER_VSU_CYCLES: u64 = 56;

/// The cycles from a vector reciprocal estimate to a dependent
/// instruction.
///
/// [CBE-Handbook p:742 s:A.3.2] `vrefp vrsqrtefp`: latency 14.
pub const PPU_VMX_ESTIMATE_LATENCY_CYCLES: u64 = 14;

/// The cycles from a vector multiply or multiply-sum to a dependent
/// instruction.
///
/// [CBE-Handbook p:743 s:A.3.2] complex: latency 9.
pub const PPU_VMX_COMPLEX_LATENCY_CYCLES: u64 = 9;

/// The ways of the L1 data cache.
///
/// [CBE-Handbook p:135 s:6.1] 32 KB, 4-way set-associative, 128-byte
/// lines, write-through; a store that misses allocates no line.
pub const PPU_L1D_WAYS: usize = 4;

/// The sets of the L1 data cache: 32 KB over four ways of 128-byte
/// lines.
pub const PPU_L1D_SETS: usize = 64;

/// The ways of the L2 cache.
///
/// [CBE-Handbook p:139 s:6.1.5] 512 KB, 8-way set-associative, 128-byte
/// lines, allocation on store miss.
pub const PPU_L2_WAYS: usize = 8;

/// The sets of the L2 cache: 512 KB over eight ways of 128-byte lines.
pub const PPU_L2_SETS: usize = 512;

/// The bytes of one cache line.
///
/// [CBE-Handbook p:135 s:6.1] 128-byte cache-line size.
pub const PPU_CACHE_LINE_BYTES: u64 = 128;

/// The entries of the store queue, which the threads share. Each entry
/// holds the bytes of one block of [`PPU_STORE_ENTRY_BYTES`].
///
/// [CBE-Handbook p:135 s:6.1] 16-entry store queue.
pub const PPU_STORE_QUEUE_ENTRIES: usize = 16;

/// The entries of a thread's branch history table.
///
/// [CBE-Handbook p:139 s:6.1.4] 4096 x 2-bit entries and 6 bits of
/// global branch history, and a 4-entry link stack, per thread.
pub const PPU_BHT_ENTRIES: usize = 4096;

/// The bits of a thread's global branch history.
pub const PPU_GLOBAL_HISTORY_BITS: u32 = 6;

/// The entries of a thread's link stack.
pub const PPU_LINK_STACK_ENTRIES: usize = 4;

#[cfg(test)]
#[path = "tests/ppu_issue_tests.rs"]
mod tests;
