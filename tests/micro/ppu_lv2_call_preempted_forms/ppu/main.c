/* PPU probe: what a higher-priority wake costs a thread in an LV2 call,
 * and what a wait woken with ECANCELED costs, each timed with mftb.
 *
 * The main thread runs at priority 1001. Loops 1 and 2 run a spinner at
 * priority 900 and a waiter at priority 500. The spinner and main hold
 * the two hardware threads. The waiter waits on a semaphore. In loop 2
 * the spinner posts the semaphore every GAP_TB time-base ticks. The
 * waiter then takes the hardware thread of main, the lowest-priority
 * thread running, counts the wake and waits again. Main spends most of
 * its time inside its call, so most wakes land in a call.
 *
 * Loops 3 and 4 run one helper at main's priority, so each thread keeps
 * its own hardware thread. In each round main sets bit 0 of the go flag
 * and waits for bit 0 of the back flag. The helper waits on go, then
 * answers on back: loop 3 sets it, and loop 4 cancels it, so main's wait
 * returns ECANCELED. A cancel that finds no waiter is made again.
 *
 *   loop
 *    0  sc 147 sys_time_get_timebase_frequency, CALLS a run, alone
 *    1  sc 147, CALLS a run, beside the spinner, no posts
 *    2  sc 147, CALLS a run, beside the spinner, a post every GAP_TB
 *    3  event flag handoff answered by a set, HANDOFFS a run
 *    4  event flag handoff answered by a cancel, HANDOFFS a run
 *
 *   word (big-endian u64)
 *    0, 1   loop 0's fastest and slowest run, in time-base ticks
 *    8 + R  loop 1's run R, in ticks      16 + R  loop 2's run R
 *   24 + R  the waiter's wakes in loop 2's run R
 *   32 + R  the waiter's wakes in loop 1's run R
 *   40, 41  loop 3's fastest and slowest run
 *   42, 43  loop 4's fastest and slowest run
 *   44 + L  loop L's first unexpected return code, or 0 (loops 0 to 2
 *           expect the frequency, loop 4 expects ECANCELED)
 *   52 semaphore create rc   53 waiter create rc   54 spinner create rc
 *   55 waiter join rc        56 spinner join rc    57 semaphore destroy rc
 *   58 the waiter's first nonzero wait rc          59 the waiter's wakes
 *   60 go flag create rc     61 back flag create rc
 *   62 set helper create rc  63 set helper join rc
 *   64 cancel helper create rc                     65 cancel helper join rc
 *   66 back flag destroy rc  67 go flag destroy rc
 *   68 the set helper's first nonzero rc           69 the cancel helper's
 *   70 the set helper's handoffs                   71 the cancel helper's
 *   72 cancels that found no waiter
 *   73 cancels that found more than one waiter
 *   76 the first setup or teardown word whose rc was nonzero, or 0
 *   77 REPS (5)   78 CALLS   79 HANDOFFS   80 GAP_TB
 *   90 0x5050_4C50 when the probe wrote every word
 *
 * Payload: 96 u64 words, 768 bytes.
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
#define SYS_EVENT_FLAG_CANCEL            132
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147

#define FLAG_WAIT_OR_CLEAR 0x12
#define ECANCELED          0x80010013ull
#define TIMEBASE_HZ        79800000ull

/* A handoff wait, in us: far over a handoff, so only a lost wake ends a
 * loop. */
#define HANDOFF_TIMEOUT_US 1000000u
#define HELPER_STACK       0x4000
/* Cancels a round may make before the helper gives up. */
#define CANCEL_TRIES       100000u

#define WORDS    96
#define PAYLOAD  (WORDS * 8)
#define REPS     5
#define CALLS    (1u << 13)
#define HANDOFFS (1u << 12)
#define GAP_TB   2000ull
#define DONE     0x50504C50ull

enum { ANSWER_SET, ANSWER_CANCEL };

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

/* Attribute blocks, each field its own 32-bit word. */
static const unsigned int sem_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0, 0, 0 };                    /* FIFO, private */
static const unsigned int flag_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0x10000, 0, 0 };              /* FIFO, private, one waiter */

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
static volatile unsigned int sem_id, go_id, back_id;

/* Each on its own line: the waiter stores `wakes`, the spinner loads
 * `posting` and `stop`. */
static volatile u64 wakes __attribute__((aligned(128)));
static volatile unsigned int posting __attribute__((aligned(128)));
static volatile unsigned int stop __attribute__((aligned(128)));

static volatile s32 waiter_rc;
static volatile u64 main_bits __attribute__((aligned(8)));
static volatile u64 helper_bits __attribute__((aligned(8)));
static volatile unsigned int cancelled __attribute__((aligned(8)));
static volatile u64 helper_rc[2], handoffs[2];
static volatile u64 empty_cancels, crowded_cancels;

static u64 first_rc[5];
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

/* CALLS calls of the cheapest call as loop `loop`; returns the ticks. */
static u64 __attribute__((noinline)) calls(unsigned loop)
{
    u64 i, rc, bad = 0;
    u64 t0 = timebase();
    for (i = 0; i < CALLS; i++) {
        rc = (u64)cgov_sc(SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0, 0, 0, 0, 0);
        bad = bad ? bad : (rc != TIMEBASE_HZ ? rc | (1ull << 63) : 0);
    }
    t0 = timebase() - t0;
    if (bad && !first_rc[loop])
        first_rc[loop] = bad;
    return t0;
}

/* The waiter: wait on the semaphore, count each wake, until stopped. */
static void waiter_entry(void *arg)
{
    s32 rc;
    (void)arg;
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
static void spinner_entry(void *arg)
{
    (void)arg;
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

static void preempted_loops(void)
{
    unsigned long long waiter = 0, spinner = 0, retval = 0;
    unsigned r;
    u64 sem_rc, wait_rc, spin_rc;

    stop = 0;
    posting = 0;
    wakes = 0;
    sem_rc = step(52, (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0,
                                   0x10000, 0, 0, 0, 0));
    sem_id = id_word;
    if (sem_rc)
        return;
    wait_rc = step(53, (u64)cgov_ppu_thread_create(&waiter,
        make_thread_param(&waiter_param, (const void *)&waiter_entry), 0, 0,
        WAITER_PRIORITY, HELPER_STACK, THREAD_JOINABLE, 0));
    spin_rc = step(54, (u64)cgov_ppu_thread_create(&spinner,
        make_thread_param(&spinner_param, (const void *)&spinner_entry), 0, 0,
        SPINNER_PRIORITY, HELPER_STACK, THREAD_JOINABLE, 0));
    if (wait_rc == 0 && spin_rc == 0) {
        for (r = 0; r < REPS; r++) {
            u64 before = wakes;
            put(8 + r, calls(1));
            put(32 + r, wakes - before);
        }
        for (r = 0; r < REPS; r++) {
            u64 before;
            posting = 1;
            __asm__ volatile ("sync" ::: "memory");
            before = wakes;
            put(16 + r, calls(2));
            put(24 + r, wakes - before);
            posting = 0;
            __asm__ volatile ("sync" ::: "memory");
        }
    }
    stop = 1;
    __asm__ volatile ("sync" ::: "memory");
    if (wait_rc == 0) {
        (void)cgov_sc(SYS_SEMAPHORE_POST, sem_id, 1, 0, 0, 0, 0, 0, 0);
        step(55, (u64)cgov_sc(SYS_PPU_THREAD_JOIN, waiter, EA(&retval), 0, 0, 0, 0, 0, 0));
    }
    if (spin_rc == 0)
        step(56, (u64)cgov_sc(SYS_PPU_THREAD_JOIN, spinner, EA(&retval), 0, 0, 0, 0, 0, 0));
    step(57, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, sem_id, 0, 0, 0, 0, 0, 0, 0));
    put(58, (u64)(unsigned int)waiter_rc);
    put(59, wakes);
}

/* Answer on the back flag: a set, or a cancel made again until it finds
 * the waiter. */
static u64 answer(unsigned how)
{
    unsigned tries;
    u64 rc;
    if (how == ANSWER_SET)
        return (u64)cgov_sc(SYS_EVENT_FLAG_SET, back_id, 1, 0, 0, 0, 0, 0, 0)
               & 0xFFFFFFFFull;
    for (tries = 0; tries < CANCEL_TRIES; tries++) {
        cancelled = 0;
        rc = (u64)cgov_sc(SYS_EVENT_FLAG_CANCEL, back_id, EA(&cancelled), 0, 0, 0, 0, 0, 0)
             & 0xFFFFFFFFull;
        if (rc)
            return rc;
        if (cancelled == 1)
            return 0;
        if (cancelled == 0)
            empty_cancels = empty_cancels + 1;
        else
            crowded_cancels = crowded_cancels + 1;
    }
    return 1;
}

/* A handoff helper: answer each of main's wakes, REPS * HANDOFFS times. */
static void helper_entry(void *arg)
{
    unsigned how = (unsigned)(unsigned long)arg;
    u64 n, rc = 0;
    for (n = 0; n < (u64)REPS * HANDOFFS; n++) {
        rc = (u64)cgov_sc(SYS_EVENT_FLAG_WAIT, go_id, 1, FLAG_WAIT_OR_CLEAR,
                          EA(&helper_bits), HANDOFF_TIMEOUT_US, 0, 0, 0) & 0xFFFFFFFFull;
        if (rc)
            break;
        rc = answer(how);
        if (rc)
            break;
    }
    helper_rc[how] = rc;
    handoffs[how] = n;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

/* HANDOFFS handoffs answered `how`; returns the ticks. A wait with an
 * unexpected return ends the loop. */
static u64 __attribute__((noinline)) handoff_calls(unsigned how)
{
    unsigned loop = 3 + how;
    u64 expect = how == ANSWER_SET ? 0 : ECANCELED;
    u64 i, rc = 0;
    u64 t0 = timebase();
    for (i = 0; i < HANDOFFS; i++) {
        rc = (u64)cgov_sc(SYS_EVENT_FLAG_SET, go_id, 1, 0, 0, 0, 0, 0, 0) & 0xFFFFFFFFull;
        if (rc)
            break;
        rc = (u64)cgov_sc(SYS_EVENT_FLAG_WAIT, back_id, 1, FLAG_WAIT_OR_CLEAR,
                          EA(&main_bits), HANDOFF_TIMEOUT_US, 0, 0, 0) & 0xFFFFFFFFull;
        if (rc != expect)
            break;
        rc = 0;
    }
    t0 = timebase() - t0;
    if (rc && !first_rc[loop])
        first_rc[loop] = rc;
    return t0;
}

static void handoff_loops(unsigned how, unsigned create, unsigned join)
{
    unsigned long long tid = 0, retval = 0;
    unsigned loop = 3 + how;
    u64 lo = ~0ull, hi = 0;
    unsigned r;
    if (step(create, (u64)cgov_ppu_thread_create(&tid,
            make_thread_param(&helper_param[how], (const void *)&helper_entry), how, 0,
            MAIN_PRIORITY, HELPER_STACK, THREAD_JOINABLE, 0)) != 0)
        return;
    for (r = 0; r < REPS && !first_rc[loop]; r++) {
        u64 t = handoff_calls(how);
        lo = t < lo ? t : lo;
        hi = t > hi ? t : hi;
    }
    put(40 + 2 * how, lo);
    put(41 + 2 * how, hi);
    step(join, (u64)cgov_sc(SYS_PPU_THREAD_JOIN, tid, EA(&retval), 0, 0, 0, 0, 0, 0));
}

static void flag_loops(void)
{
    u64 go_rc, back_rc;
    go_rc = step(60, (u64)cgov_sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0,
                                  0, 0, 0, 0, 0));
    go_id = id_word;
    back_rc = step(61, (u64)cgov_sc(SYS_EVENT_FLAG_CREATE, EA(&id_word), EA(flag_attr), 0,
                                    0, 0, 0, 0, 0));
    back_id = id_word;
    if (go_rc == 0 && back_rc == 0) {
        handoff_loops(ANSWER_SET, 62, 63);
        handoff_loops(ANSWER_CANCEL, 64, 65);
    }
    if (back_rc == 0)
        step(66, (u64)cgov_sc(SYS_EVENT_FLAG_DESTROY, back_id, 0, 0, 0, 0, 0, 0, 0));
    if (go_rc == 0)
        step(67, (u64)cgov_sc(SYS_EVENT_FLAG_DESTROY, go_id, 0, 0, 0, 0, 0, 0, 0));
}

int main(void)
{
    unsigned i, r;
    u64 lo = ~0ull, hi = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (r = 0; r < REPS; r++) {
        u64 t = calls(0);
        lo = t < lo ? t : lo;
        hi = t > hi ? t : hi;
    }
    put(0, lo);
    put(1, hi);

    preempted_loops();
    flag_loops();

    for (i = 0; i < 5; i++)
        put(44 + i, first_rc[i]);
    put(68, helper_rc[ANSWER_SET]);
    put(69, helper_rc[ANSWER_CANCEL]);
    put(70, handoffs[ANSWER_SET]);
    put(71, handoffs[ANSWER_CANCEL]);
    put(72, empty_cancels);
    put(73, crowded_cancels);
    put(76, first_failed);
    put(77, REPS);
    put(78, CALLS);
    put(79, HANDOFFS);
    put(80, GAP_TB);
    put(90, DONE);
    write_frame();
    return 0;
}
