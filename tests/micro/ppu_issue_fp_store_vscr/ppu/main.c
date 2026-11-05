/* PPU probe: whether each floating-point store form waits for an older
 * mfvscr's result, and how that wait falls off as independent work
 * separates the mfvscr from the store. Each loop runs a fixed body ITER
 * times between two time-base reads. The probe runs each loop REPS times
 * and records the smallest and the largest delta. Every store addresses
 * one 128-byte-aligned buffer the probe zeroes first; r29 holds its
 * address. f14 and r14 start zero. Each loop counts in CTR and closes
 * with bdnz.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mfvscr v3
 *   1  std r14,0(r29); mfvscr v3
 *   2  stfd f14,0(r29); mfvscr v3
 *   3  stfs f14,0(r29); mfvscr v3
 *   4  stfiwx f14,0,r29; mfvscr v3
 *   5  stfdx f14,0,r29; mfvscr v3
 *   6  stfsx f14,0,r29; mfvscr v3
 *   7  mfvscr v3; 8 x li r10,1; std r14,0(r29)
 *   8  mfvscr v3; 8 x li r10,1; stfd f14,0(r29)
 *   9  mfvscr v3; 16 x li r10,1; std r14,0(r29)
 *  10  mfvscr v3; 16 x li r10,1; stfd f14,0(r29)
 *  11  mfvscr v3; 32 x li r10,1; std r14,0(r29)
 *  12  mfvscr v3; 32 x li r10,1; stfd f14,0(r29)
 *   word
 *  26  REPS
 *  27  log2 of ITER
 *  28  0x50504656 ("PPFV"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS        29
#define PAYLOAD      (WORDS * 8)
#define REPS         5
#define ITER_LOG2    16

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
#define STFDX  "stfdx 14,0,29\n"
#define STFSX  "stfsx 14,0,29\n"
#define MFVSCR "mfvscr 3\n"
#define LI1    "li 10,1\n"
#define LI8    LI1 LI1 LI1 LI1 LI1 LI1 LI1 LI1
#define LI16   LI8 LI8
#define LI32   LI16 LI16

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "mr 29,%[p]\n" "mtctr %[n]\n"                          \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                 \
            : : [n] "r"(n), [p] "r"(buf)                           \
            : "r10", "r29", "ctr", "memory", "v3");                \
        return timebase() - t0;                                    \
    }

MLOOP(loop_mfvscr, MFVSCR)
MLOOP(loop_std_mfvscr, STD MFVSCR)
MLOOP(loop_stfd_mfvscr, STFD MFVSCR)
MLOOP(loop_stfs_mfvscr, STFS MFVSCR)
MLOOP(loop_stfiwx_mfvscr, STFIWX MFVSCR)
MLOOP(loop_stfdx_mfvscr, STFDX MFVSCR)
MLOOP(loop_stfsx_mfvscr, STFSX MFVSCR)
MLOOP(loop_mfvscr_8_std, MFVSCR LI8 STD)
MLOOP(loop_mfvscr_8_stfd, MFVSCR LI8 STFD)
MLOOP(loop_mfvscr_16_std, MFVSCR LI16 STD)
MLOOP(loop_mfvscr_16_stfd, MFVSCR LI16 STFD)
MLOOP(loop_mfvscr_32_std, MFVSCR LI32 STD)
MLOOP(loop_mfvscr_32_stfd, MFVSCR LI32 STFD)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* f14 and r14 start zero. */
    __asm__ volatile (
        "lfd 14,0(%[p])\n" "li 14,0\n"
        : : [p] "b"(buf) : "fr14", "r14", "memory");

    TIMED(0, loop_mfvscr(n));
    TIMED(1, loop_std_mfvscr(n));
    TIMED(2, loop_stfd_mfvscr(n));
    TIMED(3, loop_stfs_mfvscr(n));
    TIMED(4, loop_stfiwx_mfvscr(n));
    TIMED(5, loop_stfdx_mfvscr(n));
    TIMED(6, loop_stfsx_mfvscr(n));
    TIMED(7, loop_mfvscr_8_std(n));
    TIMED(8, loop_mfvscr_8_stfd(n));
    TIMED(9, loop_mfvscr_16_std(n));
    TIMED(10, loop_mfvscr_16_stfd(n));
    TIMED(11, loop_mfvscr_32_std(n));
    TIMED(12, loop_mfvscr_32_stfd(n));

    put(26, REPS);
    put(27, ITER_LOG2);
    put(28, 0x50504656ull);

    write_frame();
    return 0;
}
