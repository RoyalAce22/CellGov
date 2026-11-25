/* PPU probe: where in a 128-byte line an mfvscr, li and store loop starts,
 * and how many stores end it. Each loop runs a fixed body ITER times
 * between two time-base reads. The probe runs each loop REPS times and
 * records the smallest and the largest delta. Every store addresses one
 * 128-byte-aligned buffer the probe zeroes first; r29 holds its address.
 * f14 and r14 start zero. Each loop counts in CTR and closes with bdnz.
 *
 * A loop "@N" starts N bytes past a 128-byte boundary: nops before the
 * loop's label fill the N bytes and run once.
 *
 * "li" cycles li r3,1 to li r10,1. "std" is std r14,0(r29); "2 std" adds
 * std r14,8(r29), "3 std" std r14,16(r29) and "4 std" std r14,24(r29).
 * "stfd" is stfd f14,0(r29).
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mfvscr v3; 16 li; stfd        @0
 *   1  mfvscr v3; 16 li; stfd        @32
 *   2  mfvscr v3; 16 li; stfd        @64
 *   3  mfvscr v3; 16 li; stfd        @96
 *   4  mfvscr v3; 16 li; std         @0
 *   5  mfvscr v3; 16 li; std         @32
 *   6  mfvscr v3; 16 li; std         @64
 *   7  mfvscr v3; 16 li; std         @96
 *   8  mfvscr v3; 16 li; 2 std       @0
 *   9  mfvscr v3; 16 li; 3 std       @0
 *  10  mfvscr v3; 16 li; 4 std       @0
 *  11  mfvscr v3; 16 li; 2 std       @64
 *  12  mfvscr v3; 48 li; std         @0
 *  13  mfvscr v3; 48 li; std         @64
 *  14  mfvscr v3; 48 li; stfd        @0
 *  15  mfvscr v3; 48 li; stfd        @64
 *   word
 *  32  REPS
 *  33  log2 of ITER
 *  34  0x50504d50 ("PPMP"): the probe wrote every word
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

#define MFVSCR  "mfvscr 3\n"
#define STD     "std 14,0(29)\n"
#define STD2    STD "std 14,8(29)\n"
#define STD3    STD2 "std 14,16(29)\n"
#define STD4    STD3 "std 14,24(29)\n"
#define STFD    "stfd 14,0(29)\n"
#define D8      "li 3,1\n" "li 4,1\n" "li 5,1\n" "li 6,1\n" \
                "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n"
#define D16     D8 D8
#define D48     D16 D16 D16

#define N8      "nop\n" "nop\n" "nop\n" "nop\n" "nop\n" "nop\n" "nop\n" "nop\n"
#define AT0     ""
#define AT32    N8
#define AT64    N8 N8
#define AT96    N8 N8 N8

/* A loop whose label sits `pad` past a 128-byte boundary. r29 holds the
 * buffer's address. */
#define MLOOP(name, pad, body)                                         \
    static u64 __attribute__((noinline)) name(u64 n)                   \
    {                                                                  \
        u64 t0 = timebase();                                           \
        __asm__ volatile (                                             \
            "mr 29,%[p]\n" "mtctr %[n]\n"                              \
            ".p2align 7\n" pad "1:\n" body "bdnz 1b\n"                 \
            : : [n] "r"(n), [p] "r"(buf)                               \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",         \
              "r29", "ctr", "memory", "v3");                           \
        return timebase() - t0;                                        \
    }

MLOOP(loop_f_0, AT0, MFVSCR D16 STFD)
MLOOP(loop_f_32, AT32, MFVSCR D16 STFD)
MLOOP(loop_f_64, AT64, MFVSCR D16 STFD)
MLOOP(loop_f_96, AT96, MFVSCR D16 STFD)
MLOOP(loop_d_0, AT0, MFVSCR D16 STD)
MLOOP(loop_d_32, AT32, MFVSCR D16 STD)
MLOOP(loop_d_64, AT64, MFVSCR D16 STD)
MLOOP(loop_d_96, AT96, MFVSCR D16 STD)
MLOOP(loop_d2_0, AT0, MFVSCR D16 STD2)
MLOOP(loop_d3_0, AT0, MFVSCR D16 STD3)
MLOOP(loop_d4_0, AT0, MFVSCR D16 STD4)
MLOOP(loop_d2_64, AT64, MFVSCR D16 STD2)
MLOOP(loop_l_d_0, AT0, MFVSCR D48 STD)
MLOOP(loop_l_d_64, AT64, MFVSCR D48 STD)
MLOOP(loop_l_f_0, AT0, MFVSCR D48 STFD)
MLOOP(loop_l_f_64, AT64, MFVSCR D48 STFD)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* f14 and r14 start zero. */
    __asm__ volatile ("lfd 14,0(%[p])\n" "li 14,0\n"
                      : : [p] "b"(buf) : "fr14", "r14", "memory");

    TIMED(0, loop_f_0(n));
    TIMED(1, loop_f_32(n));
    TIMED(2, loop_f_64(n));
    TIMED(3, loop_f_96(n));
    TIMED(4, loop_d_0(n));
    TIMED(5, loop_d_32(n));
    TIMED(6, loop_d_64(n));
    TIMED(7, loop_d_96(n));
    TIMED(8, loop_d2_0(n));
    TIMED(9, loop_d3_0(n));
    TIMED(10, loop_d4_0(n));
    TIMED(11, loop_d2_64(n));
    TIMED(12, loop_l_d_0(n));
    TIMED(13, loop_l_d_64(n));
    TIMED(14, loop_l_f_0(n));
    TIMED(15, loop_l_f_64(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x50504d50ull);

    write_frame();
    return 0;
}
