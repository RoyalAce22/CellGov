/* PPU probe: what a store multiple costs by its word count, alone and
 * with independent work after it. Each loop runs a fixed body ITER times
 * between two time-base reads. The probe runs each loop REPS times and
 * records the smallest and the largest delta. The loop branch is bdnz.
 *
 * Every store multiple writes from 0(p), the start of a 128-byte-aligned
 * buffer of two lines. The independent work is eight addi to r3 through
 * r10, which no store reads.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  stmw r31 (1 word)
 *   1  stmw r30 (2 words)
 *   2  stmw r28 (4 words)
 *   3  stmw r24 (8 words)
 *   4  stmw r16 (16 words)
 *   5  stmw r14 (18 words)
 *   6  stmw r28, then 8 independent addi
 *   7  stmw r24, then 8 independent addi
 *   8  stmw r16, then 8 independent addi
 *   9  stmw r14, then 8 independent addi
 *  10  std r31, the one-store baseline
 *  11  8 independent addi alone
 *   word
 *  24  REPS
 *  25  log2 of ITER
 *  26  0x50505357 ("PPSW"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     27
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 18

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int lines[64] __attribute__((aligned(128)));

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

#define ADDI8 "addi 3,3,1\n" "addi 4,4,1\n" "addi 5,5,1\n" "addi 6,6,1\n" \
              "addi 7,7,1\n" "addi 8,8,1\n" "addi 9,9,1\n" "addi 10,10,1\n"

#define CLOBBER_R14 "r14", "r15", "r16", "r17", "r18", "r19", "r20", "r21", \
                    "r22", "r23", "r24", "r25", "r26", "r27", "r28", "r29", \
                    "r30", "r31"
#define CLOBBER_ADDI "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10"

/* One timed loop of `body` with `p` the buffer. */
#define STORES(name, body)                                         \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : : [n] "r"(n), [p] "b"(EA(&lines[0]))                 \
            : CLOBBER_R14, CLOBBER_ADDI, "ctr", "memory");         \
        return timebase() - t0;                                    \
    }

STORES(loop_stmw_1, "stmw 31,0(%[p])\n")
STORES(loop_stmw_2, "stmw 30,0(%[p])\n")
STORES(loop_stmw_4, "stmw 28,0(%[p])\n")
STORES(loop_stmw_8, "stmw 24,0(%[p])\n")
STORES(loop_stmw_16, "stmw 16,0(%[p])\n")
STORES(loop_stmw_18, "stmw 14,0(%[p])\n")
STORES(loop_stmw_4_addi, "stmw 28,0(%[p])\n" ADDI8)
STORES(loop_stmw_8_addi, "stmw 24,0(%[p])\n" ADDI8)
STORES(loop_stmw_16_addi, "stmw 16,0(%[p])\n" ADDI8)
STORES(loop_stmw_18_addi, "stmw 14,0(%[p])\n" ADDI8)
STORES(loop_std, "std 31,0(%[p])\n")
STORES(loop_addi, ADDI8)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 64; i++)
        lines[i] = i;

    TIMED(0, loop_stmw_1(n));
    TIMED(1, loop_stmw_2(n));
    TIMED(2, loop_stmw_4(n));
    TIMED(3, loop_stmw_8(n));
    TIMED(4, loop_stmw_16(n));
    TIMED(5, loop_stmw_18(n));
    TIMED(6, loop_stmw_4_addi(n));
    TIMED(7, loop_stmw_8_addi(n));
    TIMED(8, loop_stmw_16_addi(n));
    TIMED(9, loop_stmw_18_addi(n));
    TIMED(10, loop_std(n));
    TIMED(11, loop_addi(n));
    put(24, REPS);
    put(25, ITER_LOG2);
    put(26, 0x50505357ull);

    write_frame();
    return 0;
}
