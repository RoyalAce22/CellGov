/* PPU probe: why a sibling with no divide presses harder on a timed loop
 * than a sibling with one. In the ppu_issue_viq_depth probe the sibling
 * with no fdiv costs the loop about a quarter more than the sibling with
 * one to seven. Here the round's head holds padding in place of the
 * divides, so the inner loop of 64 addi starts at another word of its
 * line, with no divide in the round.
 *
 *   phase  the round's head (the sibling is an assembly loop, its head
 *            at a 128-byte line; phase 0 has no sibling)
 *   0      no sibling
 *   1      nothing (the inner loop at word 2 of the line)
 *   2      1 fdiv
 *   3..7   1, 2, 3, 4, 6 nop
 *   8, 9   1, 2 independent addi (r21, r22)
 *   10     30 nop (the inner loop at word 0 of the next line)
 *
 * The fdiv divides 1.5 by 1.0. The addi are four passes of 64
 * independent addi (four targets) to a bdnz. Each round ends with the
 * stop flag's load and the round count's increment.
 *
 *   timed loop  16 independent addi (four targets) and a bdnz, its head
 *               at a 128-byte line; ITER iterations
 *
 * The main thread runs at priority 1001 and the sibling at 2000.
 *
 * Before a phase's runs the main thread sleeps, and reads the sibling's
 * rounds over the sleep: the sibling's own round time. A run counts only
 * when the sibling kept at least an eighth of that rate: the kernel can
 * take the sibling's hardware thread for a while, and a run then times no
 * sibling. Each phase runs until REPS runs count, or MAX_RUNS runs.
 *
 * Payload: big-endian u64 words. Run r (0 to REPS - 1) of the runs that
 * count in phase p, at 2 * (REPS * p + r):
 *   + 0            the timed loop's time-base ticks
 *   + 1            the sibling's rounds over it
 * Phase p, at 390 + 9p + j:
 *   j = 0          the runs that count
 *   j = 1          the runs made
 *   j = 2, 3       the sibling's rounds, and the ticks, over the sleep
 *   j = 4          its create rc (all ones in phase 0)
 *   j = 5          its join rc (all ones: not joined)
 *   j = 6          its exit value (expected 0x5157)
 *   j = 7          1 when it reported it runs
 *   j = 8          the sleep's rc (all ones: none slept)
 * Word 507: the sum of the loop's results (keeps it live). Word 508:
 * REPS. Word 509: ITER. Word 510: 0x5050564C ("PPVL"): the probe wrote
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

#define PHASES   11
#define WORDS    511
#define PAYLOAD  (WORDS * 8)
#define REPS     15
#define MAX_RUNS (8 * REPS)
#define ITER     4096u

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

/* The sibling's rounds and the ticks of the sleep: zero rounds while no
 * sibling runs. */
static u64 alone_rounds, alone_ticks;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)
#define R16(x) R8(x) R8(x)

#define ADDI4 "addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n"
#define ADDI64 R16(ADDI4)
#define FDIV "fdiv %[a],%[a],%[b]\n"
#define NOP "nop\n"
#define PAD_ADDI1 "addi 21,21,1\n"
#define PAD_ADDI2 "addi 21,21,1\n" "addi 22,22,1\n"

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

/* One sibling round: `divides`, then four passes of 64 addi; stop when
 * the stop flag is set, else count the round and go back to the head. */
#define SIB_ROUND(divides)                                         \
    ".p2align 7\n"                                                 \
    "1:\n"                                                         \
    divides                                                        \
    "li 14,4\n"                                                    \
    "mtctr 14\n"                                                   \
    "3:\n" ADDI64 "bdnz 3b\n"                                      \
    "lwz 0,0(%[stop])\n"                                           \
    "cmpwi 0,0\n"                                                  \
    "bne 9f\n"                                                     \
    "ld 9,0(%[cnt])\n"                                             \
    "addi 9,9,1\n"                                                 \
    "std 9,0(%[cnt])\n"                                            \
    "b 1b\n"                                                       \
    "9:\n"

#define SIB_OPERANDS                                               \
    : [a] "+f"(a)                                                  \
    : [b] "f"(b), [stop] "b"(&stop), [cnt] "b"(&rounds)            \
    : "r0", "r9", "r14", "r17", "r18", "r19", "r20", "r21", "r22", \
      "ctr", "cr0", "memory"

static void sibling_entry(void *arg)
{
    double a = 1.5, b = 1.0;
    (void)arg;
    started = 1;
    switch (sib_phase) {
    case 1:
        __asm__ volatile (SIB_ROUND("") SIB_OPERANDS);
        break;
    case 2:
        __asm__ volatile (SIB_ROUND(FDIV) SIB_OPERANDS);
        break;
    case 3:
        __asm__ volatile (SIB_ROUND(NOP) SIB_OPERANDS);
        break;
    case 4:
        __asm__ volatile (SIB_ROUND(R2(NOP)) SIB_OPERANDS);
        break;
    case 5:
        __asm__ volatile (SIB_ROUND(R2(NOP) NOP) SIB_OPERANDS);
        break;
    case 6:
        __asm__ volatile (SIB_ROUND(R4(NOP)) SIB_OPERANDS);
        break;
    case 7:
        __asm__ volatile (SIB_ROUND(R4(NOP) R2(NOP)) SIB_OPERANDS);
        break;
    case 8:
        __asm__ volatile (SIB_ROUND(PAD_ADDI1) SIB_OPERANDS);
        break;
    case 9:
        __asm__ volatile (SIB_ROUND(PAD_ADDI2) SIB_OPERANDS);
        break;
    default:
        __asm__ volatile (SIB_ROUND(R16(NOP) R8(NOP) R4(NOP) R2(NOP)) SIB_OPERANDS);
        break;
    }
    sink += (u64)a;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

/* `n` iterations of the timed loop; returns r17. */
static u64 __attribute__((noinline)) timed_loop(u64 n)
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

/* Whether `n` rounds over `t` ticks keep an eighth of the sleep's rate. */
static int kept(u64 n, u64 t)
{
    return alone_rounds == 0 || 8 * n * alone_ticks >= alone_rounds * t;
}

/* Time phase `p`'s runs until REPS runs count. */
static void timed(unsigned p)
{
    unsigned runs = 0, counted = 0;
    while (counted < REPS && runs < MAX_RUNS) {
        u64 r0, r1, t0, t1;
        r0 = rounds;
        t0 = timebase();
        sink += timed_loop(ITER);
        t1 = timebase();
        r1 = rounds;
        runs++;
        if (!kept(r1 - r0, t1 - t0))
            continue;
        put(2 * (REPS * p + counted) + 0, t1 - t0);
        put(2 * (REPS * p + counted) + 1, r1 - r0);
        counted++;
    }
    put(390 + 9 * p, counted);
    put(391 + 9 * p, runs);
}

static void with_sibling(unsigned p)
{
    unsigned long long tid = 0, retval = 0;
    unsigned base = 390 + 9 * p;
    unsigned j;
    s32 rc;

    for (j = 2; j < 9; j++)
        put(base + j, ~0ull);
    started = 0;
    stop = 0;
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
    {
        u64 before = rounds;
        u64 t0 = timebase();
        s32 slept = syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
        alone_ticks = timebase() - t0;
        alone_rounds = rounds - before;
        put(base + 8, (u64)(s64)slept);
    }
    put(base + 2, alone_rounds);
    put(base + 3, alone_ticks);
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

    for (i = 2; i < 9; i++)
        put(390 + i, ~0ull);
    alone_rounds = 0;
    timed(0);

    for (p = 1; p < PHASES; p++)
        with_sibling(p);

    put(507, sink);
    put(508, REPS);
    put(509, ITER);
    put(510, 0x5050564Cull);

    write_frame();
    return 0;
}
