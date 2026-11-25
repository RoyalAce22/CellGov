/* PPU probe: what part of a younger instruction's wait after an mfvscr
 * follows the mfvscr's issue and what part its result. Each loop runs a
 * fixed body ITER times between two time-base reads, on a 32-byte
 * boundary. The probe runs each loop REPS times and records the smallest
 * and the largest delta. f14 starts zero. Each loop counts in CTR and
 * closes with bdnz.
 *
 * "li" is a run of li cycling li r3,1 to li r10,1. "fadd" is the
 * independent fadd f1,f14,f14, "vadd" the independent vaddubm v1,v2,v3
 * and "mfvscr2" mfvscr v4. "b" is a taken branch to the next
 * instruction.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mfvscr v0
 *   1  mfvscr v0; fadd
 *   2  mfvscr v0; li; fadd
 *   3  mfvscr v0; 2 li; fadd
 *   4  mfvscr v0; 4 li; fadd
 *   5  mfvscr v0; b; fadd
 *   6  mfvscr v0; vadd
 *   7  mfvscr v0; li; vadd
 *   8  mfvscr v0; 2 li; vadd
 *   9  mfvscr v0; 4 li; vadd
 *  10  mfvscr v0; b; vadd
 *  11  mfvscr v0; mfvscr2
 *  12  mfvscr v0; li; mfvscr2
 *  13  mfvscr v0; 2 li; mfvscr2
 *  14  mfvscr v0; 4 li; mfvscr2
 *  15  mfvscr v0; b; mfvscr2
 *  16  mfvscr v0; b; li; fadd
 *  17  mfvscr v0; 8 li; fadd
 *   word
 *  36  REPS
 *  37  log2 of ITER
 *  38  0x50505647 ("PPVG"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS        18
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

#define MFVSCR  "mfvscr 0\n"
#define MFVSCR2 "mfvscr 4\n"
#define FADD    "fadd 1,14,14\n"
#define VADD    "vaddubm 1,2,3\n"
#define B       "b 2f\n" "2:\n"
#define D1      "li 3,1\n"
#define D2      D1 "li 4,1\n"
#define D4      D2 "li 5,1\n" "li 6,1\n"
#define D8      D4 "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n"

/* A loop whose label sits on a 32-byte boundary. */
#define MLOOP(name, body)                                              \
    static u64 __attribute__((noinline)) name(u64 n)                   \
    {                                                                  \
        u64 t0 = timebase();                                           \
        __asm__ volatile (                                             \
            "mtctr %[n]\n"                                             \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                     \
            : : [n] "r"(n)                                             \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",         \
              "ctr", "fr1", "v0", "v1", "v4");                         \
        return timebase() - t0;                                        \
    }

MLOOP(loop_s, MFVSCR)
MLOOP(loop_s_f, MFVSCR FADD)
MLOOP(loop_s_1_f, MFVSCR D1 FADD)
MLOOP(loop_s_2_f, MFVSCR D2 FADD)
MLOOP(loop_s_4_f, MFVSCR D4 FADD)
MLOOP(loop_s_b_f, MFVSCR B FADD)
MLOOP(loop_s_v, MFVSCR VADD)
MLOOP(loop_s_1_v, MFVSCR D1 VADD)
MLOOP(loop_s_2_v, MFVSCR D2 VADD)
MLOOP(loop_s_4_v, MFVSCR D4 VADD)
MLOOP(loop_s_b_v, MFVSCR B VADD)
MLOOP(loop_s_s, MFVSCR MFVSCR2)
MLOOP(loop_s_1_s, MFVSCR D1 MFVSCR2)
MLOOP(loop_s_2_s, MFVSCR D2 MFVSCR2)
MLOOP(loop_s_4_s, MFVSCR D4 MFVSCR2)
MLOOP(loop_s_b_s, MFVSCR B MFVSCR2)
MLOOP(loop_s_b_1_f, MFVSCR B D1 FADD)
MLOOP(loop_s_8_f, MFVSCR D8 FADD)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* f14 starts zero. */
    __asm__ volatile ("lfd 14,0(%[p])\n"
                      : : [p] "b"(buf) : "fr14", "memory");

    TIMED(0, loop_s(n));
    TIMED(1, loop_s_f(n));
    TIMED(2, loop_s_1_f(n));
    TIMED(3, loop_s_2_f(n));
    TIMED(4, loop_s_4_f(n));
    TIMED(5, loop_s_b_f(n));
    TIMED(6, loop_s_v(n));
    TIMED(7, loop_s_1_v(n));
    TIMED(8, loop_s_2_v(n));
    TIMED(9, loop_s_4_v(n));
    TIMED(10, loop_s_b_v(n));
    TIMED(11, loop_s_s(n));
    TIMED(12, loop_s_1_s(n));
    TIMED(13, loop_s_2_s(n));
    TIMED(14, loop_s_4_s(n));
    TIMED(15, loop_s_b_s(n));
    TIMED(16, loop_s_b_1_f(n));
    TIMED(17, loop_s_8_f(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x50505647ull);

    write_frame();
    return 0;
}
