/* PPU probe: what a barrier costs after a single-precision store against
 * a double-precision one, and whether the store's width, the conversion
 * or the stored value costs the difference. Each loop runs a fixed body
 * ITER times between two time-base reads. The probe runs each loop REPS
 * times and records the smallest and the largest delta. Every store
 * addresses one 128-byte-aligned buffer the probe zeroes first; r29
 * holds its address. Every loop counts in CTR and closes with bdnz.
 *
 * Each loop loads f14 = 0.0 and f15 = 1.5 and sets r14 = 0 before its
 * timed body. ADDI8 is eight independent addi, to r3 through r10.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  std r14,0(r29); sync
 *   1  stfd f14,0(r29); sync
 *   2  stfs f14,0(r29); sync
 *   3  stfiwx f14,0,r29; sync
 *   4  stw r14,0(r29); sync
 *   5  stfs f15,0(r29); sync
 *   6  stfd f14,0(r29); lwsync
 *   7  stfs f14,0(r29); lwsync
 *   8  stfd f14,0(r29); isync
 *   9  stfs f14,0(r29); isync
 *  10  stfs f14,0(r29); ADDI8; sync
 *  11  stfd f14,0(r29); ADDI8; sync
 *  12  stfs f14,4(r29); sync
 *  13  frsp f16,f14; stfd f16,0(r29); sync
 *   word
 *  28  REPS
 *  29  log2 of ITER
 *  30  0x50504653 ("PPFS"): the probe wrote every word
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
static const double consts[2] __attribute__((aligned(16))) = { 0.0, 1.5 };

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
#define STW     "stw 14,0(29)\n"
#define STFD    "stfd 14,0(29)\n"
#define STFS    "stfs 14,0(29)\n"
#define STFS15  "stfs 15,0(29)\n"
#define STFS4   "stfs 14,4(29)\n"
#define STFIWX  "stfiwx 14,0,29\n"
#define FRSPD   "frsp 16,14\n" "stfd 16,0(29)\n"
#define ADDI8   "addi 3,3,1\n" "addi 4,4,1\n" "addi 5,5,1\n" "addi 6,6,1\n" \
                "addi 7,7,1\n" "addi 8,8,1\n" "addi 9,9,1\n" "addi 10,10,1\n"
#define SYNC    "sync\n"
#define LWSYNC  "lwsync\n"
#define ISYNC   "isync\n"

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0;                                                    \
        __asm__ volatile (                                         \
            "lfd 14,0(%[c])\n" "lfd 15,8(%[c])\n" "li 14,0\n"      \
            : : [c] "b"(consts)                                    \
            : "fr14", "fr15", "r14");                              \
        t0 = timebase();                                           \
        __asm__ volatile (                                         \
            "mr 29,%[p]\n" "mtctr %[n]\n"                          \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                 \
            : : [n] "r"(n), [p] "r"(buf)                           \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",     \
              "r29", "fr16", "ctr", "memory");                     \
        return timebase() - t0;                                    \
    }

MLOOP(loop_std_sync, STD SYNC)
MLOOP(loop_stfd_sync, STFD SYNC)
MLOOP(loop_stfs_sync, STFS SYNC)
MLOOP(loop_stfiwx_sync, STFIWX SYNC)
MLOOP(loop_stw_sync, STW SYNC)
MLOOP(loop_stfs15_sync, STFS15 SYNC)
MLOOP(loop_stfd_lwsync, STFD LWSYNC)
MLOOP(loop_stfs_lwsync, STFS LWSYNC)
MLOOP(loop_stfd_isync, STFD ISYNC)
MLOOP(loop_stfs_isync, STFS ISYNC)
MLOOP(loop_stfs_addi8_sync, STFS ADDI8 SYNC)
MLOOP(loop_stfd_addi8_sync, STFD ADDI8 SYNC)
MLOOP(loop_stfs4_sync, STFS4 SYNC)
MLOOP(loop_frsp_stfd_sync, FRSPD SYNC)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;

    TIMED(0, loop_std_sync(n));
    TIMED(1, loop_stfd_sync(n));
    TIMED(2, loop_stfs_sync(n));
    TIMED(3, loop_stfiwx_sync(n));
    TIMED(4, loop_stw_sync(n));
    TIMED(5, loop_stfs15_sync(n));
    TIMED(6, loop_stfd_lwsync(n));
    TIMED(7, loop_stfs_lwsync(n));
    TIMED(8, loop_stfd_isync(n));
    TIMED(9, loop_stfs_isync(n));
    TIMED(10, loop_stfs_addi8_sync(n));
    TIMED(11, loop_stfd_addi8_sync(n));
    TIMED(12, loop_stfs4_sync(n));
    TIMED(13, loop_frsp_stfd_sync(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x50504653ull);

    write_frame();
    return 0;
}
