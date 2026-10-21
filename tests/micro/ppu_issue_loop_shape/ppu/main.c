/* PPU probe: whether the shape of a sibling's loop sets what the two
 * PPU threads cost each other. Each phase starts one sibling running
 * dependent divd or independent addi in one of four loop shapes,
 * measures the sibling's rate while the main thread sleeps, then times
 * three chains on the main thread beside it. Phase 0 runs no sibling.
 *
 *   Every shape runs four passes of 8 instructions and a bdnz a round.
 *   shape  between rounds
 *   A      a return to C that reads the time base, checks the stop flag
 *          and stores the round count (the ppu_issue_shared loop)
 *   B      in assembly: a load of the stop flag and a load, add and
 *          store of the round count (the ppu_issue_sibling loop)
 *   C      B with an mftb before the stop flag's load
 *   D      A with no time-base read
 *
 *   phase  sibling
 *   0      none
 *   1-4    dependent divd (0x7fff0000 / 1), shapes A to D
 *   5-8    independent addi (four targets), shapes A to D
 *
 *   chain  timed loop (1 << 16 iterations, bdnz)
 *   0      8 dependent add
 *   1      8 dependent fadd
 *   2      8 dependent ld (a pointer to itself, an L1 hit)
 *
 * The main thread runs at priority 1001 and the sibling at 2000.
 * The stop flag and the round count each sit on their own 128-byte line.
 *
 * Payload: big-endian u64 words. Loop L = 3 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  REPS runs
 *   54 + L         the sibling's rounds while the loop's REPS runs ran
 *   81 + L         the time-base ticks those REPS runs took
 * Phase p:
 *   108 + 2p       the sibling's rounds while the main thread slept
 *   109 + 2p       the time-base ticks of that sleep
 *   126 + p        the sibling's create rc (all ones: none created)
 *   135 + p        its join rc (all ones: not joined)
 *   144 + p        its exit value (expected 0x5157)
 *   153 + p        1 when it reported it runs
 * Word 162: the sum of every chain's result (keeps each loop live).
 * Word 163: REPS. Word 164: 0x5050534C ("PPSL"): the probe wrote every
 * word.
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

#define PHASES  9
#define CHAINS  3
#define WORDS   168
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define ITER    (1u << 16)

/* A C-loop sibling also stops 10 s of time base after it starts. */
#define SIBLING_LIMIT_TB (79800000ull * 10ull)
/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull
/* The sleep the sibling's own rate is measured over. */
#define ALONE_US 50000

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 sink;

static volatile unsigned int started;
static volatile unsigned int kind;
static volatile u64 rounds __attribute__((aligned(128)));
static volatile unsigned int stop __attribute__((aligned(128)));

static u64 self_ptr __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)

#define DIVD R8("divd 15,15,16\n")
#define ADDI R2("addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n")

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

/* One round of a C-loop sibling: four passes of `body` and a bdnz. */
#define ROUND(body)                                                \
    __asm__ volatile (                                             \
        "li 14,4\n"                                                \
        "mtctr 14\n"                                               \
        "lis 15,0x7fff\n"                                          \
        "li 16,1\n"                                                \
        "1:\n" body "bdnz 1b\n"                                    \
        : : : "r14", "r15", "r16", "r17", "r18", "r19", "r20", "ctr")

/* An assembly-loop sibling: rounds until the stop flag, `between` run
 * before the stop flag's load. The head sits at a 128-byte line. */
#define ASM_LOOP(body, between)                                    \
    __asm__ volatile (                                             \
        "lis 15,0x7fff\n"                                          \
        "li 16,1\n"                                                \
        ".p2align 7\n"                                             \
        "1:\n"                                                     \
        "li 14,4\n"                                                \
        "mtctr 14\n"                                               \
        "2:\n" body "bdnz 2b\n"                                    \
        between                                                    \
        "lwz 0,0(%[stop])\n"                                       \
        "cmpwi 0,0\n"                                              \
        "bne 3f\n"                                                 \
        "ld 9,0(%[cnt])\n"                                         \
        "addi 9,9,1\n"                                             \
        "std 9,0(%[cnt])\n"                                        \
        "b 1b\n"                                                   \
        "3:\n"                                                     \
        : : [stop] "b"(&stop), [cnt] "b"(&rounds)                  \
        : "r0", "r9", "r10", "r14", "r15", "r16", "r17", "r18",    \
          "r19", "r20", "ctr", "cr0", "memory")

static void __attribute__((noinline)) round_divd(void) { ROUND(DIVD); }
static void __attribute__((noinline)) round_addi(void) { ROUND(ADDI); }

static void sibling_entry(void *arg)
{
    unsigned k = kind;
    unsigned divides = k <= 4;
    unsigned shape = (k - 1) % 4;
    (void)arg;
    started = 1;
    if (shape == 0) {
        u64 start = timebase();
        while (!stop && timebase() - start < SIBLING_LIMIT_TB) {
            if (divides)
                round_divd();
            else
                round_addi();
            rounds = rounds + 1;
        }
    } else if (shape == 3) {
        while (!stop) {
            if (divides)
                round_divd();
            else
                round_addi();
            rounds = rounds + 1;
        }
    } else if (shape == 1) {
        if (divides)
            ASM_LOOP(DIVD, "");
        else
            ASM_LOOP(ADDI, "");
    } else {
        if (divides)
            ASM_LOOP(DIVD, "mftb 10\n");
        else
            ASM_LOOP(ADDI, "mftb 10\n");
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

static u64 __attribute__((noinline)) chain_add(u64 n)
{
    u64 a = 1;
    u64 t0 = timebase();
    __asm__ volatile (
        "li 15,3\n"
        LOOP_BEGIN R8("add %[a],%[a],15\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "r15", "ctr");
    sink += a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_fadd(u64 n)
{
    double a = 1.0, z = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("fadd %[a],%[a],%[z]\n") LOOP_END
        : [a] "+f"(a) : [z] "f"(z), [n] "r"(n) : "ctr");
    sink += (u64)a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_ld(u64 n)
{
    u64 p = (u64)(unsigned long)&self_ptr;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("ld %[p],0(%[p])\n") LOOP_END
        : [p] "+b"(p) : [n] "r"(n) : "ctr");
    sink += p;
    return timebase() - t0;
}

static u64 run_chain(unsigned c)
{
    switch (c) {
    case 0: return chain_add(ITER);
    case 1: return chain_fadd(ITER);
    default: return chain_ld(ITER);
    }
}

/* Time chain `c` REPS times as loop `loop`, with the sibling's rounds
 * and the ticks the runs took. */
static void timed(unsigned loop, unsigned c)
{
    u64 lo = ~0ull, hi = 0;
    u64 before = rounds;
    u64 t0 = timebase();
    unsigned r;
    for (r = 0; r < REPS; r++) {
        u64 t = run_chain(c);
        if (t < lo)
            lo = t;
        if (t > hi)
            hi = t;
    }
    put(81 + loop, timebase() - t0);
    put(54 + loop, rounds - before);
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i, p, c;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    self_ptr = (u64)(unsigned long)&self_ptr;

    for (p = 0; p < PHASES; p++) {
        unsigned long long tid = 0, retval = 0;
        s32 rc;
        put(126 + p, ~0ull);
        put(135 + p, ~0ull);
        if (p == 0) {
            for (c = 0; c < CHAINS; c++)
                timed(CHAINS * p + c, c);
            continue;
        }
        started = 0;
        stop = 0;
        rounds = 0;
        kind = p;
        rc = cgov_ppu_thread_create(
            &tid, make_thread_param(&sibling_param, (const void *)&sibling_entry),
            0, 0, LOWER_PRIORITY, 0x4000, THREAD_JOINABLE, 0);
        put(126 + p, (u64)(s64)rc);
        if (rc != 0)
            continue;
        {
            u64 t0 = timebase();
            u64 before;
            while (!started && timebase() - t0 < START_WAIT_TB) {
            }
            put(153 + p, started);
            before = rounds;
            t0 = timebase();
            syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
            put(109 + 2 * p, timebase() - t0);
            put(108 + 2 * p, rounds - before);
        }
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS * p + c, c);
        stop = 1;
        put(135 + p, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(144 + p, retval);
    }
    put(162, sink);
    put(163, REPS);
    put(164, 0x5050534Cull);

    write_frame();
    return 0;
}
