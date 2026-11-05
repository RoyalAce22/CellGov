/* PPU probe: which stores a load that hits the store queue waits for, and
 * whether its recycle holds the instructions after it. Each loop runs a
 * fixed body ITER times between two time-base reads. The probe runs each
 * loop REPS times and records the smallest and the largest delta. The
 * loop branch is bdnz.
 *
 * Lines A to E are five 128-byte lines in a row, A first. Loops 0 to 5
 * chain each iteration through r9: an ld of line A loads r9, and the next
 * iteration's std stores it. Loops 1 to 4 put stores to other lines
 * before or after the store to A; loop 5 stores r9 to line E, so its load
 * hits no store.
 *
 * Loops 6 to 11 follow a store multiple and a load multiple of the same
 * bytes, or one std and ld of the same doubleword, with independent addi
 * on r16 to r23, which nothing in the loop reads.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1. The words from 24 on are listed after the
 * loops.
 *   loop
 *   0  std A, ld A                                (1 << 18 iterations)
 *   1  std A, std B, ld A                         (1 << 18)
 *   2  std B, std A, ld A                         (1 << 18)
 *   3  std A, std B, std C, std D, ld A           (1 << 18)
 *   4  std B, std C, std D, std A, ld A           (1 << 18)
 *   5  std B, std C, std D, std E, ld A           (1 << 18)
 *   6  stmw r24 A, lmw r24 A                      (1 << 17)
 *   7  stmw r24 A, lmw r24 A, 8 addi              (1 << 17)
 *   8  stmw r24 A, lmw r24 A, 32 addi             (1 << 17)
 *   9  lmw r24 A, 32 addi                         (1 << 17)
 *  10  std A, ld A, 32 addi                       (1 << 18)
 *  11  32 addi                                    (1 << 18)
 *   word
 *  24  the sum of the loaded r9 values (keeps them live)
 *  25  REPS
 *  26  0x504c4853 ("PLHS"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS   27
#define PAYLOAD (WORDS * 8)
#define REPS    11

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 lines[80] __attribute__((aligned(128)));
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
#define CLOBBER_R24 "r24", "r25", "r26", "r27", "r28", "r29", "r30", "r31"

#define ADDI_8                                                     \
    "addi 16,16,1\n" "addi 17,17,1\n" "addi 18,18,1\n" "addi 19,19,1\n" \
    "addi 20,20,1\n" "addi 21,21,1\n" "addi 22,22,1\n" "addi 23,23,1\n"
#define ADDI_32 ADDI_8 ADDI_8 ADDI_8 ADDI_8

#define STD_A "std 9,0(%[p])\n"
#define STD_B "std 10,128(%[p])\n"
#define STD_C "std 10,256(%[p])\n"
#define STD_D "std 10,384(%[p])\n"
#define STD_E "std 9,512(%[p])\n"
#define LD_A  "ld 9,0(%[p])\n"

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
            : CLOBBER_R16, CLOBBER_R24, "ctr", "memory");          \
        u64 t1 = timebase();                                       \
        sink += v9 + v10;                                          \
        return t1 - t0;                                            \
    }

LOOP(loop_a, STD_A LD_A)
LOOP(loop_a_b, STD_A STD_B LD_A)
LOOP(loop_b_a, STD_B STD_A LD_A)
LOOP(loop_a_bcd, STD_A STD_B STD_C STD_D LD_A)
LOOP(loop_bcd_a, STD_B STD_C STD_D STD_A LD_A)
LOOP(loop_bcd_e, STD_B STD_C STD_D STD_E LD_A)
LOOP(loop_pair, "stmw 24,0(%[p])\n" "lmw 24,0(%[p])\n")
LOOP(loop_pair_addi_8, "stmw 24,0(%[p])\n" "lmw 24,0(%[p])\n" ADDI_8)
LOOP(loop_pair_addi_32, "stmw 24,0(%[p])\n" "lmw 24,0(%[p])\n" ADDI_32)
LOOP(loop_lmw_addi_32, "lmw 24,0(%[p])\n" ADDI_32)
LOOP(loop_a_addi_32, STD_A LD_A ADDI_32)
LOOP(loop_addi_32, ADDI_32)

int main(void)
{
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 80; i++)
        lines[i] = i;

    TIMED(0, loop_a(1u << 18));
    TIMED(1, loop_a_b(1u << 18));
    TIMED(2, loop_b_a(1u << 18));
    TIMED(3, loop_a_bcd(1u << 18));
    TIMED(4, loop_bcd_a(1u << 18));
    TIMED(5, loop_bcd_e(1u << 18));
    TIMED(6, loop_pair(1u << 17));
    TIMED(7, loop_pair_addi_8(1u << 17));
    TIMED(8, loop_pair_addi_32(1u << 17));
    TIMED(9, loop_lmw_addi_32(1u << 17));
    TIMED(10, loop_a_addi_32(1u << 18));
    TIMED(11, loop_addi_32(1u << 18));
    put(24, sink);
    put(25, REPS);
    put(26, 0x504c4853ull);

    write_frame();
    return 0;
}
