/* PPU probe: what a sibling that changes instruction class partway
 * through the timed loop costs the main thread. The main thread times a
 * loop of sixteen independent addi in two halves, and between them sets
 * a flag the sibling reads once a round.
 *
 *   phase  sibling (an assembly loop, its head at a 128-byte line)
 *   0      none
 *   1      fdiv rounds before the flag and after it
 *   2      addi rounds before the flag and after it
 *   3      fdiv rounds before the flag, addi rounds after it
 *   4      addi rounds before the flag, fdiv rounds after it
 *   5      8 dependent fdiv, then 256 independent addi, each round;
 *          it reads no flag
 *
 * An fdiv round is four passes of 8 dependent fdiv (1.5 / 1.0) and a
 * bdnz. An addi round is four passes of 64 independent addi (four
 * targets) and a bdnz. Each round of phases 1 to 4 first loads the flag
 * and branches to the round it selects. Each round of every phase ends
 * with the stop flag's load and the round count's increment.
 *
 *   timed loop  16 independent addi (four targets) and a bdnz, its head
 *               at a 128-byte line; HALF iterations, the flag set, then
 *               HALF iterations more
 *
 * The main thread runs at priority 1001 and the sibling at 2000.
 *
 * Before a phase's runs the main thread sleeps twice, with the flag
 * clear and then set, and reads the sibling's rounds over each sleep: the
 * sibling's rate in the round it runs before the flag and after it. A
 * run counts only when, in each half, the sibling kept at least an
 * eighth of the rate of that half's round: the kernel can take the
 * sibling's hardware thread for a while, and a run then times no
 * sibling. Each phase runs until REPS runs count, or MAX_RUNS runs.
 *
 * Payload: big-endian u64 words. Run r (0 to REPS - 1) of the runs that
 * count in phase p, at 4 * (REPS * p + r):
 *   + 0            the first half's time-base ticks
 *   + 1            the second half's
 *   + 2            the sibling's rounds in the first half
 *   + 3            its rounds in the second half
 * Phase p:
 *   360 + 2p       the runs that count
 *   361 + 2p       the runs made
 *   372 + 10p + j  the sibling of phase p (all ones in phase 0):
 *                  j = 0, 1  its rounds, and the ticks, of the sleep
 *                            with the flag clear
 *                  j = 2, 3  the same with the flag set
 *                  j = 4     its create rc
 *                  j = 5     its join rc (all ones: not joined)
 *                  j = 6     its exit value (expected 0x5157)
 *                  j = 7     1 when it reported it runs
 *                  j = 8, 9  the two sleeps' rc (all ones: none slept)
 * Word 432: the sum of the loop's results (keeps it live). Word 433:
 * REPS. Word 434: HALF. Word 435: 0x50505357 ("PPSW"): the probe wrote
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

#define PHASES   6
#define WORDS    436
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

/* The sibling's rounds and the ticks of each sleep: zero rounds while no
 * sibling runs. */
static u64 alone_rounds[2], alone_ticks[2];

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)
#define R16(x) R8(x) R8(x)

#define ADDI4 "addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n"
#define FDIV8 R8("fdiv %[a],%[a],%[b]\n")
#define ADDI64 R16(ADDI4)

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

/* 8 dependent fdiv, then four passes of 64 addi, each round. */
#define SIB_ALTERNATE                                              \
    ".p2align 7\n"                                                 \
    "1:\n"                                                         \
    FDIV8                                                          \
    "li 14,4\n"                                                    \
    "mtctr 14\n"                                                   \
    "3:\n" ADDI64 "bdnz 3b\n"                                      \
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
    switch (sib_phase) {
    case 1:
        __asm__ volatile (SIB_SWITCH(FDIV8, FDIV8) SIB_OPERANDS);
        break;
    case 2:
        __asm__ volatile (SIB_SWITCH(ADDI64, ADDI64) SIB_OPERANDS);
        break;
    case 3:
        __asm__ volatile (SIB_SWITCH(FDIV8, ADDI64) SIB_OPERANDS);
        break;
    case 4:
        __asm__ volatile (SIB_SWITCH(ADDI64, FDIV8) SIB_OPERANDS);
        break;
    default:
        __asm__ volatile (SIB_ALTERNATE SIB_OPERANDS);
        break;
    }
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

/* Whether `n` rounds over `t` ticks keep an eighth of sleep `s`'s rate. */
static int kept(unsigned s, u64 n, u64 t)
{
    return alone_rounds[s] == 0 || 8 * n * alone_ticks[s] >= alone_rounds[s] * t;
}

/* Time phase `p`'s runs until REPS runs count. */
static void timed(unsigned p)
{
    unsigned runs = 0, counted = 0;
    while (counted < REPS && runs < MAX_RUNS) {
        u64 r0, r1, r2, t0, t1, t2;
        mid = 0;
        r0 = rounds;
        t0 = timebase();
        sink += half(HALF);
        t1 = timebase();
        r1 = rounds;
        mid = 1;
        sink += half(HALF);
        t2 = timebase();
        r2 = rounds;
        mid = 0;
        runs++;
        if (!kept(0, r1 - r0, t1 - t0) || !kept(1, r2 - r1, t2 - t1))
            continue;
        put(4 * (REPS * p + counted) + 0, t1 - t0);
        put(4 * (REPS * p + counted) + 1, t2 - t1);
        put(4 * (REPS * p + counted) + 2, r1 - r0);
        put(4 * (REPS * p + counted) + 3, r2 - r1);
        counted++;
    }
    put(360 + 2 * p, counted);
    put(361 + 2 * p, runs);
}

/* The sibling's rounds over a sleep of ALONE_US, into slot `s`. */
static s32 sleep_rate(unsigned s)
{
    u64 before = rounds;
    u64 t0 = timebase();
    s32 rc = syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
    alone_ticks[s] = timebase() - t0;
    alone_rounds[s] = rounds - before;
    return rc;
}

static void with_sibling(unsigned p)
{
    unsigned long long tid = 0, retval = 0;
    unsigned base = 372 + 10 * p;
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
    put(base + 8, (u64)(s64)sleep_rate(0));
    mid = 1;
    put(base + 9, (u64)(s64)sleep_rate(1));
    mid = 0;
    put(base + 0, alone_rounds[0]);
    put(base + 1, alone_ticks[0]);
    put(base + 2, alone_rounds[1]);
    put(base + 3, alone_ticks[1]);
    timed(p);
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
        put(372 + i, ~0ull);
    alone_rounds[0] = alone_rounds[1] = 0;
    timed(0);

    for (p = 1; p < PHASES; p++)
        with_sibling(p);

    put(432, sink);
    put(433, REPS);
    put(434, HALF);
    put(435, 0x50505357ull);

    write_frame();
    return 0;
}
