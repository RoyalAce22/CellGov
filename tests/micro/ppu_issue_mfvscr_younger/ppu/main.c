/* PPU probe: when the instructions younger than an mfvscr issue, and how
 * fast a run of li issues with and without a write to one register.
 * Each loop runs a fixed body ITER times between two time-base reads.
 * The probe runs each loop REPS times and records the smallest and the
 * largest delta. Every store addresses one 128-byte-aligned buffer the
 * probe zeroes first; r29 holds its address. f14 and r14 start zero.
 * Each loop counts in CTR and closes with bdnz.
 *
 * "li same" is li r10,1 repeated; "li distinct" cycles li r3,1 to
 * li r10,1.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  32 li same
 *   1  32 li distinct
 *   2  mfvscr v3; 8 li distinct
 *   3  mfvscr v3; 16 li distinct
 *   4  mfvscr v3; 32 li distinct
 *   5  mfvscr v3; 48 li distinct
 *   6  mfvscr v3; 64 li distinct
 *   7  mfvscr v3; 16 li same
 *   8  mfvscr v3; 48 li same
 *   9  mfvscr v3; 4 independent vaddubm
 *  10  mfvscr v3; 16 li distinct; stfd f14,0(r29)
 *  11  mfvscr v3; 48 li distinct; stfd f14,0(r29)
 *  12  mfvscr v3; 48 li distinct; std r14,0(r29)
 *  13  mfvscr v3; 1 li distinct; std r14,0(r29)
 *   word
 *  28  REPS
 *  29  log2 of ITER
 *  30  0x50504d59 ("PPMY"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS        31
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
#define MFVSCR "mfvscr 3\n"
#define S1     "li 10,1\n"
#define S8     S1 S1 S1 S1 S1 S1 S1 S1
#define S16    S8 S8
#define S32    S16 S16
#define S48    S32 S16
#define D1     "li 3,1\n"
#define D8     "li 3,1\n" "li 4,1\n" "li 5,1\n" "li 6,1\n" \
               "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n"
#define D16    D8 D8
#define D32    D16 D16
#define D48    D32 D16
#define D64    D32 D32
#define VADD4  "vaddubm 4,1,1\n" "vaddubm 5,1,1\n" \
               "vaddubm 6,1,1\n" "vaddubm 7,1,1\n"

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "mr 29,%[p]\n" "mtctr %[n]\n"                          \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                 \
            : : [n] "r"(n), [p] "r"(buf)                           \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",     \
              "r29", "ctr", "memory", "v3", "v4", "v5", "v6", "v7"); \
        return timebase() - t0;                                    \
    }

MLOOP(loop_same_32, S32)
MLOOP(loop_distinct_32, D32)
MLOOP(loop_mfvscr_d8, MFVSCR D8)
MLOOP(loop_mfvscr_d16, MFVSCR D16)
MLOOP(loop_mfvscr_d32, MFVSCR D32)
MLOOP(loop_mfvscr_d48, MFVSCR D48)
MLOOP(loop_mfvscr_d64, MFVSCR D64)
MLOOP(loop_mfvscr_s16, MFVSCR S16)
MLOOP(loop_mfvscr_s48, MFVSCR S48)
MLOOP(loop_mfvscr_vadd4, MFVSCR VADD4)
MLOOP(loop_mfvscr_d16_stfd, MFVSCR D16 STFD)
MLOOP(loop_mfvscr_d48_stfd, MFVSCR D48 STFD)
MLOOP(loop_mfvscr_d48_std, MFVSCR D48 STD)
MLOOP(loop_mfvscr_d1_std, MFVSCR D1 STD)

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

    TIMED(0, loop_same_32(n));
    TIMED(1, loop_distinct_32(n));
    TIMED(2, loop_mfvscr_d8(n));
    TIMED(3, loop_mfvscr_d16(n));
    TIMED(4, loop_mfvscr_d32(n));
    TIMED(5, loop_mfvscr_d48(n));
    TIMED(6, loop_mfvscr_d64(n));
    TIMED(7, loop_mfvscr_s16(n));
    TIMED(8, loop_mfvscr_s48(n));
    TIMED(9, loop_mfvscr_vadd4(n));
    TIMED(10, loop_mfvscr_d16_stfd(n));
    TIMED(11, loop_mfvscr_d48_stfd(n));
    TIMED(12, loop_mfvscr_d48_std(n));
    TIMED(13, loop_mfvscr_d1_std(n));

    put(28, REPS);
    put(29, ITER_LOG2);
    put(30, 0x50504d59ull);

    write_frame();
    return 0;
}
