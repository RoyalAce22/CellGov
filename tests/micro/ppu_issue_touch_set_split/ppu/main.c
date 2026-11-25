/* PPU probe: whether an integer load across a 32-byte sector that waits
 * for an L1 set that touch reloads hold holds the instructions after it
 * for that wait. Each loop runs a fixed body ITER times between two
 * time-base reads, REPS times, and records the smallest and the largest
 * delta. The loop branch is bdnz. No load in a loop waits for another:
 * r23 counts up 128 a pass, and picks the pass's L1 set and buffer
 * group, so each pass works in the set after the last pass's.
 *
 * r20 holds a 64 KB-aligned 320 KB conflict buffer. The L1 picks a
 * line's set from address bits 7 to 12, and a user page is 64 KB or
 * 1 MB, so the effective and the real address agree in those bits. The
 * prologue P puts in r10 a buffer line in the L1 set that r23's bits 7
 * to 12 pick, in one of four 64 KB groups that r23's bits 13 and 14
 * pick. A pass reaches three lines of its group in that set, and the
 * other groups' passes in the set reach nine more before the group's
 * next, so each touch and load misses the L1. The buffer stays in the
 * L2, so each touch and load hits the L2.
 *
 *   P    = rlwinm 10,23,0,19,24; rlwinm 11,23,3,14,15;
 *          add 10,10,11; add 10,10,20
 *   T(2) = 2 x (dcbt 0,r10; addi 10,10,8192)
 *          lines 0 and 1 of the group in the set
 *   W    = 32 x (addi 24,24,1; addi 25,25,1; addi 26,26,1;
 *                addi 27,27,1)
 *   N    = addi 23,23,128
 *
 *   loop
 *   0  P; T(2); ld 5,28(10);    W; N
 *   1  P; T(2); ld 5,24(10);    W; N
 *   2  P; ld 5,16412(10); T(2); W; N
 *   3  P; ld 5,16408(10); T(2); W; N
 *
 * Each ld reads line 2 of the group in the set: bytes 28 to 35 (across
 * a sector) in loops 0 and 2, and bytes 24 to 31 (in one) in loops 1
 * and 3. In loops 0 and 1 the two touches before the load hold its set;
 * in loops 2 and 3 they come after it. Nothing reads r5. Loop 0 less
 * loop 1 is what the sector costs after the set's wait; loop 2 less
 * loop 3 is what it costs with no wait.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   word
 *   8  r23 after the last pass, plus r24 to r27 (keeps each loop live)
 *   9  REPS
 *  10  log2 of ITER
 *  11  0x50505458 ("PPTX"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS     4
#define WORDS     (2 * LOOPS + 4)
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 12

#define CONFLICT_BYTES (320u * 1024u)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static unsigned char conflict[CONFLICT_BYTES] __attribute__((aligned(65536)));
static u64 pass;
static u64 live;

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

#define DCBT_R10 ".long 0x7c00522c\n"
#define PROLOGUE                                                   \
    "rlwinm 10,23,0,19,24\n" "rlwinm 11,23,3,14,15\n"              \
    "add 10,10,11\n" "add 10,10,20\n"
#define SAME     DCBT_R10 "addi 10,10,8192\n"
#define T2       SAME SAME
#define W1       "addi 24,24,1\n" "addi 25,25,1\n"                 \
                 "addi 26,26,1\n" "addi 27,27,1\n"
#define W2       W1 W1
#define W4       W2 W2
#define W8       W4 W4
#define W16      W8 W8
#define W        W16 W16
#define NEXT     "addi 23,23,128\n"
#define SPLIT    "ld 5,28(10)\n"
#define WHOLE    "ld 5,24(10)\n"
#define SPLIT2   "ld 5,16412(10)\n"
#define WHOLE2   "ld 5,16408(10)\n"

#define PASSES(name, body)                                         \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        register u64 q __asm__("23") = pass;                       \
        register u64 c __asm__("20") = EA(conflict);               \
        register u64 a __asm__("24") = 0;                          \
        register u64 b __asm__("25") = 0;                          \
        register u64 d __asm__("26") = 0;                          \
        register u64 e __asm__("27") = 0;                          \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : "+r"(q), "+r"(a), "+r"(b), "+r"(d), "+r"(e)          \
            : [n] "r"(n), "r"(c)                                   \
            : "r5", "r10", "r11", "ctr", "memory");                \
        u64 t1 = timebase();                                       \
        pass = q;                                                  \
        live += a + b + d + e;                                     \
        return t1 - t0;                                            \
    }

PASSES(loop_held_split, PROLOGUE T2 SPLIT W NEXT)
PASSES(loop_held_whole, PROLOGUE T2 WHOLE W NEXT)
PASSES(loop_free_split, PROLOGUE SPLIT2 T2 W NEXT)
PASSES(loop_free_whole, PROLOGUE WHOLE2 T2 W NEXT)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < CONFLICT_BYTES; i += 128)
        conflict[i] = (unsigned char)i;

    TIMED(0, loop_held_split(n));
    TIMED(1, loop_held_whole(n));
    TIMED(2, loop_free_split(n));
    TIMED(3, loop_free_whole(n));

    put(2 * LOOPS, pass + live);
    put(2 * LOOPS + 1, REPS);
    put(2 * LOOPS + 2, ITER_LOG2);
    put(2 * LOOPS + 3, 0x50505458ull);

    write_frame();
    return 0;
}
