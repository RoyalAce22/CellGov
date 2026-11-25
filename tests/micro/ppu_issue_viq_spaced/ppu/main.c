/* PPU probe: whether fixed-point work between dependent fdiv changes how
 * many of them the VSU issue queue holds behind a running divide before
 * the sibling's stall reaches the issue stage both threads share. The
 * main thread times a loop of sixteen independent addi beside a sibling
 * whose every round runs N dependent fdiv, K independent addi after each,
 * and then 256 independent addi.
 *
 *   phase  1 + 5k + n for K = 0, 1, 2, 4 (k = 0..3) and N = 5, 6, 7, 8, 10
 *          (n = 0..4); phase 0 has no sibling. The sibling is an assembly
 *          loop, its head at a 128-byte line.
 *
 * The fdiv divide 1.5 by 1.0, each the result of the one before. The K
 * addi after each fdiv are addi 21..24,21..24,1 in turn. The 256 addi are
 * four passes of 64 independent addi (four targets) to a bdnz. Each round
 * ends with the stop flag's load and the round count's increment.
 *
 *   timed loop  16 independent addi (four targets) and a bdnz, its head
 *               at a 128-byte line; ITER iterations
 *
 * The main thread runs at priority 1001 and the sibling at 2000.
 *
 * Before a phase's runs the main thread sleeps, and reads the sibling's
 * rounds over the sleep: the sibling's own round time. A run counts only
 * when the sibling kept at least an eighth of that rate. Each phase runs
 * until REPS runs count, or MAX_RUNS runs.
 *
 * Payload: big-endian u64 words. Run r (0 to REPS - 1) of the runs that
 * count in phase p, at 2 * (REPS * p + r):
 *   + 0            the timed loop's time-base ticks
 *   + 1            the sibling's rounds over it
 * Phase p, at 630 + 9p + j:
 *   j = 0          the runs that count
 *   j = 1          the runs made
 *   j = 2, 3       the sibling's rounds, and the ticks, over the sleep
 *   j = 4          its create rc (all ones in phase 0)
 *   j = 5          its join rc (all ones: not joined)
 *   j = 6          its exit value (expected 0x5157)
 *   j = 7          1 when it reported it runs
 *   j = 8          the sleep's rc (all ones: none slept)
 * Word 819: the sum of the loop's results (keeps it live). Word 820:
 * REPS. Word 821: ITER. Word 822: 0x50505653 ("PPVS"): the probe wrote
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

#define PHASES   21
#define META     630
#define WORDS    823
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

/* The K addi after each fdiv. */
#define SP0 ""
#define SP1 "addi 21,21,1\n"
#define SP2 SP1 "addi 22,22,1\n"
#define SP4 SP2 "addi 23,23,1\n" "addi 24,24,1\n"

/* N copies of `x`. */
#define N5(x) R4(x) x
#define N6(x) R4(x) R2(x)
#define N7(x) R4(x) R2(x) x
#define N8(x) R8(x)
#define N10(x) R8(x) R2(x)

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
      "r23", "r24", "ctr", "cr0", "memory"

/* Phase `p`'s sibling: N copies of an fdiv and its K addi. */
#define CASE(p, n, sp)                                             \
    case p:                                                        \
        __asm__ volatile (SIB_ROUND(n(FDIV sp)) SIB_OPERANDS);     \
        break;
#define ROW(base, sp)                                              \
    CASE(base + 0, N5, sp)                                         \
    CASE(base + 1, N6, sp)                                         \
    CASE(base + 2, N7, sp)                                         \
    CASE(base + 3, N8, sp)                                         \
    CASE(base + 4, N10, sp)

static void sibling_entry(void *arg)
{
    double a = 1.5, b = 1.0;
    (void)arg;
    started = 1;
    switch (sib_phase) {
    ROW(1, SP0)
    ROW(6, SP1)
    ROW(11, SP2)
    ROW(16, SP4)
    default:
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
    put(META + 9 * p, counted);
    put(META + 1 + 9 * p, runs);
}

static void with_sibling(unsigned p)
{
    unsigned long long tid = 0, retval = 0;
    unsigned base = META + 9 * p;
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
        put(META + i, ~0ull);
    alone_rounds = 0;
    timed(0);

    for (p = 1; p < PHASES; p++)
        with_sibling(p);

    put(819, sink);
    put(820, REPS);
    put(821, ITER);
    put(822, 0x50505653ull);

    write_frame();
    return 0;
}
