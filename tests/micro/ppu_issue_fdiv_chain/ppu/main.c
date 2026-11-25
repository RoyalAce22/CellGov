/* PPU probe: why a dependent chain and independent addi pairs pay
 * differently beside a lower-priority sibling of back-to-back dependent
 * fdiv. In the ppu_issue_fdiv_window capture each dispatch group of a
 * loop of independent addi waits about 4/3 of the sibling's divide
 * period. In the ppu_issue_shared capture a dependent add chain beside
 * the same divides takes about 15 cycles an instruction. The two
 * captures run the sibling in different shapes, so this probe times
 * each loop beside both.
 *
 *   sibling  32 dependent fdiv (1.5 / 1.0) a round, at priority 2000
 *            (the main thread at 1001)
 *   phase    0 no sibling
 *            1 the ppu_issue_fdiv_window shape: one assembly loop of four
 *              passes of 8 fdiv and a bdnz, then the stop flag's load
 *              and the round count's increment
 *            2 the ppu_issue_shared shape: a C loop that runs four passes
 *              of 8 fdiv and a bdnz in an asm block, increments the round
 *              count, and tests the stop flag and a time-base limit
 *
 *   loop L   ITER iterations, its head at a 32-byte block
 *            0 8 dependent add (add 17,17,15) and a bdnz
 *            1 4 dependent add and a bdnz
 *            2 8 independent addi (r17..r24) and a bdnz
 *            3 a bdnz
 *
 * In phases 1 and 2 a run counts only when the sibling kept at least half
 * the rate it ran at while the main thread slept. Each loop runs until
 * REPS runs count, or MAX_RUNS runs.
 *
 * Payload: big-endian u64 words.
 *   L              loop L's fastest run alone, in time-base ticks
 *   8 + 120s + 30L + 2r
 *                  phase s + 1, loop L, counted run r: its ticks
 *   + 1            the sibling's rounds over it
 *   260 + 10s + j  phase s + 1's sibling: j = 0 its rounds while the main
 *                  thread slept, 1 the ticks of that sleep, 2 its create
 *                  rc, 3 its join rc, 4 its exit value (0x5157), 5 1 when
 *                  it reported it runs, 6 the sleep's rc
 *   290 + 4s + L   phase s + 1, loop L: the runs that count
 *   300 + 4s + L   the runs made
 *   310 the sum of every loop's result   311 REPS   312 ITER   313 LOOPS
 *   319 0x50504643 ("PPFC") when the probe wrote every word
 *
 * Payload: 320 words, 2560 bytes.
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
#define WORDS    320
#define PAYLOAD  (WORDS * 8)
#define REPS     15
#define MAX_RUNS (8 * REPS)
#define ITER     2048u
#define DONE     0x50504643ull

/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull
/* The sleep the sibling's own rate is measured over. */
#define ALONE_US 50000
/* The C-loop sibling's own limit, as in ppu_issue_shared: 10 s. */
#define SIBLING_LIMIT_TB (79800000ull * 10ull)

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

#define A8 "addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n" \
           "addi 21,21,1\n" "addi 22,22,1\n" "addi 23,23,1\n" "addi 24,24,1\n"

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

/* Phase 1's sibling: one assembly loop. */
static void asm_sibling(void *arg)
{
    double a = 1.5, b = 1.0;
    (void)arg;
    started = 1;
    __asm__ volatile (
        ".p2align 7\n"
        "1:\n"
        "li 14,4\n"
        "mtctr 14\n"
        "2:\n" R8("fdiv %[a],%[a],%[b]\n") "bdnz 2b\n"
        "lwz 0,0(%[stop])\n"
        "cmpwi 0,0\n"
        "bne 3f\n"
        "ld 9,0(%[cnt])\n"
        "addi 9,9,1\n"
        "std 9,0(%[cnt])\n"
        "b 1b\n"
        "3:\n"
        : [a] "+f"(a)
        : [b] "f"(b), [stop] "b"(&stop), [cnt] "b"(&rounds)
        : "r0", "r9", "r14", "ctr", "cr0", "memory");
    sink += (u64)a;
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

/* Phase 2's sibling: ppu_issue_shared's round in a C loop. */
static void c_sibling(void *arg)
{
    u64 start = timebase();
    (void)arg;
    started = 1;
    while (!stop && timebase() - start < SIBLING_LIMIT_TB) {
        double a_ = 1.5, b_ = 1.0;
        __asm__ volatile (
            "li 14,4\n"
            "mtctr 14\n"
            "1:\n" R8("fdiv %[a],%[a],%[b]\n") "bdnz 1b\n"
            : [a] "+f"(a_) : [b] "f"(b_) : "r14", "ctr");
        sink += (u64)a_;
        rounds = rounds + 1;
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

/* A loop of `body` and a bdnz, `n` iterations; returns its ticks. */
#define LOOP(name, body)                                           \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 r;                                                     \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "li 15,3\n" "li 17,0\n" "mtctr %[n]\n"                 \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n" "mr %[r],17\n"  \
            : [r] "=&r"(r)                                         \
            : [n] "r"(n)                                           \
            : "r15", "r17", "r18", "r19", "r20", "r21", "r22",     \
              "r23", "r24", "ctr");                                \
        t0 = timebase() - t0;                                      \
        sink += r;                                                 \
        return t0;                                                 \
    }

LOOP(loop_dep8, R8("add 17,17,15\n"))
LOOP(loop_dep4, R4("add 17,17,15\n"))
LOOP(loop_ind8, A8)
LOOP(loop_bdnz, "")

typedef u64 (*loop_fn)(u64);

static const loop_fn loops[LOOPS] = { loop_dep8, loop_dep4, loop_ind8, loop_bdnz };

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
        put(8 + 120 * s + 30 * l + 2 * counted, t);
        put(9 + 120 * s + 30 * l + 2 * counted, n);
        counted++;
    }
    put(290 + 4 * s + l, counted);
    put(300 + 4 * s + l, runs);
}

static void with_sibling(unsigned s, void (*entry)(void *))
{
    unsigned long long tid = 0, retval = 0;
    unsigned base = 260 + 10 * s;
    unsigned l;
    s32 rc;

    put(base + 2, ~0ull);
    put(base + 3, ~0ull);
    put(base + 6, ~0ull);
    started = 0;
    stop = 0;
    rounds = 0;
    rc = cgov_ppu_thread_create(
        &tid, make_thread_param(&sibling_param, (const void *)entry),
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
    unsigned i, l;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (l = 0; l < LOOPS; l++)
        alone(l);
    with_sibling(0, asm_sibling);
    with_sibling(1, c_sibling);

    put(310, sink);
    put(311, REPS);
    put(312, ITER);
    put(313, LOOPS);
    put(319, DONE);
    write_frame();
    return 0;
}
