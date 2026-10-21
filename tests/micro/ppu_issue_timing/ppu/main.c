/* PPU probe: the PPE's issue timing, one timed loop per term of the
 * PPU guest-time model. Each loop runs a fixed body ITER times between
 * two time-base reads. The probe runs each loop REPS times and records
 * the smallest and the largest delta. The loop branch is bdnz, and
 * loop 0 times it alone.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1. The words from 46 on are listed after the
 * loops.
 *   loop
 *   0  empty: bdnz only                      (1 << 20 iterations)
 *   1  8 independent addi, one per register  (1 << 20)
 *   2  8 dependent add                        (1 << 20)
 *   3  8 dependent mulld                      (1 << 18)
 *   4  4 dependent divd                       (1 << 16)
 *   5  8 dependent fadd                       (1 << 18)
 *   6  8 dependent fmadd                      (1 << 18)
 *   7  8 dependent vaddfp                     (1 << 18)
 *   8  4 x (addi, ld from one L1 line), all independent   (1 << 20)
 *   9  4 x (addi, fadd), all independent      (1 << 20)
 *  10  8 independent fadd                     (1 << 20)
 *  11  8 dependent ld, 128-byte stride ring of 16 KB      (1 << 17)
 *  12  8 dependent ld, shuffled ring of 256 KB            (1 << 15)
 *  13  8 dependent ld, shuffled ring of 8 MB              (1 << 12)
 *  14  4 x (std, ld) to one doubleword, dependent         (1 << 18)
 *  15  8 x (cmpdi, bne not taken)             (1 << 20)
 *  16  8 x b to the next instruction          (1 << 20)
 *  17  LCG step, then a branch that is always taken       (1 << 20)
 *  18  LCG step, then a branch on the LCG's top bit        (1 << 20)
 *  19  sc 147 sys_time_get_timebase_frequency (1 << 12 calls)
 *  20  loop 1 again, a sibling thread busy on independent addi
 *  21  loop 2 again, the sibling busy
 *  22  loop 11 again, the sibling busy
 *   word
 *  46  the time-base frequency sc 147 returned
 *  47  sc 52 thread create rc
 *  48  time-base ticks until the sibling reported it runs (all ones: it never did)
 *  49  sibling rounds (512 addi each) across loops 20 to 22
 *  50  sc 44 join rc
 *  51  the sibling's exit value (expected 0x5157)
 *  52  loop 18's count of branches that fell through, in its last run
 *  53  the sum of every loop's result register (keeps each loop live)
 *  54  REPS
 *  55  0x50504954 ("PPIT"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT             41
#define SYS_PPU_THREAD_JOIN             44
#define SYS_TIME_GET_TIMEBASE_FREQUENCY 147

#define WORDS   56
#define PAYLOAD (WORDS * 8)
#define REPS    5

#define LINE 128
#define L1_RING_BYTES  (16u * 1024u)
#define L2_RING_BYTES  (256u * 1024u)
#define MEM_RING_BYTES (8u * 1024u * 1024u)

/* The sibling stops at the stop flag, or 10 s of time base after it
 * starts, whichever comes first. */
#define SIBLING_LIMIT_TB (79800000ull * 10ull)
/* The main thread waits at most 1 s for the sibling to start. */
#define START_WAIT_TB 79800000ull

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static unsigned char l1_ring[L1_RING_BYTES] __attribute__((aligned(128)));
static unsigned char l2_ring[L2_RING_BYTES] __attribute__((aligned(128)));
static unsigned char mem_ring[MEM_RING_BYTES] __attribute__((aligned(128)));
static unsigned int order[MEM_RING_BYTES / LINE];
static volatile u64 one_line[16] __attribute__((aligned(128)));
static volatile u64 sink;

static volatile unsigned int sibling_started;
static volatile unsigned int sibling_stop;
static volatile u64 sibling_rounds;

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

/* The sibling: independent addi until the stop flag or its time limit. */
static void sibling_entry(void *arg)
{
    u64 start = timebase();
    (void)arg;
    sibling_started = 1;
    while (!sibling_stop && timebase() - start < SIBLING_LIMIT_TB) {
        __asm__ volatile (
            "li 14,32\n"
            "mtctr 14\n"
            "1:\n"
            R8("addi 15,15,1\n" "addi 16,16,1\n")
            "bdnz 1b\n"
            : : : "r14", "r15", "r16", "ctr");
        sibling_rounds = sibling_rounds + 1;
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5157);
    for (;;) { }
}

/* Link `bytes` of `ring` into a cycle of one doubleword per 128-byte
 * line. `shuffle` visits the lines in an LCG-shuffled order, so a
 * stride prefetch cannot follow it. Returns the address of the first
 * node. */
static u64 build_ring(unsigned char *ring, unsigned bytes, int shuffle)
{
    unsigned n = bytes / LINE;
    unsigned i;
    u64 seed = 0x2545F4914F6CDD1Dull;
    for (i = 0; i < n; i++)
        order[i] = i;
    if (shuffle) {
        for (i = n - 1; i > 0; i--) {
            unsigned j, t;
            seed = seed * 6364136223846793005ull + 1442695040888963407ull;
            j = (unsigned)((seed >> 33) % (i + 1));
            t = order[i];
            order[i] = order[j];
            order[j] = t;
        }
    }
    for (i = 0; i < n; i++) {
        u64 *node = (u64 *)(ring + order[i] * LINE);
        *node = EA(ring + order[(i + 1) % n] * LINE);
    }
    return EA(ring + order[0] * LINE);
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

static u64 __attribute__((noinline)) loop_empty(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (LOOP_BEGIN LOOP_END : : [n] "r"(n) : "ctr");
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

static u64 __attribute__((noinline)) loop_dep_mulld(u64 n)
{
    u64 a = 7;
    u64 t0 = timebase();
    __asm__ volatile (
        "li 15,1\n"
        LOOP_BEGIN R8("mulld %[a],%[a],15\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "r15", "ctr");
    sink += a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_divd(u64 n)
{
    u64 a = 0x123456789ull;
    u64 t0 = timebase();
    __asm__ volatile (
        "li 15,1\n"
        LOOP_BEGIN R4("divd %[a],%[a],15\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "r15", "ctr");
    sink += a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_fadd(u64 n)
{
    double a = 1.0, b = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("fadd %[a],%[a],%[b]\n") LOOP_END
        : [a] "+d"(a) : [b] "d"(b), [n] "r"(n) : "ctr");
    sink += (u64)a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_fmadd(u64 n)
{
    double a = 1.0, b = 1.0, c = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("fmadd %[a],%[a],%[b],%[c]\n") LOOP_END
        : [a] "+d"(a) : [b] "d"(b), [c] "d"(c), [n] "r"(n) : "ctr");
    sink += (u64)a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_vaddfp(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        "vxor 2,2,2\n" "vxor 3,3,3\n"
        LOOP_BEGIN R8("vaddfp 2,2,3\n") LOOP_END
        : : [n] "r"(n) : "v2", "v3", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_pair_addi_ld(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        "addi 14,14,1\n" "ld 18,0(%[p])\n" "addi 15,15,1\n" "ld 19,8(%[p])\n"
        "addi 16,16,1\n" "ld 20,16(%[p])\n" "addi 17,17,1\n" "ld 21,24(%[p])\n"
        LOOP_END
        : : [n] "r"(n), [p] "b"(EA(one_line))
        : "r14", "r15", "r16", "r17", "r18", "r19", "r20", "r21", "ctr");
    return timebase() - t0;
}

/* The FP loops start every register they use from a normal value: an
 * operand left over from earlier code can be a denormal or a NaN, which
 * takes a slower path. */
static u64 __attribute__((noinline)) loop_pair_addi_fadd(u64 n)
{
    double one = 1.0, zero = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        "fmr 1,%[o]\n" "fmr 2,%[o]\n" "fmr 3,%[o]\n" "fmr 4,%[o]\n" "fmr 5,%[z]\n"
        LOOP_BEGIN
        "addi 14,14,1\n" "fadd 1,1,5\n" "addi 15,15,1\n" "fadd 2,2,5\n"
        "addi 16,16,1\n" "fadd 3,3,5\n" "addi 17,17,1\n" "fadd 4,4,5\n"
        LOOP_END
        : : [n] "r"(n), [o] "d"(one), [z] "d"(zero)
        : "r14", "r15", "r16", "r17", "fr1", "fr2", "fr3", "fr4", "fr5", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_indep_fadd(u64 n)
{
    double one = 1.0, zero = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        "fmr 1,%[o]\n" "fmr 2,%[o]\n" "fmr 3,%[o]\n" "fmr 4,%[o]\n"
        "fmr 5,%[o]\n" "fmr 6,%[o]\n" "fmr 7,%[o]\n" "fmr 8,%[o]\n" "fmr 9,%[z]\n"
        LOOP_BEGIN
        "fadd 1,1,9\n" "fadd 2,2,9\n" "fadd 3,3,9\n" "fadd 4,4,9\n"
        "fadd 5,5,9\n" "fadd 6,6,9\n" "fadd 7,7,9\n" "fadd 8,8,9\n"
        LOOP_END
        : : [n] "r"(n), [o] "d"(one), [z] "d"(zero)
        : "fr1", "fr2", "fr3", "fr4", "fr5", "fr6", "fr7", "fr8", "fr9", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_chase(u64 n, u64 head)
{
    u64 p = head;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("ld %[p],0(%[p])\n") LOOP_END
        : [p] "+b"(p) : [n] "r"(n) : "ctr");
    sink += p;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_load_hit_store(u64 n)
{
    u64 v = 5;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        R4("std %[v],0(%[p])\n" "ld %[v],0(%[p])\n")
        LOOP_END
        : [v] "+r"(v) : [n] "r"(n), [p] "b"(EA(one_line)) : "ctr", "memory");
    sink += v;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_branch_not_taken(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        "li 14,0\n"
        LOOP_BEGIN R8("cmpdi 14,0\n" "bne 2f\n" "2:\n") LOOP_END
        : : [n] "r"(n) : "r14", "cr0", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_branch_taken(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("b 2f\n" "2:\n") LOOP_END
        : : [n] "r"(n) : "ctr");
    return timebase() - t0;
}

/* One LCG step a round, then a conditional branch over an addi. With
 * `random` zero the branch tests a register that stays zero, so it
 * always falls through; otherwise it tests the LCG's top bit. r19
 * counts the fall-throughs. */
static u64 __attribute__((noinline)) loop_lcg_branch(u64 n, u64 random, u64 *fell)
{
    u64 count = 0;
    u64 t0 = timebase();
    __asm__ volatile (
        "lis 16,0x5851\n" "ori 16,16,0xf42d\n"
        "li 14,1\n"
        LOOP_BEGIN
        "mulld 14,14,16\n"
        "addi 14,14,1\n"
        "rldicl 17,14,1,63\n"
        "and 17,17,%[r]\n"
        "cmpdi 17,0\n"
        "beq 2f\n"
        "addi %[c],%[c],1\n"
        "2:\n"
        LOOP_END
        : [c] "+r"(count) : [n] "r"(n), [r] "r"(random)
        : "r14", "r16", "r17", "cr0", "ctr");
    *fell = count;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_syscall(u64 n, u64 *freq)
{
    u64 i;
    u64 f = 0;
    u64 t0 = timebase();
    for (i = 0; i < n; i++)
        f = (u64)cgov_sc(SYS_TIME_GET_TIMEBASE_FREQUENCY, 0, 0, 0, 0, 0, 0, 0, 0);
    *freq = f;
    return timebase() - t0;
}

int main(void)
{
    unsigned i;
    u64 l1_head, l2_head, mem_head;
    u64 freq = 0, fell = 0, scratch = 0;
    unsigned long long tid = 0, retval = 0;
    s32 rc;
    u64 waited = ~0ull;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    l1_head = build_ring(l1_ring, L1_RING_BYTES, 0);
    l2_head = build_ring(l2_ring, L2_RING_BYTES, 1);
    mem_head = build_ring(mem_ring, MEM_RING_BYTES, 1);
    /* Warm the two cached rings. */
    (void)loop_chase(L1_RING_BYTES / LINE, l1_head);
    (void)loop_chase(L2_RING_BYTES / LINE, l2_head);

    TIMED(0, loop_empty(1u << 20));
    TIMED(1, loop_indep_addi(1u << 20));
    TIMED(2, loop_dep_add(1u << 20));
    TIMED(3, loop_dep_mulld(1u << 18));
    TIMED(4, loop_dep_divd(1u << 16));
    TIMED(5, loop_dep_fadd(1u << 18));
    TIMED(6, loop_dep_fmadd(1u << 18));
    TIMED(7, loop_dep_vaddfp(1u << 18));
    TIMED(8, loop_pair_addi_ld(1u << 20));
    TIMED(9, loop_pair_addi_fadd(1u << 20));
    TIMED(10, loop_indep_fadd(1u << 20));
    TIMED(11, loop_chase(1u << 17, l1_head));
    TIMED(12, loop_chase(1u << 15, l2_head));
    TIMED(13, loop_chase(1u << 12, mem_head));
    TIMED(14, loop_load_hit_store(1u << 18));
    TIMED(15, loop_branch_not_taken(1u << 20));
    TIMED(16, loop_branch_taken(1u << 20));
    TIMED(17, loop_lcg_branch(1u << 20, 0, &scratch));
    TIMED(18, loop_lcg_branch(1u << 20, 1, &fell));
    TIMED(19, loop_syscall(1u << 12, &freq));
    put(46, freq);
    put(52, fell);

    sibling_started = 0;
    sibling_stop = 0;
    rc = cgov_ppu_thread_create(
        &tid,
        make_thread_param(&sibling_param, (const void *)&sibling_entry),
        0, 0, 2000, 0x4000, THREAD_JOINABLE, 0);
    put(47, (u64)(s64)rc);
    if (rc == 0) {
        u64 t0 = timebase();
        while (!sibling_started && timebase() - t0 < START_WAIT_TB) {
        }
        if (sibling_started) {
            u64 before;
            waited = timebase() - t0;
            before = sibling_rounds;
            TIMED(20, loop_indep_addi(1u << 20));
            TIMED(21, loop_dep_add(1u << 20));
            TIMED(22, loop_chase(1u << 17, l1_head));
            put(49, sibling_rounds - before);
        }
        sibling_stop = 1;
        put(50, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(51, retval);
    }
    put(48, waited);
    put(53, sink);
    put(54, REPS);
    put(55, 0x50504954ull);

    write_frame();
    return 0;
}
