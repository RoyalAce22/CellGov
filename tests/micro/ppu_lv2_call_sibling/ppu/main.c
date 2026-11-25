/* PPU probe: the round trip of a cheap LV2 call while the other hardware
 * thread runs user-mode work or makes LV2 calls of its own, each timed
 * with mftb.
 *
 * The main thread runs at priority 1001. Each phase starts one sibling
 * at priority 2000, waits for it to run, measures its rate while the main
 * thread sleeps, then times both calls beside it. The PPU runs two
 * hardware threads, so the two threads run at once; a yield finds no
 * other thread of its priority waiting and does not switch.
 *
 *   phase  sibling
 *   0      none
 *   1      a loop of 64 independent addi and a round count
 *   2      a loop of sc 147 sys_time_get_timebase_frequency and a count
 *   3      a loop of sc 43 sys_ppu_thread_yield and a count
 *
 *   call  timed loop: 1 << 12 calls, REPS runs
 *   0     sc 147 sys_time_get_timebase_frequency
 *   1     sc 43 sys_ppu_thread_yield
 *
 * Payload: big-endian u64 words. Loop L = 2 * phase + call:
 *   2L, 2L + 1  the loop's fastest and slowest run, in time-base ticks
 *   16 + L      the sibling's rounds over the loop's runs
 *   24 + L      the time-base ticks of the loop's runs
 *   32 + L      the loop's first nonzero return code, or 0
 * Phase p, at 40 + 4p (phase 0 stays zero):
 *   +0  the sibling's create rc (all ones: none created)
 *   +1  its join rc (all ones: not joined)
 *   +2  1 when it reported it runs
 *   +3  its exit value (expected 0x5157)
 * Word 56: the sibling's rounds while the main thread slept, phases 1 to
 * 3 at 56, 57 and 58. Word 59: the ticks of each sleep (the last). Word
 * 60: REPS. Word 61: the calls a run. Word 62: 0x5050_4C53 ("PPLS"): the
 * probe wrote every word.
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

#define MAIN_PRIORITY  1001
#define LOWER_PRIORITY 2000

SYS_PROCESS_PARAM(MAIN_PRIORITY, 0x10000)

#define SYS_PPU_THREAD_EXIT              41
#define SYS_PPU_THREAD_YIELD             43
#define SYS_PPU_THREAD_JOIN              44
#define SYS_TIMER_USLEEP                 141
#define SYS_TIME_GET_TIMEBASE_FREQUENCY  147

#define PHASES  4
#define WORDS   64
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define CALLS   (1u << 12)
#define DONE    0x50504C53ull

/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull
/* The sleep the sibling's own rate is measured over. */
#define ALONE_US 50000

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static volatile unsigned int started;
static volatile unsigned int kind;
/* Each on its own line: the sibling stores `rounds` every round and
 * loads `stop`; the main thread reads `rounds` only between runs. */
static volatile u64 rounds __attribute__((aligned(128)));
static volatile unsigned int stop __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

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

static struct cg_thread_param sibling_param __attribute__((aligned(8)));

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

#define ADDI4  "addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n"
#define ADDI16 ADDI4 ADDI4 ADDI4 ADDI4
#define ADDI64 ADDI16 ADDI16 ADDI16 ADDI16

static void sibling_entry(void *arg)
{
    unsigned k = kind;
    (void)arg;
    started = 1;
    if (k == 1) {
        __asm__ volatile (
            ".p2align 7\n"
            "1:\n" ADDI64
            "lwz 0,0(%[stop])\n"
            "cmpwi 0,0\n"
            "bne 3f\n"
            "ld 9,0(%[cnt])\n"
            "addi 9,9,1\n"
            "std 9,0(%[cnt])\n"
            "b 1b\n"
            "3:\n"
            : : [stop] "b"(&stop), [cnt] "b"(&rounds)
            : "r0", "r9", "r17", "r18", "r19", "r20", "cr0", "memory");
    } else {
        u64 num = k == 2 ? SYS_TIME_GET_TIMEBASE_FREQUENCY : SYS_PPU_THREAD_YIELD;
        while (!stop) {
            (void)cgov_sc(num, 0, 0, 0, 0, 0, 0, 0, 0);
            rounds = rounds + 1;
        }
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

static u64 first_rc[2 * PHASES];

/* CALLS calls of syscall `num`; returns the time-base ticks they took. */
static u64 __attribute__((noinline)) calls(unsigned loop, u64 num)
{
    u64 i, rc, bad = 0;
    u64 t0 = timebase();
    for (i = 0; i < CALLS; i++) {
        rc = (u64)cgov_sc(num, 0, 0, 0, 0, 0, 0, 0, 0);
        bad = bad ? bad : (rc & 0xFFFFFFFFull);
    }
    t0 = timebase() - t0;
    if (bad && !first_rc[loop])
        first_rc[loop] = bad;
    return t0;
}

/* Time call `c` as loop `loop`: REPS runs, the fastest and the slowest,
 * and the sibling's rounds and the ticks over all of them. */
static void timed(unsigned loop, unsigned c)
{
    u64 lo = ~0ull, hi = 0, ticks = 0, sib = 0;
    unsigned r;
    u64 num = c == 0 ? SYS_TIME_GET_TIMEBASE_FREQUENCY : SYS_PPU_THREAD_YIELD;
    for (r = 0; r < REPS; r++) {
        u64 before = rounds;
        u64 t = calls(loop, num);
        sib += rounds - before;
        ticks += t;
        lo = t < lo ? t : lo;
        hi = t > hi ? t : hi;
    }
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
    put(16 + loop, sib);
    put(24 + loop, ticks);
    put(32 + loop, first_rc[loop]);
}

int main(void)
{
    unsigned i, p;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    timed(0, 0);
    timed(1, 1);

    for (p = 1; p < PHASES; p++) {
        unsigned long long tid = 0, retval = 0;
        unsigned m = 40 + 4 * p;
        s32 rc;
        put(m + 0, ~0ull);
        put(m + 1, ~0ull);
        started = 0;
        stop = 0;
        rounds = 0;
        kind = p;
        __asm__ volatile ("sync" ::: "memory");
        rc = cgov_ppu_thread_create(
            &tid, make_thread_param(&sibling_param, (const void *)&sibling_entry),
            0, 0, LOWER_PRIORITY, 0x4000, THREAD_JOINABLE, 0);
        put(m + 0, (u64)(s64)rc);
        if (rc != 0)
            continue;
        {
            u64 t0 = timebase();
            u64 before;
            while (!started && timebase() - t0 < START_WAIT_TB) {
            }
            put(m + 2, started);
            before = rounds;
            t0 = timebase();
            (void)syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
            put(59, timebase() - t0);
            put(55 + p, rounds - before);
        }
        timed(2 * p, 0);
        timed(2 * p + 1, 1);
        stop = 1;
        __asm__ volatile ("sync" ::: "memory");
        put(m + 1, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(m + 3, retval);
    }
    put(60, REPS);
    put(61, CALLS);
    put(62, DONE);

    write_frame();
    return 0;
}
