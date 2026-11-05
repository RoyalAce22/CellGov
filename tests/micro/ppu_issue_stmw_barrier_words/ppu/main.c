/* PPU probe: what an lwsync and a sync cost after a store multiple of 1
 * to 8 words. Each loop runs a fixed body ITER times between two
 * time-base reads. The probe runs each loop REPS times and records the
 * smallest and the largest delta. Every store addresses one
 * 128-byte-aligned buffer the probe zeroes first; r29 holds its address.
 * Every loop counts in CTR and closes with bdnz.
 *
 * stmw rS stores the 32 - S words from rS to r31.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  std r14,0(r29); lwsync
 *   1  stmw r31,0(r29); lwsync          (1 word)
 *   2  stmw r30,0(r29); lwsync          (2 words)
 *   3  stmw r29,0(r29); lwsync          (3 words)
 *   4  stmw r28,0(r29); lwsync          (4 words)
 *   5  stmw r27,0(r29); lwsync          (5 words)
 *   6  stmw r26,0(r29); lwsync          (6 words)
 *   7  stmw r25,0(r29); lwsync          (7 words)
 *   8  stmw r24,0(r29); lwsync          (8 words)
 *   9  std r14,0(r29); sync
 *  10  stmw r31,0(r29); sync            (1 word)
 *  11  stmw r30,0(r29); sync            (2 words)
 *  12  stmw r28,0(r29); sync            (4 words)
 *  13  stmw r24,0(r29); sync            (8 words)
 *   word
 *  28  REPS
 *  29  log2 of ITER
 *  30  0x50505358 ("PPSX"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS     14
#define WORDS     31
#define PAYLOAD   (WORDS * 8)
#define REPS      11
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
            : "r29", "ctr", "memory");                             \
        return timebase() - t0;                                    \
    }

MLOOP(loop_std_lwsync, STD LWSYNC)
MLOOP(loop_stmw31_lwsync, "stmw 31,0(29)\n" LWSYNC)
MLOOP(loop_stmw30_lwsync, "stmw 30,0(29)\n" LWSYNC)
MLOOP(loop_stmw29_lwsync, "stmw 29,0(29)\n" LWSYNC)
MLOOP(loop_stmw28_lwsync, "stmw 28,0(29)\n" LWSYNC)
MLOOP(loop_stmw27_lwsync, "stmw 27,0(29)\n" LWSYNC)
MLOOP(loop_stmw26_lwsync, "stmw 26,0(29)\n" LWSYNC)
MLOOP(loop_stmw25_lwsync, "stmw 25,0(29)\n" LWSYNC)
MLOOP(loop_stmw24_lwsync, "stmw 24,0(29)\n" LWSYNC)
MLOOP(loop_std_sync, STD SYNC)
MLOOP(loop_stmw31_sync, "stmw 31,0(29)\n" SYNC)
MLOOP(loop_stmw30_sync, "stmw 30,0(29)\n" SYNC)
MLOOP(loop_stmw28_sync, "stmw 28,0(29)\n" SYNC)
MLOOP(loop_stmw24_sync, "stmw 24,0(29)\n" SYNC)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;

    TIMED(0, loop_std_lwsync(n));
    TIMED(1, loop_stmw31_lwsync(n));
    TIMED(2, loop_stmw30_lwsync(n));
    TIMED(3, loop_stmw29_lwsync(n));
    TIMED(4, loop_stmw28_lwsync(n));
    TIMED(5, loop_stmw27_lwsync(n));
    TIMED(6, loop_stmw26_lwsync(n));
    TIMED(7, loop_stmw25_lwsync(n));
    TIMED(8, loop_stmw24_lwsync(n));
    TIMED(9, loop_std_sync(n));
    TIMED(10, loop_stmw31_sync(n));
    TIMED(11, loop_stmw30_sync(n));
    TIMED(12, loop_stmw28_sync(n));
    TIMED(13, loop_stmw24_sync(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x50505358ull);

    write_frame();
    return 0;
}
