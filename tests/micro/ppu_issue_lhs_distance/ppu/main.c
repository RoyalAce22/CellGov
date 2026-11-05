/* PPU probe: how far apart a store and a load of the same bytes must be
 * before the load stops waiting for the store. Each loop runs a fixed
 * body ITER times between two time-base reads. The probe runs each loop
 * REPS times and records the smallest and the largest delta. The loop
 * branch is bdnz.
 *
 * Line A is one 128-byte line; the compiler picks the register that
 * holds it (r6 in the built probe). LI_N is N independent li to
 * r16 through r23 in turn, which nothing in the loop reads. The ld loads
 * r10, which nothing in the loop reads.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0..7    std r9,0(A); LI_N; ld r10,0(A)  for N = 0 1 2 4 6 8 12 16
 *   8..15   stmw r24,0(A); LI_N; lmw r24,0(A)  for the same N
 *  16       std r9,0(A); LI_16
 *  17       stmw r24,0(A); LI_16
 *   word
 *  36  REPS
 *  37  log2 of ITER
 *  38  0x50504c44 ("PPLD"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS     18
#define WORDS     39
#define PAYLOAD   (WORDS * 8)
#define REPS      11
#define ITER_LOG2 16

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 lines[16] __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

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

#define CLOBBER_R16 "r16", "r17", "r18", "r19", "r20", "r21", "r22", "r23"
#define CLOBBER_R24 "r24", "r25", "r26", "r27", "r28", "r29", "r30", "r31"

#define LI_1  "li 16,1\n"
#define LI_2  LI_1 "li 17,1\n"
#define LI_4  LI_2 "li 18,1\n" "li 19,1\n"
#define LI_6  LI_4 "li 20,1\n" "li 21,1\n"
#define LI_8  LI_6 "li 22,1\n" "li 23,1\n"
#define LI_12 LI_8 LI_4
#define LI_16 LI_8 LI_8

#define STD  "std 9,0(%[p])\n"
#define LD   "ld 10,0(%[p])\n"
#define STMW "stmw 24,0(%[p])\n"
#define LMW  "lmw 24,0(%[p])\n"

/* One timed loop of `body` with `p` line A. Returns the delta. */
#define LOOP(name, body)                                           \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            :                                                      \
            : [n] "r"(n), [p] "b"(EA(&lines[0]))                   \
            : "r9", "r10", CLOBBER_R16, CLOBBER_R24, "ctr",        \
              "memory");                                           \
        return timebase() - t0;                                    \
    }

LOOP(loop_std_0, STD LD)
LOOP(loop_std_1, STD LI_1 LD)
LOOP(loop_std_2, STD LI_2 LD)
LOOP(loop_std_4, STD LI_4 LD)
LOOP(loop_std_6, STD LI_6 LD)
LOOP(loop_std_8, STD LI_8 LD)
LOOP(loop_std_12, STD LI_12 LD)
LOOP(loop_std_16, STD LI_16 LD)
LOOP(loop_stmw_0, STMW LMW)
LOOP(loop_stmw_1, STMW LI_1 LMW)
LOOP(loop_stmw_2, STMW LI_2 LMW)
LOOP(loop_stmw_4, STMW LI_4 LMW)
LOOP(loop_stmw_6, STMW LI_6 LMW)
LOOP(loop_stmw_8, STMW LI_8 LMW)
LOOP(loop_stmw_12, STMW LI_12 LMW)
LOOP(loop_stmw_16, STMW LI_16 LMW)
LOOP(loop_std_only, STD LI_16)
LOOP(loop_stmw_only, STMW LI_16)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 16; i++)
        lines[i] = i;

    TIMED(0, loop_std_0(n));
    TIMED(1, loop_std_1(n));
    TIMED(2, loop_std_2(n));
    TIMED(3, loop_std_4(n));
    TIMED(4, loop_std_6(n));
    TIMED(5, loop_std_8(n));
    TIMED(6, loop_std_12(n));
    TIMED(7, loop_std_16(n));
    TIMED(8, loop_stmw_0(n));
    TIMED(9, loop_stmw_1(n));
    TIMED(10, loop_stmw_2(n));
    TIMED(11, loop_stmw_4(n));
    TIMED(12, loop_stmw_6(n));
    TIMED(13, loop_stmw_8(n));
    TIMED(14, loop_stmw_12(n));
    TIMED(15, loop_stmw_16(n));
    TIMED(16, loop_std_only(n));
    TIMED(17, loop_stmw_only(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x50504c44ull);

    write_frame();
    return 0;
}
