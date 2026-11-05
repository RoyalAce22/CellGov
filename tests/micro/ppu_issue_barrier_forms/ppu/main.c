/* PPU probe: what a sync of each L form costs, and whether a sync, lwsync
 * or isync after a store that takes several store-queue entries waits
 * longer than after a store that takes one. Each loop runs a fixed body
 * ITER times between two time-base reads. The probe runs each loop REPS
 * times and records the smallest and the largest delta. Every store
 * addresses one 128-byte-aligned buffer the probe zeroes first; r29
 * holds its address. Every loop counts in CTR and closes with bdnz.
 *
 * The store-queue entry is one 16-byte block. std 14,12(29) writes bytes
 * 12 to 19, two blocks. stmw 24 writes 32 bytes, two blocks; stmw 16
 * writes 64 bytes, four blocks. STD4 is four std, one to each of four
 * blocks.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  sync
 *   1  sync 3
 *   2  std r14,0(r29); sync
 *   3  std r14,0(r29); sync 3
 *   4  lwsync
 *   5  std r14,0(r29); lwsync
 *   6  std r14,12(r29); sync
 *   7  std r14,12(r29); lwsync
 *   8  stmw r24,0(r29); sync
 *   9  stmw r24,0(r29); lwsync
 *  10  stmw r16,0(r29); sync
 *  11  stmw r16,0(r29); lwsync
 *  12  STD4; sync
 *  13  STD4; lwsync
 *  14  std r14,12(r29); isync
 *  15  stmw r16,0(r29); isync
 *  16  ptesync (sync 2)
 *  17  std r14,0(r29); ptesync
 *   word
 *  36  REPS
 *  37  log2 of ITER
 *  38  0x50504246 ("PPBF"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS     18
#define WORDS     39
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
#define STD12   "std 14,12(29)\n"
#define STMW24  "stmw 24,0(29)\n"
#define STMW16  "stmw 16,0(29)\n"
#define STD4    "std 14,0(29)\n" "std 14,16(29)\n" "std 14,32(29)\n" "std 14,48(29)\n"
#define SYNC    "sync\n"
#define SYNC3   ".long 0x7c6004ac\n"
#define PTESYNC ".long 0x7c4004ac\n"
#define LWSYNC  "lwsync\n"
#define ISYNC   "isync\n"

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

MLOOP(loop_sync, SYNC)
MLOOP(loop_sync3, SYNC3)
MLOOP(loop_std_sync, STD SYNC)
MLOOP(loop_std_sync3, STD SYNC3)
MLOOP(loop_lwsync, LWSYNC)
MLOOP(loop_std_lwsync, STD LWSYNC)
MLOOP(loop_std12_sync, STD12 SYNC)
MLOOP(loop_std12_lwsync, STD12 LWSYNC)
MLOOP(loop_stmw24_sync, STMW24 SYNC)
MLOOP(loop_stmw24_lwsync, STMW24 LWSYNC)
MLOOP(loop_stmw16_sync, STMW16 SYNC)
MLOOP(loop_stmw16_lwsync, STMW16 LWSYNC)
MLOOP(loop_std4_sync, STD4 SYNC)
MLOOP(loop_std4_lwsync, STD4 LWSYNC)
MLOOP(loop_std12_isync, STD12 ISYNC)
MLOOP(loop_stmw16_isync, STMW16 ISYNC)
MLOOP(loop_ptesync, PTESYNC)
MLOOP(loop_std_ptesync, STD PTESYNC)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;

    TIMED(0, loop_sync(n));
    TIMED(1, loop_sync3(n));
    TIMED(2, loop_std_sync(n));
    TIMED(3, loop_std_sync3(n));
    TIMED(4, loop_lwsync(n));
    TIMED(5, loop_std_lwsync(n));
    TIMED(6, loop_std12_sync(n));
    TIMED(7, loop_std12_lwsync(n));
    TIMED(8, loop_stmw24_sync(n));
    TIMED(9, loop_stmw24_lwsync(n));
    TIMED(10, loop_stmw16_sync(n));
    TIMED(11, loop_stmw16_lwsync(n));
    TIMED(12, loop_std4_sync(n));
    TIMED(13, loop_std4_lwsync(n));
    TIMED(14, loop_std12_isync(n));
    TIMED(15, loop_stmw16_isync(n));
    TIMED(16, loop_ptesync(n));
    TIMED(17, loop_std_ptesync(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x50504246ull);

    write_frame();
    return 0;
}
