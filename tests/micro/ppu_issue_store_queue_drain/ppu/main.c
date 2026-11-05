/* PPU probe: how fast the store queue drains, and what a store that finds
 * it full costs. Each loop runs a fixed body ITER times between two
 * time-base reads. The probe runs each loop REPS times and records the
 * smallest and the largest delta. The loop branch is bdnz.
 *
 * Every loop ends with an ld of r9 from line H, which no store writes,
 * and starts with an std of r9 to line A, so the iteration waits for the
 * load and no load waits for a store. The other stores are std of r10.
 *
 * Loops 0 to 9 run the std to A and then N - 1 std to the 16-byte blocks
 * from offset 128 in steps of 16, for N = 16, 18, 20, 22, 24, 26, 28, 30,
 * 32 and 64. Loops 10 to 12 run 32 stores to other addresses: the blocks
 * of a 16-store loop twice over, steps of 8 (two stores a block), and
 * steps of 128 (one store a line). Loops 13 to 15 put one or three
 * independent addi after each std, which space the stores out without
 * changing them. Loop 16 runs the 96 addi of loop 14 alone.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1. The words from 34 on are listed after the
 * loops. Every loop runs 1 << 15 iterations.
 *   loop  stores                                         spacing
 *   0     std A, 15 std, steps of 16                     none
 *   1     std A, 17 std, steps of 16                     none
 *   2     std A, 19 std, steps of 16                     none
 *   3     std A, 21 std, steps of 16                     none
 *   4     std A, 23 std, steps of 16                     none
 *   5     std A, 25 std, steps of 16                     none
 *   6     std A, 27 std, steps of 16                     none
 *   7     std A, 29 std, steps of 16                     none
 *   8     std A, 31 std, steps of 16                     none
 *   9     std A, 63 std, steps of 16                     none
 *  10     std A, 31 std, the blocks of loop 0 twice      none
 *  11     std A, 31 std, steps of 8                      none
 *  12     std A, 31 std, steps of 128                    none
 *  13     std A, 31 std, steps of 16                     1 addi a store
 *  14     std A, 31 std, steps of 16                     3 addi a store
 *  15     std A, 15 std, steps of 16                     3 addi a store
 *  16     96 addi                                        --
 *   word
 *  34  the sum of the loaded r9 values (keeps them live)
 *  35  REPS
 *  36  0x50535144 ("PSQD"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS   17
#define WORDS   (2 * LOOPS + 3)
#define PAYLOAD (WORDS * 8)
#define REPS    11
#define ITER    (1u << 15)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
/* 36 lines: line A, the stores' lines and line H at offset 4224. */
static volatile u64 lines[576] __attribute__((aligned(128)));
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

#define CLOBBER_R16 "r16", "r17", "r18", "r19", "r20", "r21", "r22", "r23"

#define STD_A "std 9,0(%[p])\n"
#define LD_H  "ld 9,4224(%[p])\n"

/* std of r10 at offset `o`. */
#define S(o) "std 10," #o "(%[p])\n"
/* std of r10 at offset `o`, then one or three independent addi. */
#define A1 "addi 16,16,1\n"
#define A3 "addi 16,16,1\n" "addi 17,17,1\n" "addi 18,18,1\n"
#define S1(o) S(o) A1
#define S3(o) S(o) A3

/* Steps of 16 from offset 128: the first 15, then pairs up to 63. */
#define S16_15                                                     \
    S(128) S(144) S(160) S(176) S(192) S(208) S(224) S(240)        \
    S(256) S(272) S(288) S(304) S(320) S(336) S(352)
#define S16_17 S16_15 S(368) S(384)
#define S16_19 S16_17 S(400) S(416)
#define S16_21 S16_19 S(432) S(448)
#define S16_23 S16_21 S(464) S(480)
#define S16_25 S16_23 S(496) S(512)
#define S16_27 S16_25 S(528) S(544)
#define S16_29 S16_27 S(560) S(576)
#define S16_31 S16_29 S(592) S(608)
#define S16_63                                                     \
    S16_31 S(624) S(640) S(656) S(672) S(688) S(704) S(720)        \
    S(736) S(752) S(768) S(784) S(800) S(816) S(832) S(848)        \
    S(864) S(880) S(896) S(912) S(928) S(944) S(960) S(976)        \
    S(992) S(1008) S(1024) S(1040) S(1056) S(1072) S(1088)         \
    S(1104) S(1120)

/* The 15 blocks of loop 0, then 16 more over the same blocks. */
#define TWICE_31 S16_15 S(128) S16_15

/* Steps of 8 from offset 128. */
#define S8_31                                                      \
    S(128) S(136) S(144) S(152) S(160) S(168) S(176) S(184)        \
    S(192) S(200) S(208) S(216) S(224) S(232) S(240) S(248)        \
    S(256) S(264) S(272) S(280) S(288) S(296) S(304) S(312)        \
    S(320) S(328) S(336) S(344) S(352) S(360) S(368)

/* Steps of 128 from offset 128: one store a line. */
#define S128_31                                                    \
    S(128) S(256) S(384) S(512) S(640) S(768) S(896) S(1024)       \
    S(1152) S(1280) S(1408) S(1536) S(1664) S(1792) S(1920)        \
    S(2048) S(2176) S(2304) S(2432) S(2560) S(2688) S(2816)        \
    S(2944) S(3072) S(3200) S(3328) S(3456) S(3584) S(3712)        \
    S(3840) S(3968)

/* Steps of 16 with addi after each store. */
#define SP_15(X)                                                   \
    X(128) X(144) X(160) X(176) X(192) X(208) X(224) X(240)        \
    X(256) X(272) X(288) X(304) X(320) X(336) X(352)
#define SP_31(X)                                                   \
    SP_15(X) X(368) X(384) X(400) X(416) X(432) X(448) X(464)      \
    X(480) X(496) X(512) X(528) X(544) X(560) X(576) X(592)        \
    X(608)

#define A3_8 A3 A3 A3 A3 A3 A3 A3 A3
#define A3_32 A3_8 A3_8 A3_8 A3_8

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
            : CLOBBER_R16, "ctr", "memory");                       \
        u64 t1 = timebase();                                       \
        sink += v9 + v10;                                          \
        return t1 - t0;                                            \
    }

LOOP(loop_16, STD_A S16_15 LD_H)
LOOP(loop_18, STD_A S16_17 LD_H)
LOOP(loop_20, STD_A S16_19 LD_H)
LOOP(loop_22, STD_A S16_21 LD_H)
LOOP(loop_24, STD_A S16_23 LD_H)
LOOP(loop_26, STD_A S16_25 LD_H)
LOOP(loop_28, STD_A S16_27 LD_H)
LOOP(loop_30, STD_A S16_29 LD_H)
LOOP(loop_32, STD_A S16_31 LD_H)
LOOP(loop_64, STD_A S16_63 LD_H)
LOOP(loop_twice, STD_A TWICE_31 LD_H)
LOOP(loop_step_8, STD_A S8_31 LD_H)
LOOP(loop_step_128, STD_A S128_31 LD_H)
LOOP(loop_32_addi_1, STD_A A1 SP_31(S1) LD_H)
LOOP(loop_32_addi_3, STD_A A3 SP_31(S3) LD_H)
LOOP(loop_16_addi_3, STD_A A3 SP_15(S3) LD_H)
LOOP(loop_addi_96, A3_32)

int main(void)
{
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 576; i++)
        lines[i] = i;

    TIMED(0, loop_16(ITER));
    TIMED(1, loop_18(ITER));
    TIMED(2, loop_20(ITER));
    TIMED(3, loop_22(ITER));
    TIMED(4, loop_24(ITER));
    TIMED(5, loop_26(ITER));
    TIMED(6, loop_28(ITER));
    TIMED(7, loop_30(ITER));
    TIMED(8, loop_32(ITER));
    TIMED(9, loop_64(ITER));
    TIMED(10, loop_twice(ITER));
    TIMED(11, loop_step_8(ITER));
    TIMED(12, loop_step_128(ITER));
    TIMED(13, loop_32_addi_1(ITER));
    TIMED(14, loop_32_addi_3(ITER));
    TIMED(15, loop_16_addi_3(ITER));
    TIMED(16, loop_addi_96(ITER));
    put(34, sink);
    put(35, REPS);
    put(36, 0x50535144ull);

    write_frame();
    return 0;
}
