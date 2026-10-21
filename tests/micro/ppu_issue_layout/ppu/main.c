/* PPU probe: whether a dependent add chain's cost beside a sibling PPU
 * thread moves with where the chain or the sibling's loop sits in its
 * 128-byte instruction-cache line. Each phase starts one sibling running
 * one loop at one line offset, measures the sibling's rate while the
 * main thread sleeps, then times the same chain at eight line offsets
 * beside it. Phase 0 runs no sibling.
 *
 *   phase  sibling (32 instructions a round, 8 between bdnz)
 *   0      none
 *   1      microcoded lha    (four targets, one L1 line), loop at +0x00
 *   2      microcoded lha,                               loop at +0x40
 *   3      dependent mulld   (x * 1),                    loop at +0x00
 *   4      dependent mulld,                              loop at +0x40
 *   5      dependent divd    (0x7fff0000 / 1),           loop at +0x00
 *   6      dependent fdiv    (1.5 / 1.0),                loop at +0x00
 *
 *   chain  timed loop (1 << 16 iterations, bdnz): 8 dependent add, the
 *          loop's first instruction at 16 * chain bytes past a 128-byte
 *          line (chains 0 to 7)
 *
 * The main thread runs at priority 1001 and the sibling at 2000, so the
 * two hold the PPE's two hardware threads.
 *
 * Payload: big-endian u64 words. Loop L = 8 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  REPS runs
 *   112 + L        the sibling's rounds while the loop's REPS runs ran
 *   168 + L        the time-base ticks those REPS runs took
 * Phase p:
 *   224 + 2p       the sibling's rounds while the main thread slept
 *   225 + 2p       the time-base ticks of that sleep
 *   238 + p        the sibling's create rc (all ones: none created)
 *   245 + p        its join rc (all ones: not joined)
 *   252 + p        its exit value (expected 0x5157)
 *   259 + p        1 when it reported it runs
 * Word 266: the sum of every chain's result (keeps each loop live).
 * Word 267: REPS. Word 268: 0x5050_4c59 ("PPLY"): the probe wrote every
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

#define PHASES  7
#define CHAINS  8
#define WORDS   272
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

static unsigned short halves[64] __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)

/* Pad to a 128-byte line, then `n` nops: the next label sits 4n bytes
 * past the line. The pad runs once, before the loop. */
#define AT(n) ".p2align 7\n" ".rept " #n "\n" "nop\n" ".endr\n"

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

/* One sibling round: four passes of eight instructions and a bdnz, the
 * loop at `at` nops past a 128-byte line. */
#define SIB_GPR(at, body)                                          \
    __asm__ volatile (                                             \
        "li 14,4\n"                                                \
        "mtctr 14\n"                                               \
        "lis 15,0x7fff\n"                                          \
        "li 16,1\n"                                                \
        AT(at) "1:\n" body "bdnz 1b\n"                             \
        : : : "r14", "r15", "r16", "ctr")

#define SIB_LHA(at)                                                \
    __asm__ volatile (                                             \
        "li 14,4\n"                                                \
        "mtctr 14\n"                                               \
        AT(at) "1:\n"                                              \
        R2("lha 17,0(%[p])\n" "lha 18,2(%[p])\n"                   \
           "lha 19,4(%[p])\n" "lha 20,6(%[p])\n")                  \
        "bdnz 1b\n"                                                \
        : : [p] "b"(halves) : "r14", "r17", "r18", "r19", "r20", "ctr")

static void sibling_round(unsigned k)
{
    switch (k) {
    case 1:
        SIB_LHA(0);
        break;
    case 2:
        SIB_LHA(16);
        break;
    case 3:
        SIB_GPR(0, R8("mulld 15,15,16\n"));
        break;
    case 4:
        SIB_GPR(16, R8("mulld 15,15,16\n"));
        break;
    case 5:
        SIB_GPR(0, R8("divd 15,15,16\n"));
        break;
    default: {
        double a = 1.5, b = 1.0;
        __asm__ volatile (
            "li 14,4\n"
            "mtctr 14\n"
            AT(0) "1:\n" R8("fdiv %[a],%[a],%[b]\n") "bdnz 1b\n"
            : [a] "+f"(a) : [b] "f"(b) : "r14", "ctr");
        sink += (u64)a;
        break;
    }
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

/* Eight dependent add, the loop `at` nops past a 128-byte line. */
#define ADD_CHAIN(name, at)                                        \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 a = 1;                                                 \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "li 15,3\n"                                            \
            "mtctr %[n]\n"                                         \
            AT(at) "1:\n" R8("add %[a],%[a],15\n") "bdnz 1b\n"     \
            : [a] "+r"(a) : [n] "r"(n) : "r15", "ctr");            \
        sink += a;                                                 \
        return timebase() - t0;                                    \
    }

ADD_CHAIN(chain_0, 0)
ADD_CHAIN(chain_1, 4)
ADD_CHAIN(chain_2, 8)
ADD_CHAIN(chain_3, 12)
ADD_CHAIN(chain_4, 16)
ADD_CHAIN(chain_5, 20)
ADD_CHAIN(chain_6, 24)
ADD_CHAIN(chain_7, 28)

static u64 run_chain(unsigned c)
{
    switch (c) {
    case 0: return chain_0(ITER);
    case 1: return chain_1(ITER);
    case 2: return chain_2(ITER);
    case 3: return chain_3(ITER);
    case 4: return chain_4(ITER);
    case 5: return chain_5(ITER);
    case 6: return chain_6(ITER);
    default: return chain_7(ITER);
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
    put(168 + loop, timebase() - t0);
    put(112 + loop, rounds - before);
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i, p, c;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 64; i++)
        halves[i] = (unsigned short)i;

    for (p = 0; p < PHASES; p++) {
        unsigned long long tid = 0, retval = 0;
        s32 rc;
        put(238 + p, ~0ull);
        put(245 + p, ~0ull);
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
        put(238 + p, (u64)(s64)rc);
        if (rc != 0)
            continue;
        {
            u64 t0 = timebase();
            u64 before;
            while (!started && timebase() - t0 < START_WAIT_TB) {
            }
            put(259 + p, started);
            before = rounds;
            t0 = timebase();
            syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
            put(225 + 2 * p, timebase() - t0);
            put(224 + 2 * p, rounds - before);
        }
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS * p + c, c);
        stop = 1;
        put(245 + p, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(252 + p, retval);
    }
    put(266, sink);
    put(267, REPS);
    put(268, 0x50504c59ull);

    write_frame();
    return 0;
}
