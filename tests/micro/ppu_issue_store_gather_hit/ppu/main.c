/* PPU probe: which bytes of a pending store's 16-byte block a load waits
 * for. Each loop runs a fixed body ITER times between two time-base reads.
 * The probe runs each loop REPS times and records the smallest and the
 * largest delta. The loop branch is bdnz.
 *
 * Line A is the first 128-byte line, B the second, and H the twelfth line,
 * which no store writes. Each loop starts with an stw of r9 to A + 0 and
 * ends with an lwz of r9, so an iteration waits for its load. The other
 * stores are stw of r10.
 *
 * Loops 0 to 4 run one store and a load of the bytes it wrote, of the
 * other word of its doubleword, of the other doubleword of its block, of
 * the next block, and of line H.
 *
 * Loops 5 to 8 run two stores, then a load. In loops 5 to 7 both stores
 * write block A + 0, the second at A + 8 or A + 12, and the load reads
 * bytes of that block no store wrote: between the two stores (5, 6) or
 * past the second (7). Loop 8 puts the second store in line B.
 *
 * Loops 9 and 10 put 96 independent addi between the two stores, so the
 * first store has written its bytes before the second issues; the second
 * store writes A + 4 (the same block) or B + 0, and the load reads the
 * first store's bytes. Loop 11 runs the 96 addi alone.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1. The words from 24 on are listed after the
 * loops. Every loop runs 1 << 16 iterations.
 *   loop  stores                          load
 *   0     stw A+0                         A+0
 *   1     stw A+0                         A+4
 *   2     stw A+0                         A+8
 *   3     stw A+0                         A+16
 *   4     stw A+0                         H
 *   5     stw A+0, stw A+8                A+4
 *   6     stw A+0, stw A+12               A+8
 *   7     stw A+0, stw A+8                A+12
 *   8     stw A+0, stw B+0                A+4
 *   9     stw A+0, 96 addi, stw A+4       A+0
 *  10     stw A+0, 96 addi, stw B+0       A+0
 *  11     96 addi                         --
 *   word
 *  24  the sum of the loaded r9 values (keeps them live)
 *  25  REPS
 *  26  0x50534748 ("PSGH"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS   12
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

#define CLOBBER_R16 "r16", "r17", "r18"

/* The stw of the chained word to A + 0. */
#define STW_A "stw 9,0(%[p])\n"
/* stw of r10 at offset `o`. */
#define S(o) "stw 10," #o "(%[p])\n"
/* lwz of r9 at offset `o`. */
#define L(o) "lwz 9," #o "(%[p])\n"

/* 96 independent addi over r16 to r18. */
#define A3 "addi 16,16,1\n" "addi 17,17,1\n" "addi 18,18,1\n"
#define A3_8 A3 A3 A3 A3 A3 A3 A3 A3
#define A3_32 A3_8 A3_8 A3_8 A3_8

/* One timed loop of `body` with `p` line A, r9 the chained word and r10
 * the value the other stores write. Returns the delta; adds r9 to the
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

LOOP(loop_same, STW_A L(0))
LOOP(loop_other_word, STW_A L(4))
LOOP(loop_other_dword, STW_A L(8))
LOOP(loop_next_block, STW_A L(16))
LOOP(loop_line_h, STW_A L(1408))
LOOP(loop_gather_between, STW_A S(8) L(4))
LOOP(loop_gather_between_dword, STW_A S(12) L(8))
LOOP(loop_gather_past, STW_A S(8) L(12))
LOOP(loop_apart_between, STW_A S(128) L(4))
LOOP(loop_written_gather, STW_A A3_32 S(4) L(0))
LOOP(loop_written_apart, STW_A A3_32 S(128) L(0))
LOOP(loop_addi_96, A3_32)

int main(void)
{
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 192; i++)
        lines[i] = i;

    TIMED(0, loop_same(ITER));
    TIMED(1, loop_other_word(ITER));
    TIMED(2, loop_other_dword(ITER));
    TIMED(3, loop_next_block(ITER));
    TIMED(4, loop_line_h(ITER));
    TIMED(5, loop_gather_between(ITER));
    TIMED(6, loop_gather_between_dword(ITER));
    TIMED(7, loop_gather_past(ITER));
    TIMED(8, loop_apart_between(ITER));
    TIMED(9, loop_written_gather(ITER));
    TIMED(10, loop_written_apart(ITER));
    TIMED(11, loop_addi_96(ITER));
    put(24, sink);
    put(25, REPS);
    put(26, 0x50534748ull);

    write_frame();
    return 0;
}
