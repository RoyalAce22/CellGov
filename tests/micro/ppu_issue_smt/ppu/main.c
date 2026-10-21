/* PPU probe: the PPE's issue timing beside busy sibling threads. Each
 * loop runs a fixed body ITER times between two time-base reads. The
 * probe runs each loop REPS times and records the smallest and the
 * largest delta. The loop branch is bdnz.
 *
 * A sibling runs independent addi until its phase's stop flag or a time
 * limit. Sibling 0 runs beside loops 1 and 3. Siblings 1 and 2 run beside
 * loops 4 and 5 at a lower priority than the main thread's 1001.
 * Siblings 3 and 4 run beside loops 6 and 7 at the main thread's
 * priority.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1. The words from 16 on are listed after the
 * loops.
 *   loop
 *   0  4 x (lha, addi), all independent       (1 << 18 iterations)
 *   1  loop 0 beside one sibling
 *   2  8 x (bne not taken, addi)              (1 << 20)
 *   3  loop 2 beside one sibling
 *   4  8 independent addi beside two lower-priority siblings  (1 << 20)
 *   5  8 dependent add beside two lower-priority siblings     (1 << 20)
 *   6  8 independent addi beside two siblings at priority 1001
 *   7  8 dependent add beside two siblings at priority 1001
 *   word
 *  16-20  sibling k's create rc (sc 52, then sc 53)
 *  21-25  sibling k's join rc (all ones: not joined)
 *  26-30  sibling k's exit value (expected 0x5157)
 *  31-35  sibling k's rounds (512 addi each, as ppu_issue_timing's
 *         sibling) while its loops ran
 *  36     bit k: sibling k reported it runs before its loops began. A
 *         phase's loops run once every create in it succeeds, whether
 *         or not each sibling got to run.
 *  37     the sum of every loop's result register (keeps each loop live)
 *  38     REPS
 *  39     0x50505354 ("PPST"): the probe wrote every word
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

#define WORDS    40
#define PAYLOAD  (WORDS * 8)
#define REPS     5
#define SIBLINGS 5

/* A sibling stops at its phase's stop flag, or 10 s of time base after
 * it starts, whichever comes first. */
#define SIBLING_LIMIT_TB (79800000ull * 10ull)
/* The main thread waits at most 1 s for a phase's siblings to start. */
#define START_WAIT_TB 79800000ull

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 one_line[16] __attribute__((aligned(128)));
static volatile u64 sink;

static volatile unsigned int started[SIBLINGS];
static volatile unsigned int stop;
static volatile u64 rounds[SIBLINGS];
static s32 create_rc[SIBLINGS];

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))
#define R4(x) x x x x
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

static struct cg_thread_param params[SIBLINGS] __attribute__((aligned(8)));
static unsigned long long tids[SIBLINGS];

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

/* A sibling: independent addi until the stop flag or its time limit.
 * `arg` is its index. */
static void sibling_entry(void *arg)
{
    unsigned k = (unsigned)(unsigned long)arg;
    u64 start = timebase();
    started[k] = 1;
    while (!stop && timebase() - start < SIBLING_LIMIT_TB) {
        __asm__ volatile (
            "li 14,32\n"
            "mtctr 14\n"
            "1:\n"
            R8("addi 15,15,1\n" "addi 16,16,1\n")
            "bdnz 1b\n"
            : : : "r14", "r15", "r16", "ctr");
        rounds[k] = rounds[k] + 1;
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

/* Bit k: sibling k reported it runs within START_WAIT_TB of its start. */
static u64 ran;

/* Start siblings `first` to `first + n - 1` at `prio`, and wait up to
 * START_WAIT_TB for each to report it runs. Returns 1 when every one was
 * created. */
static int start_siblings(unsigned first, unsigned n, u64 prio)
{
    unsigned k;
    int ok = 1;
    u64 t0;
    stop = 0;
    for (k = first; k < first + n; k++) {
        started[k] = 0;
        create_rc[k] = cgov_ppu_thread_create(
            &tids[k], make_thread_param(&params[k], (const void *)&sibling_entry),
            k, 0, prio, 0x4000, THREAD_JOINABLE, 0);
        put(16 + k, (u64)(s64)create_rc[k]);
        if (create_rc[k] != 0)
            ok = 0;
    }
    t0 = timebase();
    for (k = first; k < first + n; k++) {
        if (create_rc[k] != 0)
            continue;
        while (!started[k] && timebase() - t0 < START_WAIT_TB) {
        }
        if (started[k])
            ran |= 1ull << k;
    }
    return ok;
}

/* Stop and join siblings `first` to `first + n - 1`, recording each join
 * rc and exit value. Only a sibling whose create succeeded is joined. */
static void join_siblings(unsigned first, unsigned n)
{
    unsigned k;
    stop = 1;
    for (k = first; k < first + n; k++) {
        unsigned long long retval = 0;
        if (create_rc[k] != 0)
            continue;
        put(21 + k, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tids[k],
                                           (unsigned long)&retval));
        put(26 + k, retval);
    }
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

static u64 __attribute__((noinline)) loop_pair_lha_addi(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        "lha 18,0(%[p])\n" "addi 14,14,1\n" "lha 19,8(%[p])\n" "addi 15,15,1\n"
        "lha 20,16(%[p])\n" "addi 16,16,1\n" "lha 21,24(%[p])\n" "addi 17,17,1\n"
        LOOP_END
        : : [n] "r"(n), [p] "b"(EA(one_line))
        : "r14", "r15", "r16", "r17", "r18", "r19", "r20", "r21", "ctr");
    return timebase() - t0;
}

/* The compare before the loop sets CR0's EQ bit, so each bne falls
 * through. Each bne sits at an 8-byte-aligned address with its addi in
 * the same dispatch group. */
static u64 __attribute__((noinline)) loop_branch_not_taken(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        "li 14,0\n"
        "cmpdi 14,0\n"
        LOOP_BEGIN R8("bne 2f\n" "2:\n" "addi 15,15,1\n") LOOP_END
        : : [n] "r"(n) : "r14", "r15", "cr0", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_indep_addi(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        "addi 14,14,1\n" "addi 15,15,1\n" "addi 16,16,1\n" "addi 17,17,1\n"
        "addi 18,18,1\n" "addi 19,19,1\n" "addi 20,20,1\n" "addi 21,21,1\n"
        LOOP_END
        : : [n] "r"(n)
        : "r14", "r15", "r16", "r17", "r18", "r19", "r20", "r21", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_add(u64 n)
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

int main(void)
{
    unsigned i;
    u64 before[SIBLINGS];

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < SIBLINGS; i++) {
        create_rc[i] = -1;
        put(16 + i, ~0ull);
        put(21 + i, ~0ull);
    }

    TIMED(0, loop_pair_lha_addi(1u << 18));
    TIMED(2, loop_branch_not_taken(1u << 20));

    if (start_siblings(0, 1, LOWER_PRIORITY)) {
        before[0] = rounds[0];
        TIMED(1, loop_pair_lha_addi(1u << 18));
        TIMED(3, loop_branch_not_taken(1u << 20));
        put(31, rounds[0] - before[0]);
    }
    join_siblings(0, 1);

    if (start_siblings(1, 2, LOWER_PRIORITY)) {
        before[1] = rounds[1];
        before[2] = rounds[2];
        TIMED(4, loop_indep_addi(1u << 20));
        TIMED(5, loop_dep_add(1u << 20));
        put(32, rounds[1] - before[1]);
        put(33, rounds[2] - before[2]);
    }
    join_siblings(1, 2);

    if (start_siblings(3, 2, MAIN_PRIORITY)) {
        before[3] = rounds[3];
        before[4] = rounds[4];
        TIMED(6, loop_indep_addi(1u << 20));
        TIMED(7, loop_dep_add(1u << 20));
        put(34, rounds[3] - before[3]);
        put(35, rounds[4] - before[4]);
    }
    join_siblings(3, 2);

    put(36, ran);
    put(37, sink);
    put(38, REPS);
    put(39, 0x50505354ull);

    write_frame();
    return 0;
}
