/* PPU probe: whether a sync, lwsync, isync, mffs, mfvscr or sc waits
 * longer after a floating-point store than after a fixed-point store.
 * Each loop runs a fixed body ITER times between two time-base reads.
 * The probe runs each loop REPS times and records the smallest and the
 * largest delta. Every store addresses one 128-byte-aligned buffer the
 * probe zeroes first; r29 holds its address. f14 starts zero.
 *
 * The loops without sc count in CTR and close with bdnz. The sc loops
 * count in r28 (addi r28,r28,-1; cmpdi r28,0; bne) and run SC_ITER
 * times, since a call clobbers CTR. Their call is sc 147,
 * sys_time_get_timebase_frequency, which takes no arguments.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  sync
 *   1  std r14,0(r29); sync
 *   2  stfd f14,0(r29); sync
 *   3  stfs f14,0(r29); sync
 *   4  stfiwx f14,0,r29; sync
 *   5  lwsync
 *   6  std r14,0(r29); lwsync
 *   7  stfd f14,0(r29); lwsync
 *   8  isync
 *   9  std r14,0(r29); isync
 *  10  stfd f14,0(r29); isync
 *  11  mffs f3
 *  12  std r14,0(r29); mffs f3
 *  13  stfd f14,0(r29); mffs f3
 *  14  mfvscr v3
 *  15  std r14,0(r29); mfvscr v3
 *  16  stfd f14,0(r29); mfvscr v3
 *  17  li r11,147; sc
 *  18  std r14,0(r29); li r11,147; sc
 *  19  stfd f14,0(r29); li r11,147; sc
 *   word
 *  40  REPS
 *  41  log2 of ITER
 *  42  log2 of SC_ITER
 *  43  r3 after the last run of loop 17
 *  44  r3 after the last run of loop 18
 *  45  r3 after the last run of loop 19
 *  46  0x50504653 ("PPFS"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS        47
#define PAYLOAD      (WORDS * 8)
#define REPS         5
#define ITER_LOG2    16
#define SC_ITER_LOG2 12

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

#define STD    "std 14,0(29)\n"
#define STFD   "stfd 14,0(29)\n"
#define STFS   "stfs 14,0(29)\n"
#define STFIWX "stfiwx 14,0,29\n"
#define SYNC   "sync\n"
#define LWSYNC "lwsync\n"
#define ISYNC  "isync\n"
#define MFFS   "mffs 3\n"
#define MFVSCR "mfvscr 3\n"
#define CALL   "li 11,147\n" "sc\n"

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "mr 29,%[p]\n" "mtctr %[n]\n"                          \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                 \
            : : [n] "r"(n), [p] "r"(buf)                           \
            : "r29", "ctr", "memory", "fr3", "v3");                \
        return timebase() - t0;                                    \
    }

/* A loop of calls; leaves the last call's r3 in *ret. */
#define CLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n, u64 *ret)     \
    {                                                              \
        u64 r;                                                     \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "mr 29,%[p]\n" "mr 28,%[n]\n"                          \
            ".p2align 5\n" "1:\n" body CALL                        \
            "addi 28,28,-1\n" "cmpdi 28,0\n" "bne 1b\n"            \
            "mr %[r],3\n"                                          \
            : [r] "=r"(r)                                          \
            : [n] "r"(n), [p] "r"(buf)                             \
            : "r0", "r3", "r4", "r5", "r6", "r7", "r8", "r9",      \
              "r10", "r11", "r12", "r28", "r29", "ctr", "lr",      \
              "cr0", "cr1", "cr5", "cr6", "cr7", "xer", "memory",  \
              "fr0", "fr1", "fr2", "fr3", "fr4", "fr5", "fr6",     \
              "fr7", "fr8", "fr9", "fr10", "fr11", "fr12", "fr13");\
        *ret = r;                                                  \
        return timebase() - t0;                                    \
    }

MLOOP(loop_sync, SYNC)
MLOOP(loop_std_sync, STD SYNC)
MLOOP(loop_stfd_sync, STFD SYNC)
MLOOP(loop_stfs_sync, STFS SYNC)
MLOOP(loop_stfiwx_sync, STFIWX SYNC)
MLOOP(loop_lwsync, LWSYNC)
MLOOP(loop_std_lwsync, STD LWSYNC)
MLOOP(loop_stfd_lwsync, STFD LWSYNC)
MLOOP(loop_isync, ISYNC)
MLOOP(loop_std_isync, STD ISYNC)
MLOOP(loop_stfd_isync, STFD ISYNC)
MLOOP(loop_mffs, MFFS)
MLOOP(loop_std_mffs, STD MFFS)
MLOOP(loop_stfd_mffs, STFD MFFS)
MLOOP(loop_mfvscr, MFVSCR)
MLOOP(loop_std_mfvscr, STD MFVSCR)
MLOOP(loop_stfd_mfvscr, STFD MFVSCR)
CLOOP(loop_sc, "")
CLOOP(loop_std_sc, STD)
CLOOP(loop_stfd_sc, STFD)

int main(void)
{
    unsigned i;
    u64 r17 = 0, r18 = 0, r19 = 0;
    const u64 n = 1ull << ITER_LOG2;
    const u64 sc_n = 1ull << SC_ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* f14 and r14 start zero. */
    __asm__ volatile (
        "lfd 14,0(%[p])\n" "li 14,0\n"
        : : [p] "b"(buf) : "fr14", "r14", "memory");

    TIMED(0, loop_sync(n));
    TIMED(1, loop_std_sync(n));
    TIMED(2, loop_stfd_sync(n));
    TIMED(3, loop_stfs_sync(n));
    TIMED(4, loop_stfiwx_sync(n));
    TIMED(5, loop_lwsync(n));
    TIMED(6, loop_std_lwsync(n));
    TIMED(7, loop_stfd_lwsync(n));
    TIMED(8, loop_isync(n));
    TIMED(9, loop_std_isync(n));
    TIMED(10, loop_stfd_isync(n));
    TIMED(11, loop_mffs(n));
    TIMED(12, loop_std_mffs(n));
    TIMED(13, loop_stfd_mffs(n));
    TIMED(14, loop_mfvscr(n));
    TIMED(15, loop_std_mfvscr(n));
    TIMED(16, loop_stfd_mfvscr(n));
    TIMED(17, loop_sc(sc_n, &r17));
    TIMED(18, loop_std_sc(sc_n, &r18));
    TIMED(19, loop_stfd_sc(sc_n, &r19));

    put(40, REPS);
    put(41, ITER_LOG2);
    put(42, SC_ITER_LOG2);
    put(43, r17);
    put(44, r18);
    put(45, r19);
    put(46, 0x50504653ull);

    write_frame();
    return 0;
}
