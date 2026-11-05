/* PPU probe: which form of the ppu_issue_mix_switch probe's steady fdiv
 * sibling and timed loop moves what the sibling costs the main thread.
 * The main thread times a loop of sixteen independent addi beside a
 * sibling of dependent fdiv rounds.
 *
 *   phase  sibling (an assembly loop, its head at a 128-byte line)
 *          and timed loop
 *   0      none; two halves
 *   1      fdiv rounds that each first load the flag; two halves
 *   2      fdiv rounds that load no flag; two halves
 *   3      fdiv rounds that each first load the flag; one loop
 *   4      fdiv rounds that load no flag; one loop
 *
 * An fdiv round is four passes of 8 dependent fdiv (1.5 / 1.0) and a
 * bdnz. A round that loads the flag first branches on it to the same
 * fdiv passes either way. Each round ends with the stop flag's load and
 * the round count's increment. Phase 1 is the mix_switch probe's phase 1.
 *
 *   two halves  16 independent addi (four targets) and a bdnz, its head
 *               at a 128-byte line; HALF iterations, the flag set, then
 *               HALF iterations more
 *   one loop    the same loop, 2 * HALF iterations in one timed loop
 *
 * The main thread runs at priority 1001 and the sibling at 2000.
 *
 * Before a phase's runs the main thread sleeps once and reads the
 * sibling's rounds over the sleep: the sibling's rate alone. A run
 * counts only when the sibling kept at least an eighth of that rate in
 * each timed part. Each phase runs until REPS runs count, or MAX_RUNS
 * runs.
 *
 * Payload: big-endian u64 words. Run r (0 to REPS - 1) of the runs that
 * count in phase p, at 4 * (REPS * p + r):
 *   + 0            the first half's time-base ticks (one loop: all of it)
 *   + 1            the second half's (one loop: 0)
 *   + 2            the sibling's rounds in the first half (one loop: all)
 *   + 3            its rounds in the second half (one loop: 0)
 * Phase p:
 *   300 + 2p       the runs that count
 *   301 + 2p       the runs made
 *   310 + 10p + j  the sibling of phase p (all ones in phase 0):
 *                  j = 0, 1  its rounds, and the ticks, of the sleep
 *                  j = 4     its create rc
 *                  j = 5     its join rc (all ones: not joined)
 *                  j = 6     its exit value (expected 0x5157)
 *                  j = 7     1 when it reported it runs
 *                  j = 8     the sleep's rc (all ones: none slept)
 * Word 360: the sum of the loop's results (keeps it live). Word 361:
 * REPS. Word 362: HALF. Word 363: 0x50504654 ("PPFT"): the probe wrote
 * every word.
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

#define MAIN_PRIORITY  1001
#define LOWER_PRIORITY 2000

SYS_PROCESS_PARAM(MAIN_PRIORITY, 0x10000)

#define SYS_PPU_THREAD_EXIT 41
#define SYS_PPU_THREAD_JOIN 44
#define SYS_TIMER_USLEEP    141

#define PHASES   5
#define WORDS    364
#define PAYLOAD  (WORDS * 8)
#define REPS     15
#define MAX_RUNS (8 * REPS)
#define HALF     2048u

/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull
/* The sleep the sibling's own rate is measured over. */
#define ALONE_US 50000

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 sink;

static volatile unsigned int started;
static volatile unsigned int sib_phase;
static volatile u64 rounds __attribute__((aligned(128)));
static volatile unsigned int stop __attribute__((aligned(128)));
static volatile unsigned int mid __attribute__((aligned(128)));

/* The sibling's rounds and the ticks of its sleep: zero rounds while no
 * sibling runs. */
static u64 alone_rounds[1], alone_ticks[1];

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)
#define R16(x) R8(x) R8(x)

#define ADDI4 "addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n"
#define FDIV8 R8("fdiv %[a],%[a],%[b]\n")

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

/* The end of every sibling round: stop when the stop flag is set, else
 * count the round and go back to the head. */
#define SIB_TAIL                                                   \
    "5:\n"                                                         \
    "lwz 0,0(%[stop])\n"                                           \
    "cmpwi 0,0\n"                                                  \
    "bne 9f\n"                                                     \
    "ld 9,0(%[cnt])\n"                                             \
    "addi 9,9,1\n"                                                 \
    "std 9,0(%[cnt])\n"                                            \
    "b 1b\n"                                                       \
    "9:\n"

/* A round of `before` while the flag is clear, of `after` once it is
 * set: each four passes to a bdnz. */
#define SIB_SWITCH(before, after)                                  \
    ".p2align 7\n"                                                 \
    "1:\n"                                                         \
    "lwz 0,0(%[mid])\n"                                            \
    "cmpwi 0,0\n"                                                  \
    "bne 4f\n"                                                     \
    "li 14,4\n"                                                    \
    "mtctr 14\n"                                                   \
    "2:\n" before "bdnz 2b\n"                                      \
    "b 5f\n"                                                       \
    "4:\n"                                                         \
    "li 14,4\n"                                                    \
    "mtctr 14\n"                                                   \
    "3:\n" after "bdnz 3b\n"                                       \
    SIB_TAIL

/* Four passes of 8 dependent fdiv each round, with no flag load. */
#define SIB_PLAIN                                                  \
    ".p2align 7\n"                                                 \
    "1:\n"                                                         \
    "li 14,4\n"                                                    \
    "mtctr 14\n"                                                   \
    "3:\n" FDIV8 "bdnz 3b\n"                                       \
    SIB_TAIL

#define SIB_OPERANDS                                               \
    : [a] "+f"(a)                                                  \
    : [b] "f"(b), [stop] "b"(&stop), [cnt] "b"(&rounds),           \
      [mid] "b"(&mid)                                              \
    : "r0", "r9", "r14", "r17", "r18", "r19", "r20", "ctr", "cr0", \
      "memory"

static void sibling_entry(void *arg)
{
    double a = 1.5, b = 1.0;
    (void)arg;
    started = 1;
    if (sib_phase == 1 || sib_phase == 3)
        __asm__ volatile (SIB_SWITCH(FDIV8, FDIV8) SIB_OPERANDS);
    else
        __asm__ volatile (SIB_PLAIN SIB_OPERANDS);
    sink += (u64)a;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

/* `n` iterations of the timed loop; returns r17. */
static u64 __attribute__((noinline)) half(u64 n)
{
    u64 r;
    __asm__ volatile (
        "mtctr %[n]\n"
        ".p2align 7\n"
        "1:\n" R4(ADDI4) "bdnz 1b\n"
        "mr %[r],17\n"
        : [r] "=r"(r)
        : [n] "r"(n)
        : "r17", "r18", "r19", "r20", "ctr");
    return r;
}

/* Whether `n` rounds over `t` ticks keep an eighth of the sibling's rate
 * alone. */
static int kept(u64 n, u64 t)
{
    return alone_rounds[0] == 0 || 8 * n * alone_ticks[0] >= alone_rounds[0] * t;
}

/* Time phase `p`'s runs until REPS runs count: two halves with the flag
 * set between them, or one loop. */
static void timed(unsigned p, int one_loop)
{
    unsigned runs = 0, counted = 0;
    while (counted < REPS && runs < MAX_RUNS) {
        u64 r0, r1, r2, t0, t1, t2;
        mid = 0;
        r0 = rounds;
        t0 = timebase();
        if (one_loop) {
            sink += half(2 * HALF);
            t1 = t2 = timebase();
            r1 = r2 = rounds;
        } else {
            sink += half(HALF);
            t1 = timebase();
            r1 = rounds;
            mid = 1;
            sink += half(HALF);
            t2 = timebase();
            r2 = rounds;
        }
        mid = 0;
        runs++;
        if (!kept(r1 - r0, t1 - t0) || (!one_loop && !kept(r2 - r1, t2 - t1)))
            continue;
        put(4 * (REPS * p + counted) + 0, t1 - t0);
        put(4 * (REPS * p + counted) + 1, t2 - t1);
        put(4 * (REPS * p + counted) + 2, r1 - r0);
        put(4 * (REPS * p + counted) + 3, r2 - r1);
        counted++;
    }
    put(300 + 2 * p, counted);
    put(301 + 2 * p, runs);
}

/* The sibling's rounds over a sleep of ALONE_US. */
static s32 sleep_rate(void)
{
    u64 before = rounds;
    u64 t0 = timebase();
    s32 rc = syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
    alone_ticks[0] = timebase() - t0;
    alone_rounds[0] = rounds - before;
    return rc;
}

static void with_sibling(unsigned p)
{
    unsigned long long tid = 0, retval = 0;
    unsigned base = 310 + 10 * p;
    unsigned j;
    s32 rc;

    for (j = 0; j < 10; j++)
        put(base + j, ~0ull);
    started = 0;
    stop = 0;
    mid = 0;
    rounds = 0;
    sib_phase = p;
    rc = cgov_ppu_thread_create(
        &tid, make_thread_param(&sibling_param, (const void *)&sibling_entry),
        0, 0, LOWER_PRIORITY, 0x4000, THREAD_JOINABLE, 0);
    put(base + 4, (u64)(s64)rc);
    if (rc != 0)
        return;
    {
        u64 t0 = timebase();
        while (!started && timebase() - t0 < START_WAIT_TB) {
        }
    }
    put(base + 7, started);
    put(base + 8, (u64)(s64)sleep_rate());
    put(base + 0, alone_rounds[0]);
    put(base + 1, alone_ticks[0]);
    timed(p, p >= 3);
    stop = 1;
    put(base + 5, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
    put(base + 6, retval);
}

int main(void)
{
    unsigned i, p;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (i = 0; i < 10; i++)
        put(310 + i, ~0ull);
    alone_rounds[0] = 0;
    timed(0, 0);

    for (p = 1; p < PHASES; p++)
        with_sibling(p);

    put(360, sink);
    put(361, REPS);
    put(362, HALF);
    put(363, 0x50504654ull);

    write_frame();
    return 0;
}
