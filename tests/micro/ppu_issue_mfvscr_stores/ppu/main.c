/* PPU probe: how the count and the addresses of the stores at the end of
 * an mfvscr's held work set the next mfvscr's wait. Each loop runs a
 * fixed body ITER times between two time-base reads. The probe runs each
 * loop REPS times and records the smallest and the largest delta. Every
 * store addresses one 256-byte buffer, 128-byte aligned, that the probe
 * zeroes first; r29 holds its address. r14 starts zero. Each loop starts
 * on a 128-byte boundary, counts in CTR and closes with bdnz.
 *
 * Every loop is mfvscr v3; 16 li; then the stores listed. "li" cycles
 * li r3,1 to li r10,1. "std N" is std r14,N(r29); "stw N" is
 * stw r14,N(r29).
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  std 0
 *   1  std 0; std 8
 *   2  std 0; std 16
 *   3  std 0; std 64
 *   4  std 0; std 128
 *   5  std 0; std 8; std 16
 *   6  std 0 to std 24, 4 stores
 *   7  std 0 to std 32, 5 stores
 *   8  std 0 to std 40, 6 stores
 *   9  std 0 to std 56, 8 stores
 *  10  std 0; std 0
 *  11  std 0, four times
 *  12  stw 0; stw 4
 *  13  std 0; li; std 8
 *  14  std 0; 4 li; std 8
 *  15  std 0; std 8; 4 li
 *   word
 *  32  REPS
 *  33  log2 of ITER
 *  34  0x50504d53 ("PPMS"): the probe wrote every word
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
static unsigned char buf[256] __attribute__((aligned(128)));

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

#define MFVSCR  "mfvscr 3\n"
#define S(n)    "std 14," #n "(29)\n"
#define W(n)    "stw 14," #n "(29)\n"
#define L1      "li 3,1\n"
#define L4      "li 3,1\n" "li 4,1\n" "li 5,1\n" "li 6,1\n"
#define L16     L4 "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n" \
                L4 "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n"

/* A loop on a 128-byte boundary. r29 holds the buffer's address. */
#define MLOOP(name, stores)                                            \
    static u64 __attribute__((noinline)) name(u64 n)                   \
    {                                                                  \
        u64 t0 = timebase();                                           \
        __asm__ volatile (                                             \
            "mr 29,%[p]\n" "mtctr %[n]\n"                              \
            ".p2align 7\n" "1:\n" MFVSCR L16 stores "bdnz 1b\n"        \
            : : [n] "r"(n), [p] "r"(buf)                               \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",         \
              "r29", "ctr", "memory", "v3");                           \
        return timebase() - t0;                                        \
    }

MLOOP(loop_0, S(0))
MLOOP(loop_1, S(0) S(8))
MLOOP(loop_2, S(0) S(16))
MLOOP(loop_3, S(0) S(64))
MLOOP(loop_4, S(0) S(128))
MLOOP(loop_5, S(0) S(8) S(16))
MLOOP(loop_6, S(0) S(8) S(16) S(24))
MLOOP(loop_7, S(0) S(8) S(16) S(24) S(32))
MLOOP(loop_8, S(0) S(8) S(16) S(24) S(32) S(40))
MLOOP(loop_9, S(0) S(8) S(16) S(24) S(32) S(40) S(48) S(56))
MLOOP(loop_10, S(0) S(0))
MLOOP(loop_11, S(0) S(0) S(0) S(0))
MLOOP(loop_12, W(0) W(4))
MLOOP(loop_13, S(0) L1 S(8))
MLOOP(loop_14, S(0) L4 S(8))
MLOOP(loop_15, S(0) S(8) L4)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* r14 starts zero. */
    __asm__ volatile ("li 14,0\n" : : : "r14");

    TIMED(0, loop_0(n));
    TIMED(1, loop_1(n));
    TIMED(2, loop_2(n));
    TIMED(3, loop_3(n));
    TIMED(4, loop_4(n));
    TIMED(5, loop_5(n));
    TIMED(6, loop_6(n));
    TIMED(7, loop_7(n));
    TIMED(8, loop_8(n));
    TIMED(9, loop_9(n));
    TIMED(10, loop_10(n));
    TIMED(11, loop_11(n));
    TIMED(12, loop_12(n));
    TIMED(13, loop_13(n));
    TIMED(14, loop_14(n));
    TIMED(15, loop_15(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x50504d53ull);

    write_frame();
    return 0;
}
