/* PPU probe: whether the load or store part of a floating-point or
 * vector load or store younger than an mfvscr waits for the mfvscr's
 * result, and whether a run of fixed-point writes to one register issues
 * as fast as a run to eight registers for each form of fixed-point write.
 * Each loop runs a fixed body ITER times between two time-base reads.
 * The probe runs each loop REPS times and records the smallest and the
 * largest delta. Every load and store addresses one 128-byte-aligned
 * buffer the probe zeroes first; r29 holds its address. f14 starts zero
 * and r11 holds zero in each loop. Each loop counts in CTR and closes
 * with bdnz.
 *
 * "chain" is addi r4,r29,0 then seven addi r4,r4,1: it reads r29 and
 * then depends on itself. "distinct" cycles the destination r3 to r10.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mfvscr v3; 16 li distinct
 *   1  mfvscr v3; lwz r4,0(r29)
 *   2  mfvscr v3; lfd f1,0(r29)
 *   3  mfvscr v3; lvx v4,0,r29
 *   4  mfvscr v3; stvx v1,0,r29
 *   5  mfvscr v3; addi r29,r29,0; chain
 *   6  mfvscr v3; lfdu f1,0(r29); chain
 *   7  mfvscr v3; stfdu f14,0(r29); chain
 *   8  32 add r10,r11,r11
 *   9  32 add distinct,r11,r11
 *  10  32 add. r10,r11,r11
 *  11  32 addic r10,r11,1
 *  12  32 rlwinm r10,r11,1,0,31
 *  13  32 srawi r10,r11,1
 *   word
 *  28  REPS
 *  29  log2 of ITER
 *  30  0x50504d4c ("PPML"): the probe wrote every word
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

#define MFVSCR "mfvscr 3\n"
#define D8     "li 3,1\n" "li 4,1\n" "li 5,1\n" "li 6,1\n" \
               "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n"
#define D16    D8 D8
#define CHAIN  "addi 4,29,0\n" \
               "addi 4,4,1\n" "addi 4,4,1\n" "addi 4,4,1\n" "addi 4,4,1\n" \
               "addi 4,4,1\n" "addi 4,4,1\n" "addi 4,4,1\n"
#define X4(s)  s s s s
#define X32(s) X4(X4(s)) X4(X4(s))
#define ADD_D  "add 3,11,11\n" "add 4,11,11\n" "add 5,11,11\n" "add 6,11,11\n" \
               "add 7,11,11\n" "add 8,11,11\n" "add 9,11,11\n" "add 10,11,11\n"

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "mr 29,%[p]\n" "li 11,0\n" "mtctr %[n]\n"              \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                 \
            : : [n] "r"(n), [p] "r"(buf)                           \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",     \
              "r11", "r29", "ctr", "xer", "cr0", "memory",         \
              "fr1", "v3", "v4");                                  \
        return timebase() - t0;                                    \
    }

MLOOP(loop_mfvscr_d16, MFVSCR D16)
MLOOP(loop_mfvscr_lwz, MFVSCR "lwz 4,0(29)\n")
MLOOP(loop_mfvscr_lfd, MFVSCR "lfd 1,0(29)\n")
MLOOP(loop_mfvscr_lvx, MFVSCR "lvx 4,0,29\n")
MLOOP(loop_mfvscr_stvx, MFVSCR "stvx 1,0,29\n")
MLOOP(loop_mfvscr_addi_chain, MFVSCR "addi 29,29,0\n" CHAIN)
MLOOP(loop_mfvscr_lfdu_chain, MFVSCR "lfdu 1,0(29)\n" CHAIN)
MLOOP(loop_mfvscr_stfdu_chain, MFVSCR "stfdu 14,0(29)\n" CHAIN)
MLOOP(loop_add_same, X32("add 10,11,11\n"))
MLOOP(loop_add_distinct, X4(ADD_D))
MLOOP(loop_add_dot_same, X32("add. 10,11,11\n"))
MLOOP(loop_addic_same, X32("addic 10,11,1\n"))
MLOOP(loop_rlwinm_same, X32("rlwinm 10,11,1,0,31\n"))
MLOOP(loop_srawi_same, X32("srawi 10,11,1\n"))

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* f14 starts zero. */
    __asm__ volatile (
        "lfd 14,0(%[p])\n"
        : : [p] "b"(buf) : "fr14", "memory");

    TIMED(0, loop_mfvscr_d16(n));
    TIMED(1, loop_mfvscr_lwz(n));
    TIMED(2, loop_mfvscr_lfd(n));
    TIMED(3, loop_mfvscr_lvx(n));
    TIMED(4, loop_mfvscr_stvx(n));
    TIMED(5, loop_mfvscr_addi_chain(n));
    TIMED(6, loop_mfvscr_lfdu_chain(n));
    TIMED(7, loop_mfvscr_stfdu_chain(n));
    TIMED(8, loop_add_same(n));
    TIMED(9, loop_add_distinct(n));
    TIMED(10, loop_add_dot_same(n));
    TIMED(11, loop_addic_same(n));
    TIMED(12, loop_rlwinm_same(n));
    TIMED(13, loop_srawi_same(n));

    put(28, REPS);
    put(29, ITER_LOG2);
    put(30, 0x50504d4cull);

    write_frame();
    return 0;
}
