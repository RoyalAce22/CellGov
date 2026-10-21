/* PPU probe: whether a sibling PPU thread's stall reaches the other PPE
 * hardware thread through the issue point (IS2) or through the VSU issue
 * queue (VQ8). Each phase starts one sibling running one instruction
 * class, measures the sibling's rate while the main thread sleeps, then
 * times seven chains on the main thread beside it. Phase 0 runs no
 * sibling.
 *
 *   phase  sibling (32 instructions a round, 8 between bdnz)
 *   0      none
 *   1      dependent divd      (0x7fff0000 / 1)
 *   2      dependent fdiv      (1.5 / 1.0)
 *   3      independent fdiv    (1.5 / 1.0, four targets)
 *   4      microcoded lha      (four targets, one L1 line)
 *   5      dependent fadd      (x + 0)
 *   6      dependent mulld     (x * 1)
 *
 *   chain  timed loop (1 << 16 iterations, bdnz)
 *   0      16 independent addi (four targets)
 *   1      8 dependent add
 *   2      8 dependent fadd
 *   3      16 independent fadd (four targets)
 *   4      8 dependent ld (a pointer to itself, an L1 hit)
 *   5      8 independent addi and 8 independent fadd, interleaved
 *   6      8 dependent fadd, an independent addi after each
 *
 * The main thread runs at priority 1001 and the sibling at 2000, so the
 * two hold the PPE's two hardware threads.
 *
 * Payload: big-endian u64 words. Loop L = 7 * phase + chain:
 *   2L, 2L + 1     the loop's smallest and largest time-base delta of
 *                  REPS runs
 *   100 + L        the sibling's rounds while the loop's REPS runs ran
 *   150 + L        the time-base ticks those REPS runs took
 * Phase p:
 *   200 + 2p       the sibling's rounds while the main thread slept
 *   201 + 2p       the time-base ticks of that sleep
 *   214 + p        the sibling's create rc (all ones: none created)
 *   221 + p        its join rc (all ones: not joined)
 *   228 + p        its exit value (expected 0x5157)
 *   235 + p        1 when it reported it runs
 * Word 242: the sum of every chain's result (keeps each loop live).
 * Word 243: REPS. Word 244: 0x50505554 ("PPUT"): the probe wrote every
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
#define CHAINS  7
#define WORDS   248
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
static u64 self_ptr __attribute__((aligned(128)));

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

#define SIB_FPR(body, a0, b0)                                      \
    do {                                                           \
        double a_ = (a0), b_ = (b0);                               \
        __asm__ volatile (                                         \
            "li 14,4\n"                                            \
            "mtctr 14\n"                                           \
            "1:\n" body "bdnz 1b\n"                                \
            : [a] "+f"(a_) : [b] "f"(b_) : "r14", "ctr");          \
        sink += (u64)a_;                                           \
    } while (0)

static void sibling_round(unsigned k)
{
    switch (k) {
    case 1:
        SIB_GPR(R8("divd 15,15,16\n"));
        break;
    case 2:
        SIB_FPR(R8("fdiv %[a],%[a],%[b]\n"), 1.5, 1.0);
        break;
    case 3: {
        double a = 1.5, b = 1.5, c = 1.5, d = 1.5, y = 1.0;
        __asm__ volatile (
            "li 14,4\n"
            "mtctr 14\n"
            "1:\n"
            R2("fdiv %[a],%[a],%[y]\n" "fdiv %[b],%[b],%[y]\n"
               "fdiv %[c],%[c],%[y]\n" "fdiv %[d],%[d],%[y]\n")
            "bdnz 1b\n"
            : [a] "+f"(a), [b] "+f"(b), [c] "+f"(c), [d] "+f"(d)
            : [y] "f"(y) : "r14", "ctr");
        sink += (u64)(a + b + c + d);
        break;
    }
    case 4:
        __asm__ volatile (
            "li 14,4\n"
            "mtctr 14\n"
            "1:\n"
            R2("lha 17,0(%[p])\n" "lha 18,2(%[p])\n"
               "lha 19,4(%[p])\n" "lha 20,6(%[p])\n")
            "bdnz 1b\n"
            : : [p] "b"(halves) : "r14", "r17", "r18", "r19", "r20", "ctr");
        break;
    case 5:
        SIB_FPR(R8("fadd %[a],%[a],%[b]\n"), 1.0, 0.0);
        break;
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

static u64 __attribute__((noinline)) chain_fadd_ind(u64 n)
{
    double a = 1.0, b = 1.0, c = 1.0, d = 1.0, z = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        R4("fadd %[a],%[a],%[z]\n" "fadd %[b],%[b],%[z]\n"
           "fadd %[c],%[c],%[z]\n" "fadd %[d],%[d],%[z]\n")
        LOOP_END
        : [a] "+f"(a), [b] "+f"(b), [c] "+f"(c), [d] "+f"(d)
        : [z] "f"(z), [n] "r"(n) : "ctr");
    sink += (u64)(a + b + c + d);
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

static u64 __attribute__((noinline)) chain_mixed(u64 n)
{
    double a = 1.0, b = 1.0, c = 1.0, d = 1.0, z = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        R2("addi 17,17,1\n" "fadd %[a],%[a],%[z]\n"
           "addi 18,18,1\n" "fadd %[b],%[b],%[z]\n"
           "addi 19,19,1\n" "fadd %[c],%[c],%[z]\n"
           "addi 20,20,1\n" "fadd %[d],%[d],%[z]\n")
        LOOP_END
        : [a] "+f"(a), [b] "+f"(b), [c] "+f"(c), [d] "+f"(d)
        : [z] "f"(z), [n] "r"(n) : "r17", "r18", "r19", "r20", "ctr");
    sink += (u64)(a + b + c + d);
    return timebase() - t0;
}

static u64 __attribute__((noinline)) chain_fadd_addi(u64 n)
{
    double a = 1.0, z = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("fadd %[a],%[a],%[z]\n" "addi 17,17,1\n") LOOP_END
        : [a] "+f"(a) : [z] "f"(z), [n] "r"(n) : "r17", "ctr");
    sink += (u64)a;
    return timebase() - t0;
}

static u64 run_chain(unsigned c)
{
    switch (c) {
    case 0: return chain_addi(ITER);
    case 1: return chain_add(ITER);
    case 2: return chain_fadd(ITER);
    case 3: return chain_fadd_ind(ITER);
    case 4: return chain_ld(ITER);
    case 5: return chain_mixed(ITER);
    default: return chain_fadd_addi(ITER);
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
    put(150 + loop, timebase() - t0);
    put(100 + loop, rounds - before);
    put(2 * loop, lo);
    put(2 * loop + 1, hi);
}

int main(void)
{
    unsigned i, p, c;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    self_ptr = (u64)(unsigned long)&self_ptr;
    for (i = 0; i < 64; i++)
        halves[i] = (unsigned short)i;

    for (p = 0; p < PHASES; p++) {
        unsigned long long tid = 0, retval = 0;
        s32 rc;
        put(214 + p, ~0ull);
        put(221 + p, ~0ull);
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
        put(214 + p, (u64)(s64)rc);
        if (rc != 0)
            continue;
        {
            u64 t0 = timebase();
            u64 before;
            while (!started && timebase() - t0 < START_WAIT_TB) {
            }
            put(235 + p, started);
            before = rounds;
            t0 = timebase();
            syscall1_s32(SYS_TIMER_USLEEP, ALONE_US);
            put(201 + 2 * p, timebase() - t0);
            put(200 + 2 * p, rounds - before);
        }
        for (c = 0; c < CHAINS; c++)
            timed(CHAINS * p + c, c);
        stop = 1;
        put(221 + p, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(228 + p, retval);
    }
    put(242, sink);
    put(243, REPS);
    put(244, 0x50505554ull);

    write_frame();
    return 0;
}
