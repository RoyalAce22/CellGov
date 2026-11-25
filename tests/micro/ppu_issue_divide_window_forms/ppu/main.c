/* PPU probe: whether a thread's rate beside a sibling of back-to-back
 * dependent divides or square roots is set by the sibling's interval
 * between them, for each divide form other than fdiv. The
 * ppu_issue_fdiv_window capture times the same loops beside back-to-back
 * fdiv; this probe puts fsqrt, fdivs, fsqrts and divd in that sibling.
 *
 *   sibling  an assembly loop: four passes of 8 dependent instructions of
 *            one form and a bdnz, then the stop flag's load and the round
 *            count's increment; its head at a 128-byte line; 32 of them a
 *            round; at priority 2000, the main thread at 1001
 *   phase    1 fsqrt  f = sqrt(f), f from 1.0
 *            2 fdivs  f = f / 1.0, f from 1.5
 *            3 fsqrts f = sqrt(f), f from 1.0
 *            4 divd   r = r / 1, r from 0x7fff0000
 *
 *   loop L   K[L] independent addi (r17..r24 in turn) and a bdnz, ITER
 *            iterations, its head at a 32-byte block; K = { 0, 2, 4, 8 }
 *
 * A run counts only when the sibling kept at least half the rate it ran
 * at while the main thread slept. Each loop runs until REPS runs count,
 * or MAX_RUNS runs.
 *
 * Payload: big-endian u64 words.
 *   L              loop L's fastest run alone, in time-base ticks
 *   16 + 120s + 30L + 2r
 *                  phase s + 1, loop L, counted run r: its ticks
 *   + 1            the sibling's rounds over it
 *   500 + 10s + j  phase s + 1's sibling: j = 0 its rounds while the main
 *                  thread slept, 1 the ticks of that sleep, 2 its create
 *                  rc, 3 its join rc, 4 its exit value (0x5157), 5 1 when
 *                  it reported it runs, 6 the sleep's rc
 *   540 + 4s + L   phase s + 1, loop L: the runs that count
 *   560 + 4s + L   the runs made
 *   630 the sum of every loop's result   631 REPS   632 ITER   633 LOOPS
 *   634 PHASES
 *   639 0x50504457 ("PPDW") when the probe wrote every word
 *
 * Payload: 640 words, 5120 bytes.
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

#define LOOPS    4
#define PHASES   4
#define WORDS    640
#define PAYLOAD  (WORDS * 8)
#define REPS     15
#define MAX_RUNS (8 * REPS)
#define ITER     2048u
#define DONE     0x50504457ull

/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull
/* The sleep the sibling's own rate is measured over. */
#define ALONE_US 50000

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 sink;

static volatile unsigned int started;
static volatile u64 rounds __attribute__((aligned(128)));
static volatile unsigned int stop __attribute__((aligned(128)));
/* The sibling's rounds and the ticks of the main thread's sleep. */
static u64 alone_rounds, alone_ticks;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)

#define A1 "addi 17,17,1\n"
#define A2 A1 "addi 18,18,1\n"
#define A4 A2 "addi 19,19,1\n" "addi 20,20,1\n"
#define A8 A4 "addi 21,21,1\n" "addi 22,22,1\n" "addi 23,23,1\n" "addi 24,24,1\n"

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

/* The sibling's round: four passes of 8 dependent `op` to a bdnz, then
 * stop when the stop flag is set, else count the round. */
#define ROUND(op)                                                  \
    ".p2align 7\n"                                                 \
    "1:\n"                                                         \
    "li 14,4\n"                                                    \
    "mtctr 14\n"                                                   \
    "2:\n" R8(op) "bdnz 2b\n"                                      \
    "lwz 0,0(%[stop])\n"                                           \
    "cmpwi 0,0\n"                                                  \
    "bne 3f\n"                                                     \
    "ld 9,0(%[cnt])\n"                                             \
    "addi 9,9,1\n"                                                 \
    "std 9,0(%[cnt])\n"                                            \
    "b 1b\n"                                                       \
    "3:\n"

#define ROUND_CLOBBERS "r0", "r9", "r14", "ctr", "cr0", "memory"

static void sibling_fsqrt(void *arg)
{
    double a = 1.0;
    (void)arg;
    started = 1;
    __asm__ volatile (ROUND("fsqrt %[a],%[a]\n")
        : [a] "+f"(a)
        : [stop] "b"(&stop), [cnt] "b"(&rounds)
        : ROUND_CLOBBERS);
    sink += (u64)a;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

static void sibling_fdivs(void *arg)
{
    float a = 1.5f, b = 1.0f;
    (void)arg;
    started = 1;
    __asm__ volatile (ROUND("fdivs %[a],%[a],%[b]\n")
        : [a] "+f"(a)
        : [b] "f"(b), [stop] "b"(&stop), [cnt] "b"(&rounds)
        : ROUND_CLOBBERS);
    sink += (u64)a;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

static void sibling_fsqrts(void *arg)
{
    float a = 1.0f;
    (void)arg;
    started = 1;
    __asm__ volatile (ROUND("fsqrts %[a],%[a]\n")
        : [a] "+f"(a)
        : [stop] "b"(&stop), [cnt] "b"(&rounds)
        : ROUND_CLOBBERS);
    sink += (u64)a;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

static void sibling_divd(void *arg)
{
    u64 a = 0x7fff0000ull, b = 1;
    (void)arg;
    started = 1;
    __asm__ volatile (ROUND("divd %[a],%[a],%[b]\n")
        : [a] "+r"(a)
        : [b] "r"(b), [stop] "b"(&stop), [cnt] "b"(&rounds)
        : ROUND_CLOBBERS);
    sink += a;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

typedef void (*sibling_fn)(void *);

static const sibling_fn siblings[PHASES] = {
    sibling_fsqrt, sibling_fdivs, sibling_fsqrts, sibling_divd,
};

/* A loop of `body` and a bdnz, `n` iterations; returns its ticks. */
#define LOOP(name, body)                                           \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 r;                                                     \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "li 17,0\n" "mtctr %[n]\n" ".p2align 5\n" "1:\n"       \
            body "bdnz 1b\n" "mr %[r],17\n"                        \
            : [r] "=&r"(r)                                         \
            : [n] "r"(n)                                           \
            : "r17", "r18", "r19", "r20", "r21", "r22", "r23",     \
              "r24", "ctr");                                       \
        t0 = timebase() - t0;                                      \
        sink += r;                                                 \
        return t0;                                                 \
    }

LOOP(loop_0, "")
LOOP(loop_2, A2)
LOOP(loop_4, A4)
LOOP(loop_8, A8)

typedef u64 (*loop_fn)(u64);

static const loop_fn loops[LOOPS] = { loop_0, loop_2, loop_4, loop_8 };

/* Loop `l` alone: its fastest of REPS runs. */
static void alone(unsigned l)
{
    u64 lo = ~0ull;
    unsigned r;
    for (r = 0; r < REPS; r++) {
        u64 t = loops[l](ITER);
        if (t < lo)
            lo = t;
    }
    put(l, lo);
}

/* Loop `l` beside the sibling of phase `s + 1`: each run that counts,
 * with the sibling's rounds over it. */
static void beside(unsigned s, unsigned l)
{
    unsigned runs = 0, counted = 0;
    while (counted < REPS && runs < MAX_RUNS) {
        u64 before = rounds;
        u64 t = loops[l](ITER);
        u64 n = rounds - before;
        runs++;
        if (2 * n * alone_ticks < alone_rounds * t)
            continue;
        put(16 + 120 * s + 30 * l + 2 * counted, t);
        put(17 + 120 * s + 30 * l + 2 * counted, n);
        counted++;
    }
    put(540 + 4 * s + l, counted);
    put(560 + 4 * s + l, runs);
}

static void with_sibling(unsigned s)
{
    unsigned long long tid = 0, retval = 0;
    unsigned base = 500 + 10 * s;
    unsigned l;
    s32 rc;

    put(base + 2, ~0ull);
    put(base + 3, ~0ull);
    put(base + 6, ~0ull);
    started = 0;
    stop = 0;
    rounds = 0;
    rc = cgov_ppu_thread_create(
        &tid, make_thread_param(&sibling_param, (const void *)siblings[s]),
        0, 0, LOWER_PRIORITY, 0x4000, THREAD_JOINABLE, 0);
    put(base + 2, (u64)(s64)rc);
    if (rc != 0)
        return;
    {
        u64 t0 = timebase();
        u64 before;
        while (!started && timebase() - t0 < START_WAIT_TB) {
        }
        put(base + 5, started);
        before = rounds;
        t0 = timebase();
        put(base + 6, (u64)(s64)syscall1_s32(SYS_TIMER_USLEEP, ALONE_US));
        alone_ticks = timebase() - t0;
        alone_rounds = rounds - before;
    }
    put(base + 0, alone_rounds);
    put(base + 1, alone_ticks);
    for (l = 0; l < LOOPS; l++)
        beside(s, l);
    stop = 1;
    put(base + 3, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
    put(base + 4, retval);
}

int main(void)
{
    unsigned i, l, s;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (l = 0; l < LOOPS; l++)
        alone(l);
    for (s = 0; s < PHASES; s++)
        with_sibling(s);

    put(630, sink);
    put(631, REPS);
    put(632, ITER);
    put(633, LOOPS);
    put(634, PHASES);
    put(639, DONE);
    write_frame();
    return 0;
}
