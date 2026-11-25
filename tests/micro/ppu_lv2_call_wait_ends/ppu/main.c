/* PPU probe: what a wait that ends by its timeout costs, what an event
 * queue receive that a forced destroy ends costs, and what a
 * higher-priority wake costs a thread that runs in user mode, each timed
 * with mftb.
 *
 * The timed waits: each of ROUNDS rounds makes the calls below in order,
 * each between two mftb reads, on objects nothing posts, sets or sends
 * to. Each wait returns ETIMEDOUT. The time-base frequency call is the
 * baseline, as in the ppu_lv2_call_sizes probe.
 *
 *   call
 *    0         sc 147 sys_time_get_timebase_frequency
 *    1 + T     sc 92  sys_semaphore_wait(sem, TIMEOUT_US[T])
 *    8 + T     sc 85  sys_event_flag_wait(flag, 1, OR | CLEAR, &bits,
 *                                         TIMEOUT_US[T])
 *   15 + T     sc 130 sys_event_queue_receive(queue, &event, TIMEOUT_US[T])
 *   22         sc 128 sys_event_queue_create(&id, attr, 0, 64)
 *   23         sc 129 sys_event_queue_destroy(id, 0)
 *   24         sc 128, again
 *   25         sc 129 sys_event_queue_destroy(id, force), with no receiver
 *
 *   TIMEOUT_US = { 1, 2, 4, 6, 10, 100, 1000 }
 *
 * The handoffs: one helper at main's priority, so each thread keeps its
 * own hardware thread. Each round main creates an event queue and sets
 * bit 0 of the go flag. The helper waits on go, then spins DELAY_TB
 * ticks, so main's next call blocks first.
 *
 *   loop
 *    C  main waits on bit 0 of the back flag, then destroys the queue;
 *       the helper sets the back flag
 *    D  main receives on the queue; the helper destroys it with the force
 *       flag, so the receive returns ECANCELED
 *
 * The user-mode loops: main runs at priority 1001 beside a spinner at
 * priority 900 and a waiter at priority 500, as in the
 * ppu_lv2_call_preempted_forms probe. Main counts down USER_ITERS in a
 * two-instruction loop and makes no call. In loop U2 the spinner posts
 * the waiter's semaphore every GAP_TB ticks; in loop U1 it posts nothing.
 * Each wake of the waiter takes main's hardware thread in user mode.
 *
 *   word (big-endian u64)
 *    0, 1   the frequency loop's fastest and slowest run, CALLS0 calls
 *    2, 3   loop C's fastest and slowest run, HANDOFFS rounds
 *    4, 5   loop D's fastest and slowest run
 *    8 + R  loop U1's run R, in ticks      16 + R  loop U2's run R
 *   24 + R  the waiter's wakes in loop U2's run R
 *   32 + R  the waiter's wakes in loop U1's run R
 *   40 + C  timed call C's first unexpected return code, or 0; call 0
 *           expects the frequency, the waits ETIMEDOUT
 *   56 sem create rc   57 flag create rc   58 queue create rc
 *   59 sem destroy rc  60 flag destroy rc  61 queue destroy rc
 *   64 go flag create rc   65 back flag create rc
 *   66 helper C create rc  67 helper C join rc
 *   68 helper D create rc  69 helper D join rc
 *   70 back flag destroy rc   71 go flag destroy rc
 *   72 helper C's first nonzero rc      73 helper D's
 *   74 helper C's handoffs              75 helper D's
 *   76 main's first unexpected rc in loop C   77 in loop D
 *   78 loop D's receives that found the queue gone (ESRCH)
 *   88 + 4H  handoff H's (0 = C, 1 = D) sum of the ticks from the
 *            helper's mftb before its answer to main's mftb after its
 *            wait returns
 *   89 + 4H  the least of those ticks
 *   90 + 4H  the sum of the helper's answer call's ticks
 *   91 + 4H  the least of those ticks
 *   80 user sem create rc  81 waiter create rc  82 spinner create rc
 *   83 waiter join rc      84 spinner join rc   85 user sem destroy rc
 *   86 the waiter's first nonzero wait rc       87 the waiter's wakes
 *   96 the first setup or teardown word whose rc was nonzero, or 0
 *   97 REPS (5)  98 CALLS0  99 HANDOFFS  100 GAP_TB  101 ROUNDS
 *  102 TIMED (26)  103 DELAY_TB  104 USER_ITERS
 *  127 0x5050_4C54 when the probe wrote every word
 *   from byte 1024: big-endian u32 samples, timed call C round R at
 *   1024 + 4 * (C * ROUNDS + R), in time-base ticks
 *
 * Payload: 2688 bytes.
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

#define MAIN_PRIORITY    1001
#define SPINNER_PRIORITY 900
#define WAITER_PRIORITY  500

SYS_PROCESS_PARAM(MAIN_PRIORITY, 0x10000)

#define SYS_PPU_THREAD_EXIT              41
#define SYS_PPU_THREAD_JOIN              44
#define SYS_EVENT_FLAG_CREATE            82
#define SYS_EVENT_FLAG_DESTROY           83
#define SYS_EVENT_FLAG_WAIT              85
#define SYS_EVENT_FLAG_SET               87
#define SYS_SEMAPHORE_CREATE             90
#define SYS_SEMAPHORE_DESTROY            91
#define SYS_SEMAPHORE_WAIT               92
#define SYS_SEMAPHORE_POST               94
#define SYS_EVENT_QUEUE_CREATE           128
#define SYS_EVENT_QUEUE_DESTROY          129
#define SYS_EVENT_QUEUE_RECEIVE          130
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147

#define FLAG_WAIT_OR_CLEAR 0x12
#define QUEUE_DESTROY_FORCE 1
#define ETIMEDOUT          0x8001000Bull
#define ECANCELED          0x80010013ull
#define ESRCH              0x80010005ull
#define TIMEBASE_HZ        79800000ull

/* A handoff wait, in us: far over a handoff, so only a lost wake ends a
 * loop. */
#define HANDOFF_TIMEOUT_US 1000000u
#define HELPER_STACK       0x4000

#define WORDS      128
#define HEADER     (WORDS * 8)
#define ROUNDS     16
#define TIMEOUTS   7
#define TIMED      (1 + 3 * TIMEOUTS + 4)
#define QUEUE_CALL (1 + 3 * TIMEOUTS)
#define PAYLOAD    (HEADER + 4 * TIMED * ROUNDS)
#define REPS       5
#define CALLS0     (1u << 13)
#define HANDOFFS   (1u << 10)
#define GAP_TB     2000ull
#define DELAY_TB   100ull
#define USER_ITERS (1ul << 21)
#define DONE       0x50504C54ull

enum { ANSWER_SET, ANSWER_DESTROY };

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static const u64 TIMEOUT_US[TIMEOUTS] = { 1, 2, 4, 6, 10, 100, 1000 };

/* Attribute blocks, each field its own 32-bit word. */
static const unsigned int sem_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0, 0, 0 };                    /* FIFO, private */
static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0x10000, 0, 0 };              /* FIFO, private, one waiter */
static const unsigned int queue_attr[4] __attribute__((aligned(8))) =
    { 1, 1, 0, 0 };                                     /* FIFO, PPU queue */

struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct cg_thread_param {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static unsigned long make_thread_param(struct cg_thread_param *p, const void *fn)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    unsigned long tls_reg;
    __asm__ volatile ("mr %0, 13" : "=r"(tls_reg));
    p->opd_code = (unsigned int)desc->code;
    p->opd_toc = (unsigned int)desc->toc;
    p->entry_opd_ptr = (unsigned int)(unsigned long)&p->opd_code;
    p->tls = (unsigned int)tls_reg;
    return (unsigned long)p;
}

static struct cg_thread_param waiter_param, spinner_param, helper_param[2]
    __attribute__((aligned(8)));

static volatile unsigned int id_word __attribute__((aligned(8)));
static volatile unsigned int sem_id, go_id, back_id, queue_id;

/* Each on its own line: the waiter stores `wakes`, the spinner loads
 * `posting` and `stop`. */
static volatile u64 wakes __attribute__((aligned(128)));
static volatile unsigned int posting __attribute__((aligned(128)));
static volatile unsigned int stop __attribute__((aligned(128)));

static volatile s32 waiter_rc;
static volatile u64 main_bits __attribute__((aligned(8)));
static volatile u64 helper_bits __attribute__((aligned(8)));
static volatile u64 event[4] __attribute__((aligned(16)));
static volatile u64 helper_rc[2], handoffs[2];
/* The helper's mftb before its answer; main reads it after its wait. */
static volatile u64 answered_at __attribute__((aligned(128)));
static u64 wake_sum[2], wake_min[2] = { ~0ull, ~0ull };
static volatile u64 answer_sum[2], answer_min[2] = { ~0ull, ~0ull };

static u64 arg[8];
static u64 first_rc[TIMED];
static u64 main_rc[2];
static u64 gone;
static u64 first_failed;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static inline u64 timebase(void)
{
    u64 tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
}

static void put(unsigned slot, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        out[slot * 8 + i] = (unsigned char)(v >> (56 - 8 * i));
}

static void put_sample(unsigned call, unsigned round, u64 ticks)
{
    unsigned at = HEADER + 4 * (call * ROUNDS + round);
    unsigned i;
    if (ticks > 0xFFFFFFFFull)
        ticks = 0xFFFFFFFFull;
    for (i = 0; i < 4; i++)
        out[at + i] = (unsigned char)(ticks >> (24 - 8 * i));
}

/* Record a setup or teardown step's rc at `slot`; keep the first that
 * failed. */
static u64 step(unsigned slot, u64 rc)
{
    put(slot, rc & 0xFFFFFFFFull);
    if ((rc & 0xFFFFFFFFull) && !first_failed)
        first_failed = slot;
    return rc & 0xFFFFFFFFull;
}

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = PAYLOAD;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)out, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)out, len);
}

/* ---- the timed waits ---- */

/* One call of syscall `num` with arg[0..7], between two mftb reads.
 * Records the ticks as call `call`'s sample of round `round`, and the
 * first return other than `expect`. */
static void __attribute__((noinline)) one(unsigned call, unsigned round, u64 num, u64 expect)
{
    u64 rc;
    u64 t0 = timebase();
    rc = (u64)cgov_sc(num, arg[0], arg[1], arg[2], arg[3], arg[4], arg[5], arg[6], arg[7]);
    t0 = timebase() - t0;
    put_sample(call, round, t0);
    if ((rc & 0xFFFFFFFFull) != expect && !first_rc[call])
        first_rc[call] = rc | (1ull << 63);
}

static void set_args(u64 a, u64 b, u64 c, u64 d, u64 e)
{
    unsigned i;
    arg[0] = a;
    arg[1] = b;
    arg[2] = c;
    arg[3] = d;
    arg[4] = e;
    for (i = 5; i < 8; i++)
        arg[i] = 0;
}

/* Timed calls `call` and `call + 1` of round `round`: create an event
 * queue, then destroy it with `mode`. */
static void queue_pair(unsigned round, unsigned call, u64 mode)
{
    id_word = 0;
    set_args(EA(&id_word), EA(queue_attr), 0, 64, 0);
    one(call, round, SYS_EVENT_QUEUE_CREATE, 0);
    set_args(id_word, mode, 0, 0, 0);
    one(call + 1, round, SYS_EVENT_QUEUE_DESTROY, 0);
}

static void timed_waits(void)
{
    unsigned r, t;
    u64 sem_rc, flag_rc, queue_rc;
    unsigned int sem, flag, queue;

    sem_rc = step(56, (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0,
                                   0x10000, 0, 0, 0, 0));
    sem = id_word;
    flag_rc = step(57, (u64)cgov_sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0,
                                    0, 0, 0, 0, 0));
    flag = id_word;
    queue_rc = step(58, (u64)cgov_sc(SYS_EVENT_QUEUE_CREATE, EA(&id_word), EA(queue_attr), 0,
                                     64, 0, 0, 0, 0));
    queue = id_word;
    if (sem_rc == 0 && flag_rc == 0 && queue_rc == 0) {
        for (r = 0; r < ROUNDS; r++) {
            set_args(0, 0, 0, 0, 0);
            one(0, r, SYS_TIME_GET_TIMEBASE_FREQUENCY, TIMEBASE_HZ & 0xFFFFFFFFull);
            for (t = 0; t < TIMEOUTS; t++) {
                set_args(sem, TIMEOUT_US[t], 0, 0, 0);
                one(1 + t, r, SYS_SEMAPHORE_WAIT, ETIMEDOUT);
                set_args(flag, 1, FLAG_WAIT_OR_CLEAR, EA(&main_bits), TIMEOUT_US[t]);
                one(1 + TIMEOUTS + t, r, SYS_EVENT_FLAG_WAIT, ETIMEDOUT);
                set_args(queue, EA(event), TIMEOUT_US[t], 0, 0);
                one(1 + 2 * TIMEOUTS + t, r, SYS_EVENT_QUEUE_RECEIVE, ETIMEDOUT);
            }
            queue_pair(r, QUEUE_CALL, 0);
            queue_pair(r, QUEUE_CALL + 2, QUEUE_DESTROY_FORCE);
        }
    }
    if (sem_rc == 0)
        step(59, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, sem, 0, 0, 0, 0, 0, 0, 0));
    if (flag_rc == 0)
        step(60, (u64)cgov_sc(SYS_EVENT_FLAG_DESTROY, flag, 0, 0, 0, 0, 0, 0, 0));
    if (queue_rc == 0)
        step(61, (u64)cgov_sc(SYS_EVENT_QUEUE_DESTROY, queue, 0, 0, 0, 0, 0, 0, 0));
}

/* ---- the handoffs ---- */

/* A handoff helper: answer each of main's rounds, REPS * HANDOFFS times. */
static void helper_entry(void *arg0)
{
    unsigned how = (unsigned)(unsigned long)arg0;
    u64 n, rc = 0;
    for (n = 0; n < (u64)REPS * HANDOFFS; n++) {
        u64 t0;
        rc = (u64)cgov_sc(SYS_EVENT_FLAG_WAIT, go_id, 1, FLAG_WAIT_OR_CLEAR,
                          EA(&helper_bits), HANDOFF_TIMEOUT_US, 0, 0, 0) & 0xFFFFFFFFull;
        if (rc)
            break;
        t0 = timebase();
        while (timebase() - t0 < DELAY_TB) {
        }
        t0 = timebase();
        answered_at = t0;
        if (how == ANSWER_SET)
            rc = (u64)cgov_sc(SYS_EVENT_FLAG_SET, back_id, 1, 0, 0, 0, 0, 0, 0);
        else
            rc = (u64)cgov_sc(SYS_EVENT_QUEUE_DESTROY, queue_id, QUEUE_DESTROY_FORCE,
                              0, 0, 0, 0, 0, 0);
        t0 = timebase() - t0;
        answer_sum[how] = answer_sum[how] + t0;
        if (t0 < answer_min[how])
            answer_min[how] = t0;
        rc &= 0xFFFFFFFFull;
        if (rc)
            break;
    }
    helper_rc[how] = rc;
    handoffs[how] = n;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

/* Add the ticks from the helper's answer to now to handoff `how`'s
 * sums. */
static void note_wake(unsigned how)
{
    u64 t = timebase() - answered_at;
    wake_sum[how] += t;
    if (t < wake_min[how])
        wake_min[how] = t;
}

/* One round of loop C (`how` ANSWER_SET) or D; returns the first
 * unexpected rc, or 0. */
static u64 round_of(unsigned how)
{
    u64 rc;
    rc = (u64)cgov_sc(SYS_EVENT_QUEUE_CREATE, EA(&id_word), EA(queue_attr), 0, 64,
                      0, 0, 0, 0) & 0xFFFFFFFFull;
    if (rc)
        return rc;
    queue_id = id_word;
    rc = (u64)cgov_sc(SYS_EVENT_FLAG_SET, go_id, 1, 0, 0, 0, 0, 0, 0) & 0xFFFFFFFFull;
    if (rc)
        return rc;
    if (how == ANSWER_SET) {
        rc = (u64)cgov_sc(SYS_EVENT_FLAG_WAIT, back_id, 1, FLAG_WAIT_OR_CLEAR,
                          EA(&main_bits), HANDOFF_TIMEOUT_US, 0, 0, 0) & 0xFFFFFFFFull;
        note_wake(how);
        if (rc)
            return rc;
        return (u64)cgov_sc(SYS_EVENT_QUEUE_DESTROY, queue_id, 0, 0, 0, 0, 0, 0, 0)
               & 0xFFFFFFFFull;
    }
    rc = (u64)cgov_sc(SYS_EVENT_QUEUE_RECEIVE, queue_id, EA(event), HANDOFF_TIMEOUT_US,
                      0, 0, 0, 0, 0) & 0xFFFFFFFFull;
    if (rc != ESRCH)
        note_wake(how);
    if (rc == ESRCH) {
        gone++;
        return 0;
    }
    return rc == ECANCELED ? 0 : rc;
}

/* HANDOFFS rounds answered `how`; returns the ticks. */
static u64 __attribute__((noinline)) handoff_calls(unsigned how)
{
    u64 i, rc = 0;
    u64 t0 = timebase();
    for (i = 0; i < HANDOFFS; i++) {
        rc = round_of(how);
        if (rc)
            break;
    }
    t0 = timebase() - t0;
    if (rc && !main_rc[how])
        main_rc[how] = rc;
    return t0;
}

static void handoff_loops(unsigned how, unsigned create, unsigned join)
{
    unsigned long long tid = 0, retval = 0;
    u64 lo = ~0ull, hi = 0;
    unsigned r;
    if (step(create, (u64)cgov_ppu_thread_create(&tid,
            make_thread_param(&helper_param[how], (const void *)&helper_entry), how, 0,
            MAIN_PRIORITY, HELPER_STACK, THREAD_JOINABLE, 0)) != 0)
        return;
    for (r = 0; r < REPS && !main_rc[how]; r++) {
        u64 t = handoff_calls(how);
        lo = t < lo ? t : lo;
        hi = t > hi ? t : hi;
    }
    put(2 + 2 * how, lo);
    put(3 + 2 * how, hi);
    step(join, (u64)cgov_sc(SYS_PPU_THREAD_JOIN, tid, EA(&retval), 0, 0, 0, 0, 0, 0));
}

static void flag_loops(void)
{
    u64 go_rc, back_rc;
    go_rc = step(64, (u64)cgov_sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0,
                                  0, 0, 0, 0, 0));
    go_id = id_word;
    back_rc = step(65, (u64)cgov_sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0,
                                    0, 0, 0, 0, 0));
    back_id = id_word;
    if (go_rc == 0 && back_rc == 0) {
        handoff_loops(ANSWER_SET, 66, 67);
        handoff_loops(ANSWER_DESTROY, 68, 69);
    }
    if (back_rc == 0)
        step(70, (u64)cgov_sc(SYS_EVENT_FLAG_DESTROY, back_id, 0, 0, 0, 0, 0, 0, 0));
    if (go_rc == 0)
        step(71, (u64)cgov_sc(SYS_EVENT_FLAG_DESTROY, go_id, 0, 0, 0, 0, 0, 0, 0));
}

/* ---- the user-mode loops ---- */

/* USER_ITERS turns of a two-instruction loop, no call; returns the
 * ticks. */
static u64 __attribute__((noinline)) user_loop(void)
{
    unsigned long n = USER_ITERS;
    unsigned long x = 0;
    u64 t0 = timebase();
    __asm__ volatile (
        "mtctr %1\n"
        "1: addi %0, %0, 1\n"
        "bdnz 1b\n"
        : "+r"(x) : "r"(n) : "ctr");
    return timebase() - t0;
}

/* The waiter: wait on the semaphore, count each wake, until stopped. */
static void waiter_entry(void *arg0)
{
    s32 rc;
    (void)arg0;
    for (;;) {
        rc = syscall2_s32(SYS_SEMAPHORE_WAIT, sem_id, 0);
        if (stop)
            break;
        if (rc) {
            waiter_rc = rc;
            break;
        }
        wakes = wakes + 1;
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

/* The spinner: every GAP_TB ticks, post the semaphore while `posting`. */
static void spinner_entry(void *arg0)
{
    (void)arg0;
    while (!stop) {
        u64 t0 = timebase();
        if (posting)
            (void)cgov_sc(SYS_SEMAPHORE_POST, sem_id, 1, 0, 0, 0, 0, 0, 0);
        while (timebase() - t0 < GAP_TB && !stop) {
        }
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

static void user_loops(void)
{
    unsigned long long waiter = 0, spinner = 0, retval = 0;
    unsigned r;
    u64 sem_rc, wait_rc, spin_rc;

    stop = 0;
    posting = 0;
    wakes = 0;
    sem_rc = step(80, (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0,
                                   0x10000, 0, 0, 0, 0));
    sem_id = id_word;
    if (sem_rc)
        return;
    wait_rc = step(81, (u64)cgov_ppu_thread_create(&waiter,
        make_thread_param(&waiter_param, (const void *)&waiter_entry), 0, 0,
        WAITER_PRIORITY, HELPER_STACK, THREAD_JOINABLE, 0));
    spin_rc = step(82, (u64)cgov_ppu_thread_create(&spinner,
        make_thread_param(&spinner_param, (const void *)&spinner_entry), 0, 0,
        SPINNER_PRIORITY, HELPER_STACK, THREAD_JOINABLE, 0));
    if (wait_rc == 0 && spin_rc == 0) {
        for (r = 0; r < REPS; r++) {
            u64 before = wakes;
            put(8 + r, user_loop());
            put(32 + r, wakes - before);
        }
        for (r = 0; r < REPS; r++) {
            u64 before;
            posting = 1;
            __asm__ volatile ("sync" ::: "memory");
            before = wakes;
            put(16 + r, user_loop());
            put(24 + r, wakes - before);
            posting = 0;
            __asm__ volatile ("sync" ::: "memory");
        }
    }
    stop = 1;
    __asm__ volatile ("sync" ::: "memory");
    if (wait_rc == 0) {
        (void)cgov_sc(SYS_SEMAPHORE_POST, sem_id, 1, 0, 0, 0, 0, 0, 0);
        step(83, (u64)cgov_sc(SYS_PPU_THREAD_JOIN, waiter, EA(&retval), 0, 0, 0, 0, 0, 0));
    }
    if (spin_rc == 0)
        step(84, (u64)cgov_sc(SYS_PPU_THREAD_JOIN, spinner, EA(&retval), 0, 0, 0, 0, 0, 0));
    step(85, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, sem_id, 0, 0, 0, 0, 0, 0, 0));
    put(86, (u64)(unsigned int)waiter_rc);
    put(87, wakes);
}

/* CALLS0 calls of the cheapest call; returns the ticks. */
static u64 __attribute__((noinline)) frequency_calls(void)
{
    u64 i;
    u64 t0 = timebase();
    for (i = 0; i < CALLS0; i++)
        (void)cgov_sc(SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0, 0, 0, 0, 0);
    return timebase() - t0;
}

int main(void)
{
    unsigned i, r;
    u64 lo = ~0ull, hi = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (r = 0; r < REPS; r++) {
        u64 t = frequency_calls();
        lo = t < lo ? t : lo;
        hi = t > hi ? t : hi;
    }
    put(0, lo);
    put(1, hi);

    timed_waits();
    flag_loops();
    user_loops();

    for (i = 0; i < TIMED; i++)
        put(40 + i, first_rc[i]);
    put(72, helper_rc[ANSWER_SET]);
    put(73, helper_rc[ANSWER_DESTROY]);
    put(74, handoffs[ANSWER_SET]);
    put(75, handoffs[ANSWER_DESTROY]);
    put(76, main_rc[ANSWER_SET]);
    put(77, main_rc[ANSWER_DESTROY]);
    put(78, gone);
    for (i = 0; i < 2; i++) {
        put(88 + 4 * i, wake_sum[i]);
        put(89 + 4 * i, wake_min[i]);
        put(90 + 4 * i, answer_sum[i]);
        put(91 + 4 * i, answer_min[i]);
    }
    put(96, first_failed);
    put(97, REPS);
    put(98, CALLS0);
    put(99, HANDOFFS);
    put(100, GAP_TB);
    put(101, ROUNDS);
    put(102, TIMED);
    put(103, DELAY_TB);
    put(104, USER_ITERS);
    put(127, DONE);
    write_frame();
    return 0;
}
