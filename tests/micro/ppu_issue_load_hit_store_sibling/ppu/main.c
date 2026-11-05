/* PPU probe: what a load that hits the store queue costs the other PPE
 * hardware thread, and what the other thread costs it. Each phase starts
 * one sibling running one loop, measures the sibling's rate while the
 * main thread sleeps, then times four chains on the main thread beside
 * it, counting the sibling's rounds while each chain runs. Phase 0 runs
 * no sibling.
 *
 *   phase  sibling (one round: four passes of eight instructions, bdnz)
 *   0      none
 *   1      independent addi
 *   2      std and ld of one doubleword of its own line, four pairs
 *          (each ld hits the std before it in the store queue)
 *
 *   chain  timed loop (1 << 16 iterations, bdnz)
 *   0      8 dependent add
 *   1      16 independent addi (four targets)
 *   2      std and ld of one doubleword of line A, four pairs, each pair
 *          chained through r9
 *   3      std and ld of line A, then 16 independent addi
 *
 * Line A and the sibling's line are separate 128-byte lines. The main
 * thread runs at priority 1001 and the sibling at 2000, so the two hold
 * the PPE's two hardware threads.
 *
 * Payload: big-endian u64 words. Loop L = 4 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  REPS runs
 *   24 + L         the sibling's rounds while the loop's REPS runs ran
 *   36 + L         the time-base ticks those REPS runs took
 * Phase p:
 *   48 + 2p        the sibling's rounds while the main thread slept
 *   49 + 2p        the time-base ticks of that sleep
 *   54 + p         the sibling's create rc (all ones: none created)
 *   57 + p         its join rc (all ones: not joined)
 *   60 + p         its exit value (expected 0x5157)
 *   63 + p         1 when it reported it runs
 * Word 66: the sum of every chain's result (keeps each loop live).
 * Word 67: REPS. Word 68: 0x504c5353 ("PLSS"): the probe wrote every
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

#define PHASES  3
#define CHAINS  4
#define WORDS   69
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

static volatile u64 line_a[16] __attribute__((aligned(128)));
static volatile u64 line_sibling[16] __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

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

static void sibling_round(unsigned k)
{
    if (k == 1) {
        __asm__ volatile (
            "li 14,4\n"
            "mtctr 14\n"
            "1:\n"
            R2("addi 17,17,1\n" "addi 18,18,1\n"
               "addi 19,19,1\n" "addi 20,20,1\n")
            "bdnz 1b\n"
            : : : "r14", "r17", "r18", "r19", "r20", "ctr");
        return;
    }
    {
        register u64 v17 __asm__("17") = 1;
        __asm__ volatile (
            "li 14,4\n"
            "mtctr 14\n"
            "1:\n"
            R4("std 17,0(%[p])\n" "ld 17,0(%[p])\n")
            "bdnz 1b\n"
            : "+r"(v17)
            : [p] "b"(EA(&line_sibling[0]))
            : "r14", "ctr", "memory");
        sink += v17;
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

#define ADDI_16 R4("addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n")
#define LHS_A   "std 9,0(%[p])\n" "ld 9,0(%[p])\n"

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

static u64 __attribute__((noinline)) chain_addi(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN ADDI_16 LOOP_END
        : : [n] "r"(n) : "r17", "r18", "r19", "r20", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_lhs(u64 n)
{
    register u64 v9 __asm__("9") = 1;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R4(LHS_A) LOOP_END
        : "+r"(v9) : [n] "r"(n), [p] "b"(EA(&line_a[0])) : "ctr", "memory");
    u64 t1 = timebase();
    sink += v9;
    return t1 - t0;
}

static u64 __attribute__((noinline)) chain_lhs_addi(u64 n)
{
    register u64 v9 __asm__("9") = 1;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN LHS_A ADDI_16 LOOP_END
        : "+r"(v9) : [n] "r"(n), [p] "b"(EA(&line_a[0]))
        : "r17", "r18", "r19", "r20", "ctr", "memory");
    u64 t1 = timebase();
    sink += v9;
    return t1 - t0;
}

static u64 run_chain(unsigned c)
{
    switch (c) {
    case 0: return chain_add(ITER);
    case 1: return chain_addi(ITER);
    case 2: return chain_lhs(ITER);
    default: return chain_lhs_addi(ITER);
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
    put(36 + loop, timebase() - t0);
    put(24 + loop, rounds - before);
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i, p, c;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 16; i++) {
        line_a[i] = i;
        line_sibling[i] = i;
    }

    for (p = 0; p < PHASES; p++) {
        unsigned long long tid = 0, retval = 0;
        s32 rc;
        put(54 + p, ~0ull);
        put(57 + p, ~0ull);
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
        put(54 + p, (u64)(s64)rc);
        if (rc != 0)
            continue;
        {
            u64 t0 = timebase();
            u64 before;
            while (!started && timebase() - t0 < START_WAIT_TB) {
            }
            put(63 + p, started);
            before = rounds;
            t0 = timebase();
            syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
            put(49 + 2 * p, timebase() - t0);
            put(48 + 2 * p, rounds - before);
        }
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS * p + c, c);
        stop = 1;
        put(57 + p, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(60 + p, retval);
    }
    put(66, sink);
    put(67, REPS);
    put(68, 0x504c5353ull);

    write_frame();
    return 0;
}
