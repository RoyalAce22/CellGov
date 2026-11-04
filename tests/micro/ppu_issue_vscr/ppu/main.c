/* PPU probe: what mfvscr and mtvscr cost at issue. Each loop runs a
 * fixed body ITER times between two time-base reads. The probe runs each
 * loop REPS times and records the smallest and the largest delta. The
 * loop branch is bdnz. Every vector register starts zero, so mtvscr v0
 * writes a zero VSCR.
 *
 * Loops 1 and 3 time each instruction alone, loop 2 the mfvscr result's
 * latency, loops 4 and 5 what an mtvscr costs younger vector and
 * fixed-point instructions, and loop 6 an mtvscr after older vector
 * work. Loops 0 and 7 are the vector baselines.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  8 independent vaddubm
 *   1  mfvscr v1
 *   2  mfvscr v1; vaddubm v2,v1,v1
 *   3  mtvscr v0
 *   4  mtvscr v0; 4 independent vaddubm
 *   5  mtvscr v0; 4 independent addi
 *   6  4 independent vaddubm; mtvscr v0
 *   7  4 independent vaddubm
 *   word
 *  16  REPS
 *  17  log2 of ITER
 *  18  0x50505653 ("PPVS"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     19
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 18

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

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

#define VADD4                                                   \
    "vaddubm 1,20,21\n" "vaddubm 2,20,21\n"                     \
    "vaddubm 3,20,21\n" "vaddubm 4,20,21\n"
#define VADD8 VADD4                                             \
    "vaddubm 5,20,21\n" "vaddubm 6,20,21\n"                     \
    "vaddubm 7,20,21\n" "vaddubm 8,20,21\n"
#define ADDI4                                                   \
    "addi 14,14,1\n" "addi 15,15,1\n"                           \
    "addi 16,16,1\n" "addi 17,17,1\n"

#define VCLOBBER "v0", "v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8", \
                 "v20", "v21"

#define VLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : : [n] "r"(n)                                         \
            : "r14", "r15", "r16", "r17", "ctr", VCLOBBER);        \
        return timebase() - t0;                                    \
    }

VLOOP(loop_vadd8, VADD8)
VLOOP(loop_mfvscr, "mfvscr 1\n")
VLOOP(loop_mfvscr_use, "mfvscr 1\n" "vaddubm 2,1,1\n")
VLOOP(loop_mtvscr, "mtvscr 0\n")
VLOOP(loop_mtvscr_vadd, "mtvscr 0\n" VADD4)
VLOOP(loop_mtvscr_addi, "mtvscr 0\n" ADDI4)
VLOOP(loop_vadd_mtvscr, VADD4 "mtvscr 0\n")
VLOOP(loop_vadd4, VADD4)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    /* Every vector register the loops read starts zero. */
    __asm__ volatile (
        "vxor 0,0,0\n" "vxor 1,1,1\n" "vxor 20,20,20\n" "vxor 21,21,21\n"
        : : : "v0", "v1", "v20", "v21");

    TIMED(0, loop_vadd8(n));
    TIMED(1, loop_mfvscr(n));
    TIMED(2, loop_mfvscr_use(n));
    TIMED(3, loop_mtvscr(n));
    TIMED(4, loop_mtvscr_vadd(n));
    TIMED(5, loop_mtvscr_addi(n));
    TIMED(6, loop_vadd_mtvscr(n));
    TIMED(7, loop_vadd4(n));

    put(16, REPS);
    put(17, ITER_LOG2);
    put(18, 0x50505653ull);

    write_frame();
    return 0;
}
