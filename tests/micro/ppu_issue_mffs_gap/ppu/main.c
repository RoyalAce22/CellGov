/* PPU probe: how the distance between an mffs and an fadd sets the wait
 * between them, in li on either side of the fadd and across a taken
 * branch. Each loop runs a fixed body ITER times between two time-base
 * reads. The probe runs each loop REPS times and records the smallest
 * and the largest delta. f14 starts zero. Each loop counts in CTR and
 * closes with bdnz.
 *
 * "li" is a run of li cycling li r3,1 to li r10,1. "fadd" is
 * fadd f1,f14,f14. "b" is a taken branch to the next instruction.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mffs f0; fadd
 *   1  mffs f0; 1 li; fadd
 *   2  mffs f0; 2 li; fadd
 *   3  mffs f0; 4 li; fadd
 *   4  mffs f0; 8 li; fadd
 *   5  mffs f0; 16 li; fadd
 *   6  fadd; mffs f0
 *   7  1 li; fadd; mffs f0
 *   8  2 li; fadd; mffs f0
 *   9  4 li; fadd; mffs f0
 *  10  8 li; fadd; mffs f0
 *  11  16 li; fadd; mffs f0
 *  12  fadd; 1 li; mffs f0
 *  13  fadd; 4 li; mffs f0
 *  14  fadd; 16 li; mffs f0
 *  15  mffs f0; b; fadd
 *   word
 *  32  REPS
 *  33  log2 of ITER
 *  34  0x5050465a ("PPFZ"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS        16
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
#define B       "b 2f\n" "2:\n"
#define D1      "li 3,1\n"
#define D2      D1 "li 4,1\n"
#define D4      D2 "li 5,1\n" "li 6,1\n"
#define D8      D4 "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n"
#define D16     D8 D8

#define MLOOP(name, body)                                              \
    static u64 __attribute__((noinline)) name(u64 n)                   \
    {                                                                  \
        u64 t0 = timebase();                                           \
        __asm__ volatile (                                             \
            "mtctr %[n]\n"                                             \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                     \
            : : [n] "r"(n)                                             \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",         \
              "ctr", "fr0", "fr1");                                    \
        return timebase() - t0;                                        \
    }

MLOOP(loop_m_f, MFFS FADD)
MLOOP(loop_m_1_f, MFFS D1 FADD)
MLOOP(loop_m_2_f, MFFS D2 FADD)
MLOOP(loop_m_4_f, MFFS D4 FADD)
MLOOP(loop_m_8_f, MFFS D8 FADD)
MLOOP(loop_m_16_f, MFFS D16 FADD)
MLOOP(loop_f_m, FADD MFFS)
MLOOP(loop_1_f_m, D1 FADD MFFS)
MLOOP(loop_2_f_m, D2 FADD MFFS)
MLOOP(loop_4_f_m, D4 FADD MFFS)
MLOOP(loop_8_f_m, D8 FADD MFFS)
MLOOP(loop_16_f_m, D16 FADD MFFS)
MLOOP(loop_f_1_m, FADD D1 MFFS)
MLOOP(loop_f_4_m, FADD D4 MFFS)
MLOOP(loop_f_16_m, FADD D16 MFFS)
MLOOP(loop_m_b_f, MFFS B FADD)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* f14 starts zero. */
    __asm__ volatile ("lfd 14,0(%[p])\n" : : [p] "b"(buf) : "fr14", "memory");

    TIMED(0, loop_m_f(n));
    TIMED(1, loop_m_1_f(n));
    TIMED(2, loop_m_2_f(n));
    TIMED(3, loop_m_4_f(n));
    TIMED(4, loop_m_8_f(n));
    TIMED(5, loop_m_16_f(n));
    TIMED(6, loop_f_m(n));
    TIMED(7, loop_1_f_m(n));
    TIMED(8, loop_2_f_m(n));
    TIMED(9, loop_4_f_m(n));
    TIMED(10, loop_8_f_m(n));
    TIMED(11, loop_16_f_m(n));
    TIMED(12, loop_f_1_m(n));
    TIMED(13, loop_f_4_m(n));
    TIMED(14, loop_f_16_m(n));
    TIMED(15, loop_m_b_f(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x5050465aull);

    write_frame();
    return 0;
}
