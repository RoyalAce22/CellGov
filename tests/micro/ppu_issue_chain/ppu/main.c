/* PPU probe: where a chain of dependent fixed-point instructions loses
 * the fraction of a cycle the issue model does not give it. Each loop
 * runs a fixed body ITER times between two time-base reads. The probe
 * runs each loop REPS times and records the smallest and the largest
 * delta. The loop branch is bdnz.
 *
 * Loops 0 to 3 vary how many dependent add an iteration holds, and 4 to
 * 6 how many dependent mulli: an excess per iteration stays fixed as the
 * chain grows, and an excess per instruction grows with it. Loops 8 and
 * 9 run 256 of each between branches. Loops 10 and 11 repeat loops 1 and
 * 5 beside a sibling thread that holds the PPE's other hardware thread
 * with a chain of divd, which dispatches about once each 40 cycles.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1. The words from 24 on are listed after the
 * loops.
 *   loop
 *   0  4 dependent add        (1 << 21 iterations)
 *   1  8 dependent add        (1 << 20)
 *   2  16 dependent add       (1 << 19)
 *   3  32 dependent add       (1 << 18)
 *   4  4 dependent mulli      (1 << 18)
 *   5  8 dependent mulli      (1 << 17)
 *   6  16 dependent mulli     (1 << 16)
 *   7  8 dependent mullw      (1 << 17)
 *   8  256 dependent add      (1 << 15)
 *   9  256 dependent mulli    (1 << 12)
 *  10  loop 1 beside the divd sibling
 *  11  loop 5 beside the divd sibling
 *   word
 *  24  the sibling's create rc (sc 52, then sc 53)
 *  25  1 when the sibling reported it runs before loops 10 and 11
 *  26  the sibling's rounds (32 divd each) while loops 10 and 11 ran
 *  27  the sibling's join rc (all ones: not joined)
 *  28  the sibling's exit value (expected 0x5157)
 *  29  the sum of every loop's result register (keeps each loop live)
 *  30  REPS
 *  31  0x50504348 ("PPCH"): the probe wrote every word
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

#define WORDS   32
#define PAYLOAD (WORDS * 8)
#define REPS    5

/* The sibling stops at the stop flag, or 10 s of time base after it
 * starts, whichever comes first. */
#define SIBLING_LIMIT_TB (79800000ull * 10ull)
/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 sink;

static volatile unsigned int started;
static volatile unsigned int stop;
static volatile u64 rounds;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)
#define R16(x) R8(x) R8(x)
#define R32(x) R16(x) R16(x)
#define R256(x) R32(R8(x))

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

/* Run `expr`, one timed loop, REPS times; keep the smallest delta in
 * word 2 * loop and the largest in the word after it. */
#define TIMED(loop, expr)                              \
    do {                                               \
        u64 lo_ = ~0ull, hi_ = 0;                      \
        unsigned r_;                                   \
        for (r_ = 0; r_ < REPS; r_++) {                \
            u64 t_ = (expr);                           \
            if (t_ < lo_)                              \
                lo_ = t_;                              \
            if (t_ > hi_)                              \
                hi_ = t_;                              \
        }                                              \
        put(2 * (loop), lo_);                          \
        put(2 * (loop) + 1, hi_);                      \
    } while (0)

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

/* The sibling: dependent divd until the stop flag or its time limit. */
static void sibling_entry(void *arg)
{
    u64 start = timebase();
    (void)arg;
    started = 1;
    while (!stop && timebase() - start < SIBLING_LIMIT_TB) {
        __asm__ volatile (
            "li 14,32\n"
            "mtctr 14\n"
            "lis 15,0x7fff\n"
            "li 16,1\n"
            "1:\n"
            "divd 15,15,16\n"
            "bdnz 1b\n"
            : : : "r14", "r15", "r16", "ctr");
        rounds = rounds + 1;
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

#define DEP_ADD(name, rep)                                         \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 a = 1;                                                 \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "li 15,3\n"                                            \
            LOOP_BEGIN rep("add %[a],%[a],15\n") LOOP_END          \
            : [a] "+r"(a) : [n] "r"(n) : "r15", "ctr");            \
        sink += a;                                                 \
        return timebase() - t0;                                    \
    }

#define DEP_MULLI(name, rep)                                       \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 a = 7;                                                 \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN rep("mulli %[a],%[a],1\n") LOOP_END         \
            : [a] "+r"(a) : [n] "r"(n) : "ctr");                   \
        sink += a;                                                 \
        return timebase() - t0;                                    \
    }

DEP_ADD(loop_add_4, R4)
DEP_ADD(loop_add_8, R8)
DEP_ADD(loop_add_16, R16)
DEP_ADD(loop_add_32, R32)
DEP_ADD(loop_add_256, R256)
DEP_MULLI(loop_mulli_4, R4)
DEP_MULLI(loop_mulli_8, R8)
DEP_MULLI(loop_mulli_16, R16)
DEP_MULLI(loop_mulli_256, R256)

static u64 __attribute__((noinline)) loop_mullw_8(u64 n)
{
    u64 a = 7;
    u64 t0 = timebase();
    __asm__ volatile (
        "li 15,1\n"
        LOOP_BEGIN R8("mullw %[a],%[a],15\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "r15", "ctr");
    sink += a;
    return timebase() - t0;
}

int main(void)
{
    unsigned i;
    unsigned long long tid = 0, retval = 0;
    s32 rc;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    put(27, ~0ull);

    TIMED(0, loop_add_4(1u << 21));
    TIMED(1, loop_add_8(1u << 20));
    TIMED(2, loop_add_16(1u << 19));
    TIMED(3, loop_add_32(1u << 18));
    TIMED(4, loop_mulli_4(1u << 18));
    TIMED(5, loop_mulli_8(1u << 17));
    TIMED(6, loop_mulli_16(1u << 16));
    TIMED(7, loop_mullw_8(1u << 17));
    TIMED(8, loop_add_256(1u << 15));
    TIMED(9, loop_mulli_256(1u << 12));

    started = 0;
    stop = 0;
    rc = cgov_ppu_thread_create(
        &tid, make_thread_param(&sibling_param, (const void *)&sibling_entry),
        0, 0, LOWER_PRIORITY, 0x4000, THREAD_JOINABLE, 0);
    put(24, (u64)(s64)rc);
    if (rc == 0) {
        u64 t0 = timebase();
        u64 before;
        while (!started && timebase() - t0 < START_WAIT_TB) {
        }
        put(25, started);
        before = rounds;
        TIMED(10, loop_add_8(1u << 20));
        TIMED(11, loop_mulli_8(1u << 17));
        put(26, rounds - before);
        stop = 1;
        put(27, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(28, retval);
    }
    put(29, sink);
    put(30, REPS);
    put(31, 0x50504348ull);

    write_frame();
    return 0;
}
