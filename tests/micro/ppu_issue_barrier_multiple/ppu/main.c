/* PPU probe: what a barrier costs after a store multiple or a store
 * string, by the store's word count, by the work between the store and
 * the barrier, and against the same barrier after an std. Each loop runs
 * a fixed body ITER times between two time-base reads. The probe runs
 * each loop REPS times and records the smallest and the largest delta.
 * Every store addresses one 128-byte-aligned buffer the probe zeroes
 * first; r29 holds its address. Every loop counts in CTR and closes with
 * bdnz.
 *
 * ADDI8 is eight independent addi, to r3 through r10.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  std r14,0(r29); lwsync
 *   1  stmw r24,0(r29); lwsync          (8 words)
 *   2  stmw r30,0(r29); lwsync          (2 words)
 *   3  stmw r28,0(r29); lwsync          (4 words, one 16-byte block)
 *   4  stmw r24,0(r29); ADDI8; lwsync
 *   5  stmw r24,0(r29); ADDI8 x4; lwsync
 *   6  stswi r24,r29,8; lwsync          (2 words)
 *   7  stswi r24,r29,32; lwsync         (8 words)
 *   8  stswi r24,r29,32; sync
 *   9  stmw r30,0(r29); sync            (2 words)
 *  10  stmw r24,0(r29); std r14,64(r29); lwsync
 *  11  std r14,0(r29); ADDI8; lwsync
 *  12  std r14,0(r29); sync
 *   word
 *  26  REPS
 *  27  log2 of ITER
 *  28  0x5050424d ("PPBM"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS     13
#define WORDS     29
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 16

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

#define STD     "std 14,0(29)\n"
#define STD64   "std 14,64(29)\n"
#define STMW24  "stmw 24,0(29)\n"
#define STMW28  "stmw 28,0(29)\n"
#define STMW30  "stmw 30,0(29)\n"
#define STSWI8  "stswi 24,29,8\n"
#define STSWI32 "stswi 24,29,32\n"
#define ADDI8   "addi 3,3,1\n" "addi 4,4,1\n" "addi 5,5,1\n" "addi 6,6,1\n" \
                "addi 7,7,1\n" "addi 8,8,1\n" "addi 9,9,1\n" "addi 10,10,1\n"
#define SYNC    "sync\n"
#define LWSYNC  "lwsync\n"

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "mr 29,%[p]\n" "mtctr %[n]\n"                          \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                 \
            : : [n] "r"(n), [p] "r"(buf)                           \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",     \
              "r29", "ctr", "memory");                             \
        return timebase() - t0;                                    \
    }

MLOOP(loop_std_lwsync, STD LWSYNC)
MLOOP(loop_stmw24_lwsync, STMW24 LWSYNC)
MLOOP(loop_stmw30_lwsync, STMW30 LWSYNC)
MLOOP(loop_stmw28_lwsync, STMW28 LWSYNC)
MLOOP(loop_stmw24_addi8_lwsync, STMW24 ADDI8 LWSYNC)
MLOOP(loop_stmw24_addi32_lwsync, STMW24 ADDI8 ADDI8 ADDI8 ADDI8 LWSYNC)
MLOOP(loop_stswi8_lwsync, STSWI8 LWSYNC)
MLOOP(loop_stswi32_lwsync, STSWI32 LWSYNC)
MLOOP(loop_stswi32_sync, STSWI32 SYNC)
MLOOP(loop_stmw30_sync, STMW30 SYNC)
MLOOP(loop_stmw24_std_lwsync, STMW24 STD64 LWSYNC)
MLOOP(loop_std_addi8_lwsync, STD ADDI8 LWSYNC)
MLOOP(loop_std_sync, STD SYNC)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;

    TIMED(0, loop_std_lwsync(n));
    TIMED(1, loop_stmw24_lwsync(n));
    TIMED(2, loop_stmw30_lwsync(n));
    TIMED(3, loop_stmw28_lwsync(n));
    TIMED(4, loop_stmw24_addi8_lwsync(n));
    TIMED(5, loop_stmw24_addi32_lwsync(n));
    TIMED(6, loop_stswi8_lwsync(n));
    TIMED(7, loop_stswi32_lwsync(n));
    TIMED(8, loop_stswi32_sync(n));
    TIMED(9, loop_stmw30_sync(n));
    TIMED(10, loop_stmw24_std_lwsync(n));
    TIMED(11, loop_std_addi8_lwsync(n));
    TIMED(12, loop_std_sync(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x5050424dull);

    write_frame();
    return 0;
}
