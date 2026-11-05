/* PPU probe: how independent addi before or after a store multiple and a
 * load multiple of the same bytes change the pair's cost. Each loop runs
 * a fixed body ITER times between two time-base reads. The probe runs
 * each loop REPS times and records the smallest and the largest delta.
 * The loop branch is bdnz.
 *
 * Line A is one 128-byte line. Every stmw and lmw is of r24 to r31 at
 * 0 of line A. ADDI_N is N independent addi on r16 to r23, which nothing
 * in the loop reads.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  stmw, lmw
 *   1  stmw, lmw, ADDI_1
 *   2  stmw, lmw, ADDI_2
 *   3  stmw, lmw, ADDI_4
 *   4  stmw, lmw, ADDI_8
 *   5  stmw, lmw, ADDI_16
 *   6  ADDI_8, stmw, lmw
 *   7  stmw, ADDI_8, lmw
 *   8  stmw
 *   9  lmw
 *  10  stmw, ADDI_8
 *  11  stmw, lmw (loop 0 again, last)
 *   word
 *  24  REPS
 *  25  log2 of ITER
 *  26  0x5050504d ("PPPM"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS     12
#define WORDS     27
#define PAYLOAD   (WORDS * 8)
#define REPS      11
#define ITER_LOG2 17

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

#define ADDI_1 "addi 16,16,1\n"
#define ADDI_2 ADDI_1 "addi 17,17,1\n"
#define ADDI_4 ADDI_2 "addi 18,18,1\n" "addi 19,19,1\n"
#define ADDI_8 ADDI_4 "addi 20,20,1\n" "addi 21,21,1\n" "addi 22,22,1\n" "addi 23,23,1\n"
#define ADDI_16 ADDI_8 ADDI_8

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
            : CLOBBER_R16, CLOBBER_R24, "ctr", "memory");          \
        return timebase() - t0;                                    \
    }

LOOP(loop_pair, STMW LMW)
LOOP(loop_pair_addi_1, STMW LMW ADDI_1)
LOOP(loop_pair_addi_2, STMW LMW ADDI_2)
LOOP(loop_pair_addi_4, STMW LMW ADDI_4)
LOOP(loop_pair_addi_8, STMW LMW ADDI_8)
LOOP(loop_pair_addi_16, STMW LMW ADDI_16)
LOOP(loop_addi_8_pair, ADDI_8 STMW LMW)
LOOP(loop_stmw_addi_8_lmw, STMW ADDI_8 LMW)
LOOP(loop_stmw, STMW)
LOOP(loop_lmw, LMW)
LOOP(loop_stmw_addi_8, STMW ADDI_8)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 16; i++)
        lines[i] = i;

    TIMED(0, loop_pair(n));
    TIMED(1, loop_pair_addi_1(n));
    TIMED(2, loop_pair_addi_2(n));
    TIMED(3, loop_pair_addi_4(n));
    TIMED(4, loop_pair_addi_8(n));
    TIMED(5, loop_pair_addi_16(n));
    TIMED(6, loop_addi_8_pair(n));
    TIMED(7, loop_stmw_addi_8_lmw(n));
    TIMED(8, loop_stmw(n));
    TIMED(9, loop_lmw(n));
    TIMED(10, loop_stmw_addi_8(n));
    TIMED(11, loop_pair(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x5050504dull);

    write_frame();
    return 0;
}
