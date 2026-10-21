/* PPU probe: what a store multiple and a load multiple cost, alone and
 * together. Each loop runs a fixed body ITER times between two time-base
 * reads. The probe runs each loop REPS times and records the smallest
 * and the largest delta. The loop branch is bdnz.
 *
 * Loops 0 to 5 time stmw and lmw alone at 4, 8 and 16 words. Loops 6, 8
 * and 9 pair them on one 128-byte line, and loop 7 pairs them on two
 * lines, so a load waiting on the stores it reads parts from a store
 * queue that drains before the loads start. Loop 10 follows an 8-word
 * stmw with one ld of its first word.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1. The words from 22 on are listed after the
 * loops.
 *   loop
 *   0  stmw r28 (4 words)                     (1 << 20 iterations)
 *   1  stmw r24 (8 words)                     (1 << 20)
 *   2  stmw r16 (16 words)                    (1 << 19)
 *   3  lmw r28 (4 words)                      (1 << 20)
 *   4  lmw r24 (8 words)                      (1 << 20)
 *   5  lmw r16 (16 words)                     (1 << 19)
 *   6  stmw r24, lmw r24, one line            (1 << 18)
 *   7  stmw r24 to one line, lmw r24 from another       (1 << 18)
 *   8  stmw r28, lmw r28, one line            (1 << 18)
 *   9  stmw r16, lmw r16, one line            (1 << 17)
 *  10  stmw r24, then ld of its first word    (1 << 18)
 *   word
 *  22  the sum of loop 10's loaded words (keeps it live)
 *  23  REPS
 *  24  0x5050534d ("PPSM"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS   25
#define PAYLOAD (WORDS * 8)
#define REPS    5

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int lines[64] __attribute__((aligned(128)));
static volatile u64 sink;

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

/* One timed loop of `body` with `p` the first line and `q` the second. */
#define MULTIPLE(name, body)                                       \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : : [n] "r"(n), [p] "b"(EA(&lines[0])),                \
                [q] "b"(EA(&lines[32]))                            \
            : CLOBBER_R16, CLOBBER_R24, "ctr", "memory");          \
        return timebase() - t0;                                    \
    }

MULTIPLE(loop_stmw_4, "stmw 28,0(%[p])\n")
MULTIPLE(loop_stmw_8, "stmw 24,0(%[p])\n")
MULTIPLE(loop_stmw_16, "stmw 16,0(%[p])\n")
MULTIPLE(loop_lmw_4, "lmw 28,0(%[p])\n")
MULTIPLE(loop_lmw_8, "lmw 24,0(%[p])\n")
MULTIPLE(loop_lmw_16, "lmw 16,0(%[p])\n")
MULTIPLE(loop_pair_8, "stmw 24,0(%[p])\n" "lmw 24,0(%[p])\n")
MULTIPLE(loop_pair_8_two_lines, "stmw 24,0(%[p])\n" "lmw 24,0(%[q])\n")
MULTIPLE(loop_pair_4, "stmw 28,0(%[p])\n" "lmw 28,0(%[p])\n")
MULTIPLE(loop_pair_16, "stmw 16,0(%[p])\n" "lmw 16,0(%[p])\n")

static u64 __attribute__((noinline)) loop_stmw_ld(u64 n)
{
    u64 acc = 0;
    u64 t0 = timebase();
    __asm__ volatile (
        LOOP_BEGIN
        "stmw 24,0(%[p])\n"
        "lwz 9,0(%[p])\n"
        "add %[acc],%[acc],9\n"
        LOOP_END
        : [acc] "+r"(acc)
        : [n] "r"(n), [p] "b"(EA(&lines[0]))
        : CLOBBER_R24, "r9", "ctr", "memory");
    sink += acc;
    return timebase() - t0;
}

int main(void)
{
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 64; i++)
        lines[i] = i;

    TIMED(0, loop_stmw_4(1u << 20));
    TIMED(1, loop_stmw_8(1u << 20));
    TIMED(2, loop_stmw_16(1u << 19));
    TIMED(3, loop_lmw_4(1u << 20));
    TIMED(4, loop_lmw_8(1u << 20));
    TIMED(5, loop_lmw_16(1u << 19));
    TIMED(6, loop_pair_8(1u << 18));
    TIMED(7, loop_pair_8_two_lines(1u << 18));
    TIMED(8, loop_pair_4(1u << 18));
    TIMED(9, loop_pair_16(1u << 17));
    TIMED(10, loop_stmw_ld(1u << 18));
    put(22, sink);
    put(23, REPS);
    put(24, 0x5050534dull);

    write_frame();
    return 0;
}
