/* PPU probe: what the PPU does when stores fill its store queue, and how
 * many stores or 16-byte entries the queue holds. Each loop runs a fixed
 * body ITER times between two time-base reads. The probe runs each loop
 * REPS times and records the smallest and the largest delta. The loop
 * branch is bdnz.
 *
 * Lines A to F are six 128-byte lines in a row, A first; line H is the
 * twelfth line, which no store writes. Each loop ends with an ld of r9,
 * and the next iteration's first store stores r9, so the iteration waits
 * for the load.
 *
 * Loops 0 to 4 run 1, 2, 3, 4 and 6 back-to-back stmw r0 (128 bytes, 8
 * entries of 16 bytes each) to lines A, B, C and on, then an ld of the
 * first doubleword of A, which the first stmw stored. Loops 5 to 9 run
 * the same stores and load line H.
 *
 * Loops 10 to 16 run an std of r9 to A and then std of r10 to other
 * 16-byte blocks, one std each, from offset 128 in steps of 16, then an
 * ld of A or of H.
 *
 * Loops 17 to 20 run an std of r9 to A and then stmw r28 (16 bytes, one
 * entry) or stmw r24 (32 bytes, two entries) to other blocks from offset
 * 128, then an ld of A or of H. Loop 17 and loop 11 both run 16 stores
 * of one entry each; loop 19 runs 9 stores and 17 entries.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1. The words from 42 on are listed after the
 * loops. Every loop runs 1 << 16 iterations.
 *   loop  stores                                load
 *   0     stmw r0 A                             A
 *   1     stmw r0 A, B                          A
 *   2     stmw r0 A, B, C                       A
 *   3     stmw r0 A, B, C, D                    A
 *   4     stmw r0 A, B, C, D, E, F              A
 *   5     stmw r0 A                             H
 *   6     stmw r0 A, B                          H
 *   7     stmw r0 A, B, C                       H
 *   8     stmw r0 A, B, C, D                    H
 *   9     stmw r0 A, B, C, D, E, F              H
 *  10     std A, 7 std                          A
 *  11     std A, 15 std                         A
 *  12     std A, 16 std                         A
 *  13     std A, 23 std                         A
 *  14     std A, 31 std                         A
 *  15     std A, 15 std                         H
 *  16     std A, 31 std                         H
 *  17     std A, 15 stmw r28                    A
 *  18     std A, 16 stmw r28                    A
 *  19     std A, 8 stmw r24                     A
 *  20     std A, 8 stmw r24                     H
 *   word
 *  42  the sum of the loaded r9 values (keeps them live)
 *  43  REPS
 *  44  0x50535146 ("PSQF"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS   21
#define WORDS   (2 * LOOPS + 3)
#define PAYLOAD (WORDS * 8)
#define REPS    11
#define ITER    (1u << 16)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 lines[192] __attribute__((aligned(128)));
static volatile u64 sink;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

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

/* stmw r0 to line A to F: 32 words, 128 bytes. */
#define STMW_A "stmw 0,0(%[p])\n"
#define STMW_B "stmw 0,128(%[p])\n"
#define STMW_C "stmw 0,256(%[p])\n"
#define STMW_D "stmw 0,384(%[p])\n"
#define STMW_E "stmw 0,512(%[p])\n"
#define STMW_F "stmw 0,640(%[p])\n"
#define STMW_AB   STMW_A STMW_B
#define STMW_ABC  STMW_AB STMW_C
#define STMW_ABCD STMW_ABC STMW_D
#define STMW_A_F  STMW_ABCD STMW_E STMW_F

#define STD_A "std 9,0(%[p])\n"
#define LD_A  "ld 9,0(%[p])\n"
#define LD_H  "ld 9,1408(%[p])\n"

/* std of r10 to the 16-byte block at offset `o`. */
#define STDX(o) "std 10," #o "(%[p])\n"
/* The blocks from offset 128 on: 7, 15, 16, 23 and 31 of them. */
#define STD_7                                                      \
    STDX(128) STDX(144) STDX(160) STDX(176) STDX(192) STDX(208)    \
    STDX(224)
#define STD_15                                                     \
    STD_7 STDX(240) STDX(256) STDX(272) STDX(288) STDX(304)        \
    STDX(320) STDX(336) STDX(352)
#define STD_16 STD_15 STDX(368)
#define STD_23                                                     \
    STD_16 STDX(384) STDX(400) STDX(416) STDX(432) STDX(448)       \
    STDX(464) STDX(480)
#define STD_31                                                     \
    STD_23 STDX(496) STDX(512) STDX(528) STDX(544) STDX(560)       \
    STDX(576) STDX(592) STDX(608)

/* stmw r28 (16 bytes) to the block at offset `o`. */
#define STMW28(o) "stmw 28," #o "(%[p])\n"
#define STMW28_15                                                  \
    STMW28(128) STMW28(144) STMW28(160) STMW28(176) STMW28(192)    \
    STMW28(208) STMW28(224) STMW28(240) STMW28(256) STMW28(272)    \
    STMW28(288) STMW28(304) STMW28(320) STMW28(336) STMW28(352)
#define STMW28_16 STMW28_15 STMW28(368)

/* stmw r24 (32 bytes) to the two blocks at offset `o`. */
#define STMW24(o) "stmw 24," #o "(%[p])\n"
#define STMW24_8                                                   \
    STMW24(128) STMW24(160) STMW24(192) STMW24(224) STMW24(256)    \
    STMW24(288) STMW24(320) STMW24(352)

/* One timed loop of `body` with `p` line A, r9 the chained doubleword and
 * r10 the value the other stores write. Returns the delta; adds r9 to the
 * sink. */
#define LOOP(name, body)                                           \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        register u64 v9 __asm__("9") = 1;                          \
        register u64 v10 __asm__("10") = 2;                        \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : "+r"(v9), "+r"(v10)                                  \
            : [n] "r"(n), [p] "b"(EA(&lines[0]))                   \
            : "ctr", "memory");                                    \
        u64 t1 = timebase();                                       \
        sink += v9 + v10;                                          \
        return t1 - t0;                                            \
    }

LOOP(loop_stmw_1_a, STMW_A LD_A)
LOOP(loop_stmw_2_a, STMW_AB LD_A)
LOOP(loop_stmw_3_a, STMW_ABC LD_A)
LOOP(loop_stmw_4_a, STMW_ABCD LD_A)
LOOP(loop_stmw_6_a, STMW_A_F LD_A)
LOOP(loop_stmw_1_h, STMW_A LD_H)
LOOP(loop_stmw_2_h, STMW_AB LD_H)
LOOP(loop_stmw_3_h, STMW_ABC LD_H)
LOOP(loop_stmw_4_h, STMW_ABCD LD_H)
LOOP(loop_stmw_6_h, STMW_A_F LD_H)
LOOP(loop_std_8_a, STD_A STD_7 LD_A)
LOOP(loop_std_16_a, STD_A STD_15 LD_A)
LOOP(loop_std_17_a, STD_A STD_16 LD_A)
LOOP(loop_std_24_a, STD_A STD_23 LD_A)
LOOP(loop_std_32_a, STD_A STD_31 LD_A)
LOOP(loop_std_16_h, STD_A STD_15 LD_H)
LOOP(loop_std_32_h, STD_A STD_31 LD_H)
LOOP(loop_stmw28_15_a, STD_A STMW28_15 LD_A)
LOOP(loop_stmw28_16_a, STD_A STMW28_16 LD_A)
LOOP(loop_stmw24_8_a, STD_A STMW24_8 LD_A)
LOOP(loop_stmw24_8_h, STD_A STMW24_8 LD_H)

int main(void)
{
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 192; i++)
        lines[i] = i;

    TIMED(0, loop_stmw_1_a(ITER));
    TIMED(1, loop_stmw_2_a(ITER));
    TIMED(2, loop_stmw_3_a(ITER));
    TIMED(3, loop_stmw_4_a(ITER));
    TIMED(4, loop_stmw_6_a(ITER));
    TIMED(5, loop_stmw_1_h(ITER));
    TIMED(6, loop_stmw_2_h(ITER));
    TIMED(7, loop_stmw_3_h(ITER));
    TIMED(8, loop_stmw_4_h(ITER));
    TIMED(9, loop_stmw_6_h(ITER));
    TIMED(10, loop_std_8_a(ITER));
    TIMED(11, loop_std_16_a(ITER));
    TIMED(12, loop_std_17_a(ITER));
    TIMED(13, loop_std_24_a(ITER));
    TIMED(14, loop_std_32_a(ITER));
    TIMED(15, loop_std_16_h(ITER));
    TIMED(16, loop_std_32_h(ITER));
    TIMED(17, loop_stmw28_15_a(ITER));
    TIMED(18, loop_stmw28_16_a(ITER));
    TIMED(19, loop_stmw24_8_a(ITER));
    TIMED(20, loop_stmw24_8_h(ITER));
    put(42, sink);
    put(43, REPS);
    put(44, 0x50535146ull);

    write_frame();
    return 0;
}
