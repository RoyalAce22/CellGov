/* PPU probe: what an mftb costs the instructions around it. Each phase
 * times seven loops on the main thread; phase 1 runs them beside a
 * sibling thread that reads the time base in a loop, after measuring the
 * sibling's own rate while the main thread sleeps. Phase 0 runs no
 * sibling.
 *
 *   phase  sibling (32 instructions a round, 8 between bdnz)
 *   0      none
 *   1      mftb to four targets
 *
 *   chain  timed loop (1 << 14 iterations, bdnz)
 *   0      8 mftb (four targets)
 *   1      mftb, then 7 independent addi
 *   2      mftb, then 7 dependent add
 *   3      mftb, then 7 independent fadd (four targets)
 *   4      16 independent addi (four targets)
 *   5      8 dependent add
 *   6      8 dependent fadd
 *
 * The main thread runs at priority 1001 and the sibling at 2000, so the
 * two hold the PPE's two hardware threads.
 *
 * Payload: big-endian u64 words. Loop L = 7 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  REPS runs
 *   30 + L         the sibling's rounds while the loop's REPS runs ran
 *   50 + L         the time-base ticks those REPS runs took
 * Phase 1:
 *   70             the sibling's rounds while the main thread slept
 *   71             the time-base ticks of that sleep
 *   72             the sibling's create rc
 *   73             its join rc (all ones: not joined)
 *   74             its exit value (expected 0x4d42)
 *   75             1 when it reported it runs
 * Word 76: the sum of every chain's result (keeps each loop live).
 * Word 77: REPS. Word 78: ITER. Word 79: 0x50504d42 ("PPMB"): the probe
 * wrote every word.
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

#define PHASES  2
#define CHAINS  7
#define WORDS   80
#define PAYLOAD (WORDS * 8)
#define REPS    5
#define ITER    (1u << 14)

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

/* One sibling round: four passes of eight mftb and a bdnz. */
static void sibling_round(void)
{
    __asm__ volatile (
        "li 14,4\n"
        "mtctr 14\n"
        "1:\n"
        R2("mftb 15\n" "mftb 16\n" "mftb 17\n" "mftb 18\n")
        "bdnz 1b\n"
        : : : "r14", "r15", "r16", "r17", "r18", "ctr");
}

static void sibling_entry(void *arg)
{
    u64 start = timebase();
    (void)arg;
    started = 1;
    while (!stop && timebase() - start < SIBLING_LIMIT_TB) {
        sibling_round();
        rounds = rounds + 1;
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x4d42);
    for (;;) { }
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

static u64 __attribute__((noinline)) chain_mftb(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        R2("mftb 17\n" "mftb 18\n" "mftb 19\n" "mftb 20\n")
        LOOP_END
        : : [n] "r"(n) : "r17", "r18", "r19", "r20", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_mftb_addi(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        "mftb 17\n"
        "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n" "addi 21,21,1\n"
        "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n"
        LOOP_END
        : : [n] "r"(n) : "r17", "r18", "r19", "r20", "r21", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_mftb_add(u64 n)
{
    u64 a = 1;
    u64 t0 = timebase();
    __asm__ volatile (
        "li 15,3\n"
        LOOP_BEGIN
        "mftb 17\n"
        R4("add %[a],%[a],15\n") R2("add %[a],%[a],15\n") "add %[a],%[a],15\n"
        LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "r15", "r17", "ctr");
    sink += a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_mftb_fadd(u64 n)
{
    double a = 1.0, b = 1.0, c = 1.0, d = 1.0, z = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        "mftb 17\n"
        "fadd %[a],%[a],%[z]\n" "fadd %[b],%[b],%[z]\n"
        "fadd %[c],%[c],%[z]\n" "fadd %[d],%[d],%[z]\n"
        "fadd %[a],%[a],%[z]\n" "fadd %[b],%[b],%[z]\n"
        "fadd %[c],%[c],%[z]\n"
        LOOP_END
        : [a] "+f"(a), [b] "+f"(b), [c] "+f"(c), [d] "+f"(d)
        : [z] "f"(z), [n] "r"(n) : "r17", "ctr");
    sink += (u64)(a + b + c + d);
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_addi(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        R4("addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n")
        LOOP_END
        : : [n] "r"(n) : "r17", "r18", "r19", "r20", "ctr");
    return timebase() - t0;
}

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

static u64 run_chain(unsigned c)
{
    switch (c) {
    case 0: return chain_mftb(ITER);
    case 1: return chain_mftb_addi(ITER);
    case 2: return chain_mftb_add(ITER);
    case 3: return chain_mftb_fadd(ITER);
    case 4: return chain_addi(ITER);
    case 5: return chain_add(ITER);
    default: return chain_fadd(ITER);
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
    put(50 + loop, timebase() - t0);
    put(30 + loop, rounds - before);
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i, c;
    unsigned long long tid = 0, retval = 0;
    s32 rc;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (c = 0; c < CHAINS; c++)
        timed(c, c);

    put(72, ~0ull);
    put(73, ~0ull);
    started = 0;
    stop = 0;
    rounds = 0;
    rc = cgov_ppu_thread_create(
        &tid, make_thread_param(&sibling_param, (const void *)&sibling_entry),
        0, 0, LOWER_PRIORITY, 0x4000, THREAD_JOINABLE, 0);
    put(72, (u64)(s64)rc);
    if (rc == 0) {
        u64 t0 = timebase();
        u64 before;
        while (!started && timebase() - t0 < START_WAIT_TB) {
        }
        put(75, started);
        before = rounds;
        t0 = timebase();
        syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
        put(71, timebase() - t0);
        put(70, rounds - before);
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS + c, c);
        stop = 1;
        put(73, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(74, retval);
    }
    put(76, sink);
    put(77, REPS);
    put(78, ITER);
    put(79, 0x50504d42ull);

    write_frame();
    return 0;
}
