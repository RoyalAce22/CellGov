/* PPU probe: when the vector-scalar instructions younger than an mffs
 * run, and how long an mffs waits for an older vector-scalar result.
 * Each loop runs a fixed body ITER times between two time-base reads.
 * The probe runs each loop REPS times and records the smallest and the
 * largest delta. f14 and v1 start zero. Each loop counts in CTR and
 * closes with bdnz.
 *
 * "li" is a run of li cycling li r3,1 to li r10,1. "fadd" is
 * fadd f1,f14,f14; "4 fadd" writes f1 to f4 from f14. "vaddubm" is
 * vaddubm v4,v1,v1; "4 vaddubm" writes v4 to v7 from v1.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mffs f0
 *   1  mffs f0; fadd
 *   2  mffs f0; 4 fadd
 *   3  mffs f0; vaddubm
 *   4  mffs f0; 4 vaddubm
 *   5  mffs f0; 16 li
 *   6  mffs f0; 32 li
 *   7  mffs f0; fadd; 16 li
 *   8  mffs f0; vaddubm; 16 li
 *   9  fadd; mffs f0
 *  10  fadd. f1,f14,f14; mffs f0
 *  11  vaddubm; mffs f0
 *   word
 *  24  REPS
 *  25  log2 of ITER
 *  26  0x50504659 ("PPFY"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS        12
#define WORDS        (2 * LOOPS + 3)
#define PAYLOAD      (WORDS * 8)
#define REPS         5
#define ITER_LOG2    16

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static unsigned char buf[128] __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

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

#define MFFS    "mffs 0\n"
#define FADD    "fadd 1,14,14\n"
#define FADDR   "fadd. 1,14,14\n"
#define FADD4   "fadd 1,14,14\n" "fadd 2,14,14\n" \
                "fadd 3,14,14\n" "fadd 4,14,14\n"
#define VADD    "vaddubm 4,1,1\n"
#define VADD4   "vaddubm 4,1,1\n" "vaddubm 5,1,1\n" \
                "vaddubm 6,1,1\n" "vaddubm 7,1,1\n"
#define D8      "li 3,1\n" "li 4,1\n" "li 5,1\n" "li 6,1\n" \
                "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n"
#define D16     D8 D8
#define D32     D16 D16

#define MLOOP(name, body)                                              \
    static u64 __attribute__((noinline)) name(u64 n)                   \
    {                                                                  \
        u64 t0 = timebase();                                           \
        __asm__ volatile (                                             \
            "mtctr %[n]\n"                                             \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                     \
            : : [n] "r"(n)                                             \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",         \
              "ctr", "cr1", "fr0", "fr1", "fr2", "fr3", "fr4",         \
              "v4", "v5", "v6", "v7");                                 \
        return timebase() - t0;                                        \
    }

MLOOP(loop_mffs, MFFS)
MLOOP(loop_mffs_fadd, MFFS FADD)
MLOOP(loop_mffs_fadd4, MFFS FADD4)
MLOOP(loop_mffs_vadd, MFFS VADD)
MLOOP(loop_mffs_vadd4, MFFS VADD4)
MLOOP(loop_mffs_16, MFFS D16)
MLOOP(loop_mffs_32, MFFS D32)
MLOOP(loop_mffs_fadd_16, MFFS FADD D16)
MLOOP(loop_mffs_vadd_16, MFFS VADD D16)
MLOOP(loop_fadd_mffs, FADD MFFS)
MLOOP(loop_faddr_mffs, FADDR MFFS)
MLOOP(loop_vadd_mffs, VADD MFFS)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* f14 and v1 start zero. */
    __asm__ volatile (
        "lfd 14,0(%[p])\n" "lvx 1,0,%[p]\n"
        : : [p] "b"(buf) : "fr14", "v1", "memory");

    TIMED(0, loop_mffs(n));
    TIMED(1, loop_mffs_fadd(n));
    TIMED(2, loop_mffs_fadd4(n));
    TIMED(3, loop_mffs_vadd(n));
    TIMED(4, loop_mffs_vadd4(n));
    TIMED(5, loop_mffs_16(n));
    TIMED(6, loop_mffs_32(n));
    TIMED(7, loop_mffs_fadd_16(n));
    TIMED(8, loop_mffs_vadd_16(n));
    TIMED(9, loop_fadd_mffs(n));
    TIMED(10, loop_faddr_mffs(n));
    TIMED(11, loop_vadd_mffs(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x50504659ull);

    write_frame();
    return 0;
}
