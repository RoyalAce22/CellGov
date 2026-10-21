/* PPU probe: what a multiply chain loses beside a sibling PPU thread
 * whose instructions stall the shared issue point. Each phase starts one
 * sibling running one instruction class, measures the sibling's rate
 * while the main thread sleeps, then times seven chains on the main
 * thread beside it. Phase 0 runs no sibling.
 *
 *   phase  sibling (32 instructions a round, 8 between bdnz)
 *   0      none
 *   1      dependent divd      (0x7fff0000 / 1)
 *   2      dependent fdiv      (1.5 / 1.0)
 *   3      dependent mulld     (x * 1)
 *
 *   chain  timed loop (1 << 16 iterations, bdnz)
 *   0      8 dependent mulli
 *   1      8 independent mulli (four targets)
 *   2      8 dependent mullw
 *   3      8 dependent mulld
 *   4      8 dependent mulhw
 *   5      8 dependent mulli, an independent add after each
 *   6      8 dependent add
 *
 * The main thread runs at priority 1001 and the sibling at 2000, so the
 * two hold the PPE's two hardware threads.
 *
 * Payload: big-endian u64 words. Loop L = 7 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  REPS runs
 *   56 + L         the sibling's rounds while the loop's REPS runs ran
 *   84 + L         the time-base ticks those REPS runs took
 * Phase p:
 *   112 + 2p       the sibling's rounds while the main thread slept
 *   113 + 2p       the time-base ticks of that sleep
 *   120 + p        the sibling's create rc (all ones: none created)
 *   124 + p        its join rc (all ones: not joined)
 *   128 + p        its exit value (expected 0x5157)
 *   132 + p        1 when it reported it runs
 * Word 136: the sum of every chain's result (keeps each loop live).
 * Word 137: REPS. Word 138: 0x50504d55 ("PPMU"): the probe wrote every
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

#define PHASES  4
#define CHAINS  7
#define WORDS   144
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define ITER    (1u << 16)

/* The sibling stops at the stop flag, or 10 s of time base after it
 * starts, whichever comes first. */
#define SIBLING_LIMIT_TB (79800000ull * 10ull)
/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull
/* The sleep the sibling's own rate is measured over. */
#define ALONE_US 50000

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 sink;

static volatile unsigned int started;
static volatile unsigned int stop;
static volatile unsigned int kind;
static volatile u64 rounds;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)

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

/* One sibling round: four passes of eight instructions and a bdnz. */
#define SIB_GPR(body)                                              \
    __asm__ volatile (                                             \
        "li 14,4\n"                                                \
        "mtctr 14\n"                                               \
        "lis 15,0x7fff\n"                                          \
        "li 16,1\n"                                                \
        "1:\n" body "bdnz 1b\n"                                    \
        : : : "r14", "r15", "r16", "ctr")

static void sibling_round(unsigned k)
{
    switch (k) {
    case 1:
        SIB_GPR(R8("divd 15,15,16\n"));
        break;
    case 2: {
        double a = 1.5, b = 1.0;
        __asm__ volatile (
            "li 14,4\n"
            "mtctr 14\n"
            "1:\n" R8("fdiv %[a],%[a],%[b]\n") "bdnz 1b\n"
            : [a] "+f"(a) : [b] "f"(b) : "r14", "ctr");
        sink += (u64)a;
        break;
    }
    default:
        SIB_GPR(R8("mulld 15,15,16\n"));
        break;
    }
}

static void sibling_entry(void *arg)
{
    u64 start = timebase();
    unsigned k = kind;
    (void)arg;
    started = 1;
    while (!stop && timebase() - start < SIBLING_LIMIT_TB) {
        sibling_round(k);
        rounds = rounds + 1;
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

/* A loop of `body` on %[a], seeded with 7, with r15 = 1; r16 is free. */
#define DEP_CHAIN(name, body)                                      \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 a = 7;                                                 \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "li 15,1\n"                                            \
            LOOP_BEGIN body LOOP_END                               \
            : [a] "+r"(a) : [n] "r"(n) : "r15", "r16", "ctr");     \
        sink += a;                                                 \
        return timebase() - t0;                                    \
    }

DEP_CHAIN(chain_mulli, R8("mulli %[a],%[a],1\n"))
DEP_CHAIN(chain_mullw, R8("mullw %[a],%[a],15\n"))
DEP_CHAIN(chain_mulld, R8("mulld %[a],%[a],15\n"))
DEP_CHAIN(chain_mulhw, R8("mulhw %[a],%[a],15\n"))
DEP_CHAIN(chain_mulli_add, R8("mulli %[a],%[a],1\n" "add 16,16,15\n"))
DEP_CHAIN(chain_add, R8("add %[a],%[a],15\n"))

static u64 __attribute__((noinline)) chain_mulli_ind(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        "li 17,7\n" "li 18,7\n" "li 19,7\n" "li 20,7\n"
        LOOP_BEGIN
        R2("mulli 17,17,1\n" "mulli 18,18,1\n" "mulli 19,19,1\n" "mulli 20,20,1\n")
        LOOP_END
        : : [n] "r"(n) : "r17", "r18", "r19", "r20", "ctr");
    return timebase() - t0;
}

static u64 run_chain(unsigned c)
{
    switch (c) {
    case 0: return chain_mulli(ITER);
    case 1: return chain_mulli_ind(ITER);
    case 2: return chain_mullw(ITER);
    case 3: return chain_mulld(ITER);
    case 4: return chain_mulhw(ITER);
    case 5: return chain_mulli_add(ITER);
    default: return chain_add(ITER);
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
    put(84 + loop, timebase() - t0);
    put(56 + loop, rounds - before);
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i, p, c;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (p = 0; p < PHASES; p++) {
        unsigned long long tid = 0, retval = 0;
        s32 rc;
        put(120 + p, ~0ull);
        put(124 + p, ~0ull);
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
        put(120 + p, (u64)(s64)rc);
        if (rc != 0)
            continue;
        {
            u64 t0 = timebase();
            u64 before;
            while (!started && timebase() - t0 < START_WAIT_TB) {
            }
            put(132 + p, started);
            before = rounds;
            t0 = timebase();
            syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
            put(113 + 2 * p, timebase() - t0);
            put(112 + 2 * p, rounds - before);
        }
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS * p + c, c);
        stop = 1;
        put(124 + p, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(128 + p, retval);
    }
    put(136, sink);
    put(137, REPS);
    put(138, 0x50504d55ull);

    write_frame();
    return 0;
}
