/* PPU probe: the PPE's issue timing for the table latencies the PPU
 * guest-time model takes from the handbook rather than from a capture.
 * Each loop runs a fixed body ITER times between two time-base reads.
 * The probe runs each loop REPS times and records the smallest and the
 * largest delta. The loop branch is bdnz, and loop 0 times it alone.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1. The words from 42 on are listed after the
 * loops.
 *   loop
 *   0  empty: bdnz only                              (1 << 20 iterations)
 *   1  8 dependent mulli by 1                        (1 << 18)
 *   2  8 dependent mullw by 1                        (1 << 18)
 *   3  4 dependent divw of 0x7fffffff by 1           (1 << 16)
 *   4  4 dependent divd of 0x7fffffffffffffff by 1   (1 << 16)
 *   5  4 dependent fdiv by 1.0                       (1 << 16)
 *   6  4 dependent fsqrt of 1.0                      (1 << 16)
 *   7  8 dependent fadd. (records CR1)               (1 << 16)
 *   8  8 dependent rlwinm. (a microcoded record form)(1 << 16)
 *   9  8 mfcr                                        (1 << 16)
 *  10  8 mffs                                        (1 << 16)
 *  11  8 mftb                                        (1 << 16)
 *  12  8 dependent vperm                             (1 << 18)
 *  13  8 dependent vadduwm                           (1 << 18)
 *  14  8 dependent vmulouh                           (1 << 18)
 *  15  8 dependent vrefp                             (1 << 18)
 *  16  4 independent lha from one L1 line            (1 << 18)
 *  17  stmw r24 then lmw r24, one L1 line            (1 << 18)
 *  18  4 x (bl to a blr, then b over it)             (1 << 18)
 *  19  8 mtcrf 0xff                                  (1 << 16)
 *  20  4 x (lfd from one L1 line, dependent fadd)    (1 << 18)
 *   word
 *  42  the sum of every loop's result register (keeps each loop live)
 *  43  REPS
 *  44  0x50505442 ("PPTB"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS   45
#define PAYLOAD (WORDS * 8)
#define REPS    5

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 one_line[16] __attribute__((aligned(128)));
static volatile u64 sink;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))
#define R4(x) x x x x
#define R8(x) R4(x) R4(x)

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

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

static u64 __attribute__((noinline)) loop_empty(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (LOOP_BEGIN LOOP_END : : [n] "r"(n) : "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_mulli(u64 n)
{
    u64 a = 7;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("mulli %[a],%[a],1\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "ctr");
    sink += a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_mullw(u64 n)
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

static u64 __attribute__((noinline)) loop_dep_divw(u64 n)
{
    u64 a = 0x7fffffffull;
    u64 t0 = timebase();
    __asm__ volatile (
        "li 15,1\n"
        LOOP_BEGIN R4("divw %[a],%[a],15\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "r15", "ctr");
    sink += a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_divd(u64 n)
{
    u64 a = 0x7fffffffffffffffull;
    u64 t0 = timebase();
    __asm__ volatile (
        "li 15,1\n"
        LOOP_BEGIN R4("divd %[a],%[a],15\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "r15", "ctr");
    sink += a;
    return timebase() - t0;
}

/* The FP loops start every register they use from a normal value: an
 * operand left over from earlier code can be a denormal or a NaN, which
 * takes a slower path. */
static u64 __attribute__((noinline)) loop_dep_fdiv(u64 n)
{
    double a = 1.0, b = 1.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R4("fdiv %[a],%[a],%[b]\n") LOOP_END
        : [a] "+d"(a) : [b] "d"(b), [n] "r"(n) : "ctr");
    sink += (u64)a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_fsqrt(u64 n)
{
    double a = 1.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R4("fsqrt %[a],%[a]\n") LOOP_END
        : [a] "+d"(a) : [n] "r"(n) : "ctr");
    sink += (u64)a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_fadd_record(u64 n)
{
    double a = 1.0, b = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("fadd. %[a],%[a],%[b]\n") LOOP_END
        : [a] "+d"(a) : [b] "d"(b), [n] "r"(n) : "cr1", "ctr");
    sink += (u64)a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_rlwinm_record(u64 n)
{
    u64 a = 5;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("rlwinm. %[a],%[a],0,0,31\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "cr0", "ctr");
    sink += a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_mfcr(u64 n)
{
    u64 a = 0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("mfcr %[a]\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "ctr");
    sink += a;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_mffs(u64 n)
{
    double a = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("mffs %[a]\n") LOOP_END
        : [a] "+d"(a) : [n] "r"(n) : "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_mftb(u64 n)
{
    u64 a = 0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("mftb %[a]\n") LOOP_END
        : [a] "+r"(a) : [n] "r"(n) : "ctr");
    sink += a & 1;
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_vperm(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        "vxor 2,2,2\n" "vxor 3,3,3\n"
        LOOP_BEGIN R8("vperm 2,2,2,3\n") LOOP_END
        : : [n] "r"(n) : "v2", "v3", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_vadduwm(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        "vxor 2,2,2\n" "vxor 3,3,3\n"
        LOOP_BEGIN R8("vadduwm 2,2,3\n") LOOP_END
        : : [n] "r"(n) : "v2", "v3", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_dep_vmulouh(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        "vxor 2,2,2\n" "vxor 3,3,3\n"
        LOOP_BEGIN R8("vmulouh 2,2,3\n") LOOP_END
        : : [n] "r"(n) : "v2", "v3", "ctr");
    return timebase() - t0;
}

/* vrefp of 1.0, splatted from an integer 1, so the operand is normal. */
static u64 __attribute__((noinline)) loop_dep_vrefp(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        "vspltisw 2,1\n" "vcfsx 2,2,0\n"
        LOOP_BEGIN R8("vrefp 2,2\n") LOOP_END
        : : [n] "r"(n) : "v2", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_lha(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        "lha 14,0(%[p])\n" "lha 15,8(%[p])\n" "lha 16,16(%[p])\n" "lha 17,24(%[p])\n"
        LOOP_END
        : : [n] "r"(n), [p] "b"(EA(one_line))
        : "r14", "r15", "r16", "r17", "ctr");
    return timebase() - t0;
}

/* r24 to r31 are callee-saved, so the asm saves and restores them
 * around the loop through the line it stores to. */
static u64 __attribute__((noinline)) loop_stmw_lmw(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        "stmw 24,0(%[p])\n" "lmw 24,0(%[p])\n"
        LOOP_END
        : : [n] "r"(n), [p] "b"(EA(one_line))
        : "ctr", "memory");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_bl_blr(u64 n)
{
    u64 t0 = timebase();
    __asm__ volatile (
        "mflr 14\n"
        LOOP_BEGIN
        R4("bl 2f\n" "b 3f\n" "2:\n" "blr\n" "3:\n")
        LOOP_END
        "mtlr 14\n"
        : : [n] "r"(n) : "r14", "lr", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_mtcrf(u64 n)
{
    u64 v = 0x22222222ull;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN R8("mtcrf 0xff,%[v]\n") LOOP_END
        : : [v] "r"(v), [n] "r"(n)
        : "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", "cr6", "cr7", "ctr");
    return timebase() - t0;
}

static u64 __attribute__((noinline)) loop_lfd_fadd(u64 n)
{
    double zero = 0.0;
    u64 t0 = timebase();
    __asm__ volatile (
        "fmr 12,%[z]\n"
        LOOP_BEGIN
        R4("lfd 1,64(%[p])\n" "fadd 1,1,12\n")
        LOOP_END
        : : [n] "r"(n), [p] "b"(EA(one_line)), [z] "d"(zero)
        : "fr1", "fr12", "ctr");
    return timebase() - t0;
}

int main(void)
{
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    /* one_line holds 1.0 in every doubleword. The stmw loop overwrites
     * the first 32 bytes, so the lfd loop reads from byte 64. */
    for (i = 0; i < 16; i++)
        one_line[i] = 0x3ff0000000000000ull;

    TIMED(0, loop_empty(1u << 20));
    TIMED(1, loop_dep_mulli(1u << 18));
    TIMED(2, loop_dep_mullw(1u << 18));
    TIMED(3, loop_dep_divw(1u << 16));
    TIMED(4, loop_dep_divd(1u << 16));
    TIMED(5, loop_dep_fdiv(1u << 16));
    TIMED(6, loop_dep_fsqrt(1u << 16));
    TIMED(7, loop_dep_fadd_record(1u << 16));
    TIMED(8, loop_dep_rlwinm_record(1u << 16));
    TIMED(9, loop_mfcr(1u << 16));
    TIMED(10, loop_mffs(1u << 16));
    TIMED(11, loop_mftb(1u << 16));
    TIMED(12, loop_dep_vperm(1u << 18));
    TIMED(13, loop_dep_vadduwm(1u << 18));
    TIMED(14, loop_dep_vmulouh(1u << 18));
    TIMED(15, loop_dep_vrefp(1u << 18));
    TIMED(16, loop_lha(1u << 18));
    TIMED(17, loop_stmw_lmw(1u << 18));
    TIMED(18, loop_bl_blr(1u << 18));
    TIMED(19, loop_mtcrf(1u << 16));
    TIMED(20, loop_lfd_fadd(1u << 18));
    put(42, sink);
    put(43, REPS);
    put(44, 0x50505442ull);

    write_frame();
    return 0;
}
