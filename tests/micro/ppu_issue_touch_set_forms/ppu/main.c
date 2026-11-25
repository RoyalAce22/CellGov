/* PPU probe: whether a load that misses the L1, an ldarx, and an
 * integer load across a 32-byte sector wait for or hold an L1 set that
 * touch reloads hold. Each loop runs a fixed body ITER times between
 * two time-base reads, REPS times, and records the smallest and the
 * largest delta. The loop branch is bdnz. r4 walks an 8 MB ring of one
 * doubleword per 128-byte line, each node holding the address of the
 * next, in line order, so a load reaches memory unless a touch brought
 * its line in.
 *
 * r20 holds a 64 KB-aligned 320 KB conflict buffer. The L1 picks a
 * line's set from address bits 7 to 12, and a user page is 64 KB or
 * 1 MB, so the effective and the real address agree in those bits. The
 * prologue P puts in r10 a buffer line in the L1 set of r4's line, in
 * one of four 64 KB groups that r4's address bits 13 and 14 pick. Each
 * group's lines in a set are reached once in four visits to that set,
 * so a touch or a load of one misses the L1. The buffer stays in the
 * L2, so each such touch or load hits the L2.
 *
 *   P    = rlwinm 10,4,0,19,24; rlwinm 11,4,3,14,15;
 *          add 10,10,11; add 10,10,20
 *   S(n) = P; n x (dcbt 0,r10; addi 10,10,8192)
 *          n lines in r4's L1 set
 *   L    = ld 5,0(10); addi 10,10,8192
 *          a load of a line in r10's L1 set; nothing reads r5
 *   Y    = addi 10,10,4096
 *          moves r10 to the L1 set 32 sets from r4's
 *   T(n) = n x (dcbt 0,r10; addi 10,10,8192)
 *          n lines in r10's L1 set
 *
 *   loop
 *   0  S(0);                       ld r4,0(r4)
 *   1  S(1);                       ld r4,0(r4)
 *   2  S(2);                       ld r4,0(r4)
 *   3  P; L;                       ld r4,0(r4)
 *   4  P; L; T(1);                 ld r4,0(r4)
 *   5  S(4);                       ldarx r4,0,r4
 *   6  S(0);                       ldarx r4,0,r4
 *   7  P; Y; T(4); ld 5,28(10);    ld r4,0(r4)
 *   8  P; Y; T(4); ld 5,24(10);    ld r4,0(r4)
 *   9  P; Y;       ld 5,28(10);    ld r4,0(r4)
 *
 * Loops 3 and 4 put a load miss in r4's set ahead of the chase's load,
 * and ahead of one touch there. Loop 7's ld 5 reads bytes 28 to 35 of
 * its line, across a sector; loop 8's reads bytes 24 to 31, in one.
 * Both are in a set that the four touches before them hold, and not in
 * the chase's set. Loop 9's ld 5 crosses a sector in a set no touch
 * holds. The ring's pointer carries over from loop to loop and from run
 * to run.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   word
 *  20  the ring's final pointer (keeps each loop live)
 *  21  REPS
 *  22  log2 of ITER
 *  23  0x50505451 ("PPTQ"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS     10
#define WORDS     (2 * LOOPS + 4)
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 12

#define LINE           128
#define MEM_BYTES      (8u * 1024u * 1024u)
#define CONFLICT_BYTES (320u * 1024u)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static unsigned char mem_ring[MEM_BYTES] __attribute__((aligned(128)));
static unsigned char conflict[CONFLICT_BYTES] __attribute__((aligned(65536)));
static u64 mem_head;

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

/* Link `bytes` at `ring` into one cycle of one doubleword per line, in
 * line order. Returns the address of the first node. */
static u64 build_ring(unsigned char *ring, unsigned bytes)
{
    unsigned n = bytes / LINE;
    unsigned i;
    for (i = 0; i < n; i++) {
        u64 *node = (u64 *)(ring + i * LINE);
        *node = EA(ring + ((i + 1) % n) * LINE);
    }
    return EA(ring);
}

#define LOOP_BEGIN "mtctr %[n]\n" ".p2align 5\n" "1:\n"
#define LOOP_END   "bdnz 1b\n"

#define DCBT_R10 ".long 0x7c00522c\n"
#define LD       "ld 4,0(4)\n"
#define LDARX    "ldarx 4,0,4\n"
#define PROLOGUE                                                   \
    "rlwinm 10,4,0,19,24\n" "rlwinm 11,4,3,14,15\n"                \
    "add 10,10,11\n" "add 10,10,20\n"
#define SAME     DCBT_R10 "addi 10,10,8192\n"
#define SAME2    SAME SAME
#define SAME4    SAME2 SAME2
#define LOAD     "ld 5,0(10)\n" "addi 10,10,8192\n"
#define FAR      "addi 10,10,4096\n"
#define SPLIT    "ld 5,28(10)\n"
#define WHOLE    "ld 5,24(10)\n"

#define S0 PROLOGUE
#define S1 PROLOGUE SAME
#define S2 PROLOGUE SAME2
#define S4 PROLOGUE SAME4

#define CHASE(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        register u64 p __asm__("4") = mem_head;                    \
        register u64 c __asm__("20") = EA(conflict);               \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : "+r"(p) : [n] "r"(n), "r"(c)                         \
            : "r5", "r10", "r11", "ctr", "memory");                \
        mem_head = p;                                              \
        return timebase() - t0;                                    \
    }

CHASE(loop_s0, S0 LD)
CHASE(loop_s1, S1 LD)
CHASE(loop_s2, S2 LD)
CHASE(loop_load, PROLOGUE LOAD LD)
CHASE(loop_load_touch, PROLOGUE LOAD SAME LD)
CHASE(loop_s4_ldarx, S4 LDARX)
CHASE(loop_s0_ldarx, S0 LDARX)
CHASE(loop_far_split, PROLOGUE FAR SAME4 SPLIT LD)
CHASE(loop_far_whole, PROLOGUE FAR SAME4 WHOLE LD)
CHASE(loop_far_split_free, PROLOGUE FAR SPLIT LD)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    mem_head = build_ring(mem_ring, MEM_BYTES);

    TIMED(0, loop_s0(n));
    TIMED(1, loop_s1(n));
    TIMED(2, loop_s2(n));
    TIMED(3, loop_load(n));
    TIMED(4, loop_load_touch(n));
    TIMED(5, loop_s4_ldarx(n));
    TIMED(6, loop_s0_ldarx(n));
    TIMED(7, loop_far_split(n));
    TIMED(8, loop_far_whole(n));
    TIMED(9, loop_far_split_free(n));

    put(2 * LOOPS, mem_head);
    put(2 * LOOPS + 1, REPS);
    put(2 * LOOPS + 2, ITER_LOG2);
    put(2 * LOOPS + 3, 0x50505451ull);

    write_frame();
    return 0;
}
