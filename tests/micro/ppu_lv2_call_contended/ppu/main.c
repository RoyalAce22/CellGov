/* PPU probe: the round trip of sys_ppu_thread_yield with other threads
 * of the caller's priority runnable, and of a semaphore handoff whose
 * wait blocks and whose post wakes the waiter, each timed with mftb.
 *
 * Each loop makes 1 << 12 calls or handoffs, five runs a loop. The probe
 * creates two semaphores (count 0, maximum 1) and three joinable PPU
 * threads, all at main's priority, and destroys or joins all five before
 * it writes its result.
 *
 * A yield gives the core to a runnable thread of equal or higher
 * priority. The PPU runs two hardware threads, so a yield finds another
 * thread waiting only when three threads of main's priority are
 * runnable. Each yielding helper calls sys_ppu_thread_yield until main
 * sets its stop word.
 *
 *   loop
 *    0  sc 147 sys_time_get_timebase_frequency, the baseline
 *    1  sc 43 sys_ppu_thread_yield, no other thread runnable
 *    2  sc 43 sys_ppu_thread_yield, one yielding helper
 *    3  sc 43 sys_ppu_thread_yield, two yielding helpers
 *    4  a handoff: main posts the first semaphore and waits on the
 *       second; the handoff helper waits on the first and posts the
 *       second. Each main wait blocks until the helper posts.
 *
 *   word
 *   2L, 2L+1   loop L's fastest and slowest run, in time-base ticks
 *   16 + L     loop L's first nonzero return code, or 0
 *   24 + L     loop L's last return code, as the low 32 bits of r3
 *   32  first semaphore create rc     33  second semaphore create rc
 *   34  first yielding helper create rc
 *   35  second yielding helper create rc
 *   36  first yielding helper join rc
 *   37  second yielding helper join rc
 *   38  handoff helper create rc      39  handoff helper join rc
 *   40  the handoff helper's first nonzero wait rc, or 0
 *   41  the handoff helper's first nonzero post rc, or 0
 *   42  second semaphore destroy rc   43  first semaphore destroy rc
 *   48  the yields the first helper made
 *   49  the yields the second helper made
 *   50  the handoffs the handoff helper completed
 *   57  the first setup or teardown step whose rc was nonzero: its word
 *       above, or 0 when every step returned 0
 *   59  runs a loop (5)
 *   60  0x5050_4C59 when the probe wrote every word
 *
 * A loop whose setup failed is skipped and its words stay 0.
 *
 * Payload: 64 u64 words, 512 bytes.
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT              41
#define SYS_PPU_THREAD_YIELD             43
#define SYS_PPU_THREAD_JOIN              44
#define SYS_SEMAPHORE_CREATE             90
#define SYS_SEMAPHORE_DESTROY            91
#define SYS_SEMAPHORE_WAIT               92
#define SYS_SEMAPHORE_POST               94
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147

/* A handoff wait, in us: far over a handoff, so only a lost post ends a
 * loop. */
#define HANDOFF_TIMEOUT_US 1000000u
#define HELPER_PRIO   1001
#define HELPER_STACK  0x4000

#define WORDS   64
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define CALLS   (1u << 12)
#define LOOPS   5
#define HANDOFF_LOOP 4
#define DONE    0x50504C59ull

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

/* Attribute block, each field its own 32-bit word. */
static const unsigned int sem_attr[8] __attribute__((aligned(8))) =
    { 1, 0x200, 0, 0, 0, 0, 0, 0 };                    /* FIFO, private */

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

/* Repack the toolchain's 24-byte ELFv1 descriptor into the kernel's
 * { u32 code; u32 toc } form, as lv2_destroy_parked does. */
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

static struct cg_thread_param yield_param[2] __attribute__((aligned(8)));
static struct cg_thread_param handoff_param __attribute__((aligned(8)));

static volatile unsigned int id_word __attribute__((aligned(8)));
static volatile unsigned int go_sem, back_sem;
static volatile unsigned int stop[2];
static volatile u64 yields[2];
static volatile u64 handoffs;
static volatile s32 helper_wait_rc, helper_post_rc;

static u64 first_rc[LOOPS];
static u64 last_rc[LOOPS];
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

/* CALLS calls of syscall `num`; returns the time-base ticks they took.
 * Keeps the first nonzero and the last return code of the loop. */
static u64 __attribute__((noinline)) calls(unsigned loop, u64 num)
{
    u64 i, rc = 0, bad = 0;
    u64 t0 = timebase();
    for (i = 0; i < CALLS; i++) {
        rc = (u64)cgov_sc(num, 0, 0, 0, 0, 0, 0, 0, 0);
        bad = bad ? bad : (rc & 0xFFFFFFFFull);
    }
    t0 = timebase() - t0;
    if (bad && !first_rc[loop])
        first_rc[loop] = bad;
    last_rc[loop] = rc & 0xFFFFFFFFull;
    return t0;
}

/* CALLS handoffs; returns the time-base ticks they took. A failed post
 * or wait ends the loop, so a lost handoff never waits out more than one
 * timeout. */
static u64 __attribute__((noinline)) handoff_calls(void)
{
    u64 i, rc = 0;
    u64 t0 = timebase();
    for (i = 0; i < CALLS; i++) {
        rc = (u64)cgov_sc(SYS_SEMAPHORE_POST, go_sem, 1, 0, 0, 0, 0, 0, 0) & 0xFFFFFFFFull;
        if (rc)
            break;
        rc = (u64)cgov_sc(SYS_SEMAPHORE_WAIT, back_sem, HANDOFF_TIMEOUT_US, 0, 0, 0, 0, 0, 0)
             & 0xFFFFFFFFull;
        if (rc)
            break;
    }
    t0 = timebase() - t0;
    if (rc && !first_rc[HANDOFF_LOOP])
        first_rc[HANDOFF_LOOP] = rc;
    last_rc[HANDOFF_LOOP] = rc;
    return t0;
}

/* Keep loop `loop`'s fastest and slowest of REPS runs. */
static void keep(unsigned loop, u64 lo, u64 hi)
{
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

static void timed(unsigned loop, u64 num)
{
    u64 lo = ~0ull, hi = 0;
    unsigned r;
    for (r = 0; r < REPS; r++) {
        u64 t = calls(loop, num);
        lo = t < lo ? t : lo;
        hi = t > hi ? t : hi;
    }
    keep(loop, lo, hi);
}

static void timed_handoffs(void)
{
    u64 lo = ~0ull, hi = 0;
    unsigned r;
    for (r = 0; r < REPS && !first_rc[HANDOFF_LOOP]; r++) {
        u64 t = handoff_calls();
        lo = t < lo ? t : lo;
        hi = t > hi ? t : hi;
    }
    keep(HANDOFF_LOOP, lo, hi);
}

/* A yielding helper: yield until main sets its stop word. `arg` is the
 * helper's index. */
static void yield_entry(void *arg)
{
    unsigned k = (unsigned)(unsigned long)arg;
    u64 n = 0;
    while (!stop[k]) {
        (void)cgov_sc(SYS_PPU_THREAD_YIELD, 0, 0, 0, 0, 0, 0, 0, 0);
        n++;
    }
    yields[k] = n;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

/* The handoff helper: answer each post of the first semaphore with a post
 * of the second, REPS * CALLS times, and stop at the first failure. */
static void handoff_entry(void *arg)
{
    u64 n;
    s32 rc;
    (void)arg;
    for (n = 0; n < (u64)REPS * CALLS; n++) {
        rc = syscall2_s32(SYS_SEMAPHORE_WAIT, go_sem, HANDOFF_TIMEOUT_US);
        if (rc) {
            helper_wait_rc = rc;
            break;
        }
        rc = syscall2_s32(SYS_SEMAPHORE_POST, back_sem, 1);
        if (rc) {
            helper_post_rc = rc;
            break;
        }
    }
    handoffs = n;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0);
    for (;;) { }
}

/* Start yielding helper `k`; returns its create rc. */
static u64 start_yielder(unsigned k, unsigned long long *tid)
{
    return step(34 + k, (u64)cgov_ppu_thread_create(tid,
        make_thread_param(&yield_param[k], (const void *)&yield_entry), k, 0,
        HELPER_PRIO, HELPER_STACK, THREAD_JOINABLE, 0));
}

int main(void)
{
    unsigned i;
    u64 go_rc, back_rc, y0_rc, y1_rc = 1;
    unsigned long long tid[3] = { 0, 0, 0 }, retval = 0;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    timed(0, SYS_TIME_GET_TIMEBASE_FREQUENCY);
    timed(1, SYS_PPU_THREAD_YIELD);

    y0_rc = start_yielder(0, &tid[0]);
    if (y0_rc == 0) {
        timed(2, SYS_PPU_THREAD_YIELD);
        y1_rc = start_yielder(1, &tid[1]);
        if (y1_rc == 0)
            timed(3, SYS_PPU_THREAD_YIELD);
        stop[0] = 1;
        stop[1] = 1;
        __asm__ volatile ("sync" ::: "memory");
        step(36, (u64)cgov_sc(SYS_PPU_THREAD_JOIN, tid[0], EA(&retval), 0, 0, 0, 0, 0, 0));
        if (y1_rc == 0)
            step(37, (u64)cgov_sc(SYS_PPU_THREAD_JOIN, tid[1], EA(&retval), 0, 0, 0, 0, 0, 0));
    }

    go_rc = step(32, (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0, 1, 0, 0, 0, 0));
    go_sem = id_word;
    back_rc = step(33, (u64)cgov_sc(SYS_SEMAPHORE_CREATE, EA(&id_word), EA(sem_attr), 0, 1, 0, 0, 0, 0));
    back_sem = id_word;

    if (go_rc == 0 && back_rc == 0) {
        s32 rc = cgov_ppu_thread_create(&tid[2],
            make_thread_param(&handoff_param, (const void *)&handoff_entry), 0, 0,
            HELPER_PRIO, HELPER_STACK, THREAD_JOINABLE, 0);
        if (step(38, (u64)rc) == 0) {
            timed_handoffs();
            step(39, (u64)cgov_sc(SYS_PPU_THREAD_JOIN, tid[2], EA(&retval), 0, 0, 0, 0, 0, 0));
            step(40, (u64)(unsigned int)helper_wait_rc);
            step(41, (u64)(unsigned int)helper_post_rc);
        }
    }

    if (back_rc == 0)
        step(42, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, back_sem, 0, 0, 0, 0, 0, 0, 0));
    if (go_rc == 0)
        step(43, (u64)cgov_sc(SYS_SEMAPHORE_DESTROY, go_sem, 0, 0, 0, 0, 0, 0, 0));

    for (i = 0; i < LOOPS; i++) {
        put(16 + i, first_rc[i]);
        put(24 + i, last_rc[i]);
    }
    put(48, yields[0]);
    put(49, yields[1]);
    put(50, handoffs);
    put(57, first_failed);
    put(59, REPS);
    put(60, DONE);
    write_frame();
    return 0;
}
