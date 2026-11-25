/* PPU probe: how a run of independent vector-scalar ops after an mfvscr
 * issues, and when the next mfvscr issues after it. Each loop runs a
 * fixed body ITER times between two time-base reads, on a 32-byte
 * boundary. The probe runs each loop REPS times and records the smallest
 * and the largest delta. f14 starts zero. Each loop counts in CTR and
 * closes with bdnz.
 *
 * "n vadd" is n independent vaddubm v1,v2,v3, vaddubm v5,v2,v3, ...
 * vaddubm v11,v2,v3, in that order. "n fadd" is n independent fadd f1 to
 * f8 of f14,f14. "vadd v1" is vaddubm v4,v1,v1 to vaddubm v7,v1,v1. "li"
 * is li r3,1.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mfvscr v0
 *   1  mfvscr v0; 1 vadd
 *   2  mfvscr v0; 2 vadd
 *   3  mfvscr v0; 3 vadd
 *   4  mfvscr v0; 4 vadd
 *   5  mfvscr v0; 6 vadd
 *   6  mfvscr v0; 8 vadd
 *   7  mfvscr v0; 2 fadd
 *   8  mfvscr v0; 4 fadd
 *   9  mfvscr v0; 8 fadd
 *  10  mfvscr v3; 4 vadd v1
 *  11  4 vadd
 *  12  mfvscr v0; vaddubm v1; li; vaddubm v5
 *  13  mfvscr v0; vaddubm v1; fadd f1; vaddubm v5; fadd f2
 *   word
 *  28  REPS
 *  29  log2 of ITER
 *  30  0x50505652 ("PPVR"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS        14
#define WORDS        (2 * LOOPS + 3)
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

#define MFVSCR  "mfvscr 0\n"
#define MFVSCR3 "mfvscr 3\n"
#define LI      "li 3,1\n"
#define V1      "vaddubm 1,2,3\n"
#define V2      V1 "vaddubm 5,2,3\n"
#define V3      V2 "vaddubm 6,2,3\n"
#define V4      V3 "vaddubm 7,2,3\n"
#define V6      V4 "vaddubm 8,2,3\n" "vaddubm 9,2,3\n"
#define V8      V6 "vaddubm 10,2,3\n" "vaddubm 11,2,3\n"
#define F2      "fadd 1,14,14\n" "fadd 2,14,14\n"
#define F4      F2 "fadd 3,14,14\n" "fadd 4,14,14\n"
#define F8      F4 "fadd 5,14,14\n" "fadd 6,14,14\n" \
                "fadd 7,14,14\n" "fadd 8,14,14\n"
#define VV1     "vaddubm 4,1,1\n" "vaddubm 5,1,1\n" \
                "vaddubm 6,1,1\n" "vaddubm 7,1,1\n"

/* A loop whose label sits on a 32-byte boundary. */
#define MLOOP(name, body)                                              \
    static u64 __attribute__((noinline)) name(u64 n)                   \
    {                                                                  \
        u64 t0 = timebase();                                           \
        __asm__ volatile (                                             \
            "mtctr %[n]\n"                                             \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                     \
            : : [n] "r"(n)                                             \
            : "r3", "ctr", "fr1", "fr2", "fr3", "fr4", "fr5", "fr6",   \
              "fr7", "fr8", "v0", "v1", "v3", "v4", "v5", "v6", "v7",  \
              "v8", "v9", "v10", "v11");                               \
        return timebase() - t0;                                        \
    }

MLOOP(loop_s, MFVSCR)
MLOOP(loop_s_v1, MFVSCR V1)
MLOOP(loop_s_v2, MFVSCR V2)
MLOOP(loop_s_v3, MFVSCR V3)
MLOOP(loop_s_v4, MFVSCR V4)
MLOOP(loop_s_v6, MFVSCR V6)
MLOOP(loop_s_v8, MFVSCR V8)
MLOOP(loop_s_f2, MFVSCR F2)
MLOOP(loop_s_f4, MFVSCR F4)
MLOOP(loop_s_f8, MFVSCR F8)
MLOOP(loop_s3_vv1, MFVSCR3 VV1)
MLOOP(loop_v4, V4)
MLOOP(loop_s_v_li_v, MFVSCR "vaddubm 1,2,3\n" LI "vaddubm 5,2,3\n")
MLOOP(loop_s_vfvf, MFVSCR "vaddubm 1,2,3\n" "fadd 1,14,14\n"
                          "vaddubm 5,2,3\n" "fadd 2,14,14\n")

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* f14 starts zero. */
    __asm__ volatile ("lfd 14,0(%[p])\n"
                      : : [p] "b"(buf) : "fr14", "memory");

    TIMED(0, loop_s(n));
    TIMED(1, loop_s_v1(n));
    TIMED(2, loop_s_v2(n));
    TIMED(3, loop_s_v3(n));
    TIMED(4, loop_s_v4(n));
    TIMED(5, loop_s_v6(n));
    TIMED(6, loop_s_v8(n));
    TIMED(7, loop_s_f2(n));
    TIMED(8, loop_s_f4(n));
    TIMED(9, loop_s_f8(n));
    TIMED(10, loop_s3_vv1(n));
    TIMED(11, loop_v4(n));
    TIMED(12, loop_s_v_li_v(n));
    TIMED(13, loop_s_vfvf(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x50505652ull);

    write_frame();
    return 0;
}
