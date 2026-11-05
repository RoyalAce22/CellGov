/* PPU probe: what an mfxer costs at issue, alone and beside fixed-point
 * work, with mtxer, a carry-writing addic and mfcr beside it. Each loop
 * runs a fixed body ITER times between two time-base reads. The probe
 * runs each loop REPS times and records the smallest and the largest
 * delta. r14 starts zero, so mtxer r14 writes a zero XER. Each loop
 * counts in CTR and closes with bdnz.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  8 mfxer, to r3 .. r10
 *   1  8 mfxer r10
 *   2  mfxer r3; 8 li, to r4 .. r11
 *   3  mfxer r3; addi r4,r3,1
 *   4  16 li, to r4 .. r11
 *   5  mtxer r14; mfxer r3
 *   6  addic r4,r4,1; mfxer r3
 *   7  8 mfcr, to r3 .. r10
 *   8  8 mtxer r14
 *   9  4 x (mullw r4,r5,r6; mfxer r3)
 *   word
 *  20  REPS
 *  21  log2 of ITER
 *  22  0x50505852 ("PPXR"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS        23
#define PAYLOAD      (WORDS * 8)
#define REPS         5
#define ITER_LOG2    16

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

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

#define MFXER8 "mfxer 3\n" "mfxer 4\n" "mfxer 5\n" "mfxer 6\n" \
               "mfxer 7\n" "mfxer 8\n" "mfxer 9\n" "mfxer 10\n"
#define MFXER_SAME8 "mfxer 10\n" "mfxer 10\n" "mfxer 10\n" "mfxer 10\n" \
                    "mfxer 10\n" "mfxer 10\n" "mfxer 10\n" "mfxer 10\n"
#define LI8   "li 4,1\n" "li 5,1\n" "li 6,1\n" "li 7,1\n" \
              "li 8,1\n" "li 9,1\n" "li 10,1\n" "li 11,1\n"
#define MFCR8 "mfcr 3\n" "mfcr 4\n" "mfcr 5\n" "mfcr 6\n" \
              "mfcr 7\n" "mfcr 8\n" "mfcr 9\n" "mfcr 10\n"
#define MTXER8 "mtxer 14\n" "mtxer 14\n" "mtxer 14\n" "mtxer 14\n" \
               "mtxer 14\n" "mtxer 14\n" "mtxer 14\n" "mtxer 14\n"
#define MULXER "mullw 4,5,6\n" "mfxer 3\n"

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "mtctr %[n]\n"                                         \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                 \
            : : [n] "r"(n)                                         \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",     \
              "r11", "ctr", "xer", "cr0");                         \
        return timebase() - t0;                                    \
    }

MLOOP(loop_mfxer8, MFXER8)
MLOOP(loop_mfxer_same8, MFXER_SAME8)
MLOOP(loop_mfxer_li8, "mfxer 3\n" LI8)
MLOOP(loop_mfxer_use, "mfxer 3\n" "addi 4,3,1\n")
MLOOP(loop_li16, LI8 LI8)
MLOOP(loop_mtxer_mfxer, "mtxer 14\n" "mfxer 3\n")
MLOOP(loop_addic_mfxer, "addic 4,4,1\n" "mfxer 3\n")
MLOOP(loop_mfcr8, MFCR8)
MLOOP(loop_mtxer8, MTXER8)
MLOOP(loop_mullw_mfxer, MULXER MULXER MULXER MULXER)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    /* r14 starts zero. */
    __asm__ volatile ("li 14,0\n" : : : "r14");

    TIMED(0, loop_mfxer8(n));
    TIMED(1, loop_mfxer_same8(n));
    TIMED(2, loop_mfxer_li8(n));
    TIMED(3, loop_mfxer_use(n));
    TIMED(4, loop_li16(n));
    TIMED(5, loop_mtxer_mfxer(n));
    TIMED(6, loop_addic_mfxer(n));
    TIMED(7, loop_mfcr8(n));
    TIMED(8, loop_mtxer8(n));
    TIMED(9, loop_mullw_mfxer(n));

    put(20, REPS);
    put(21, ITER_LOG2);
    put(22, 0x50505852ull);

    write_frame();
    return 0;
}
