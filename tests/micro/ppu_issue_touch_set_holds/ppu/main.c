/* PPU probe: how long a load that misses the L1 and an ldarx hold their
 * L1 set, and how long a load that waits for a set that touch reloads
 * hold holds the instructions after it. Each loop runs a fixed body
 * ITER times between two time-base reads, REPS times, and records the
 * smallest and the largest delta. The loop branch is bdnz. r4 walks an
 * 8 MB ring of one doubleword per 128-byte line, each node holding the
 * address of the next, in line order, so a load reaches memory unless a
 * touch brought its line in. The ring's buffer runs 4 MB and 64 KB past
 * the ring, so r4 + 4 MB + 4 KB is always in it.
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
 * r12 = r4 + 4 MB is a line in r4's L1 set that the walk last read
 * 4 MB of lines ago, so neither cache holds it and a load of it reaches
 * memory.
 *
 *   P    = rlwinm 10,4,0,19,24; rlwinm 11,4,3,14,15;
 *          add 10,10,11; add 10,10,20
 *   S(n) = P; n x (dcbt 0,r10; addi 10,10,8192)
 *          n lines in r4's L1 set
 *   Y    = addi 10,10,4096
 *          moves r10 to the L1 set 32 sets from r4's
 *   T(4) = 4 x (dcbt 0,r10; addi 10,10,8192)
 *          4 lines in r10's L1 set
 *   M    = addis 12,4,64
 *   MY   = addis 12,4,64; addi 12,12,4096
 *          a memory line 32 sets from r4's, in r10's set after P; Y
 *
 *   loop
 *   0  S(0);                     ld r4,0(r4)
 *   1  S(4);                     ld r4,0(r4)
 *   2  P; Y; T(4); ld 5,24(10);  ld r4,0(r4)
 *   3  MY; P; Y; T(4); ld 5,0(12); ld r4,0(r4)
 *   4  M;  ld 5,0(12);           ld r4,0(r4)
 *   5  MY; ld 5,0(12);           ld r4,0(r4)
 *   6  P;  ldarx 5,0,10;         ld r4,0(r4)
 *   7  P;  ld 5,0(10);           ld r4,0(r4)
 *
 * Nothing reads r5. Loops 2 and 3 put an L2-held and a memory line's
 * load in a set that four touches hold, ahead of the chase in another
 * set. Loops 4 and 6 put a memory line's load and an L2-held line's
 * ldarx in the chase's set ahead of it, and loop 7 an L2-held line's
 * load. Loop 5 puts the memory line's load in a set no touch holds. The
 * ring's pointer carries over from loop to loop and from run to run.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   word
 *  16  the ring's final pointer (keeps each loop live)
 *  17  REPS
 *  18  log2 of ITER
 *  19  0x50505448 ("PPTH"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS     8
#define WORDS     (2 * LOOPS + 4)
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 12

#define LINE           128
#define MEM_BYTES      (8u * 1024u * 1024u)
#define MEM_PAST       (4u * 1024u * 1024u + 64u * 1024u)
#define CONFLICT_BYTES (320u * 1024u)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static unsigned char mem_ring[MEM_BYTES + MEM_PAST] __attribute__((aligned(128)));
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
#define PROLOGUE                                                   \
    "rlwinm 10,4,0,19,24\n" "rlwinm 11,4,3,14,15\n"                \
    "add 10,10,11\n" "add 10,10,20\n"
#define SAME     DCBT_R10 "addi 10,10,8192\n"
#define SAME2    SAME SAME
#define SAME4    SAME2 SAME2
#define FAR      "addi 10,10,4096\n"
#define MEM      "addis 12,4,64\n"
#define MEM_FAR  "addis 12,4,64\n" "addi 12,12,4096\n"
#define LD_MEM   "ld 5,0(12)\n"
#define LD_L2    "ld 5,0(10)\n"
#define LD_L2_24 "ld 5,24(10)\n"
#define LDARX_L2 "ldarx 5,0,10\n"

#define CHASE(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        register u64 p __asm__("4") = mem_head;                    \
        register u64 c __asm__("20") = EA(conflict);               \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : "+r"(p) : [n] "r"(n), "r"(c)                         \
            : "r5", "r10", "r11", "r12", "ctr", "memory");         \
        mem_head = p;                                              \
        return timebase() - t0;                                    \
    }

CHASE(loop_s0, PROLOGUE LD)
CHASE(loop_s4, PROLOGUE SAME4 LD)
CHASE(loop_far_l2, PROLOGUE FAR SAME4 LD_L2_24 LD)
CHASE(loop_far_mem, MEM_FAR PROLOGUE FAR SAME4 LD_MEM LD)
CHASE(loop_mem, MEM LD_MEM LD)
CHASE(loop_mem_free, MEM_FAR LD_MEM LD)
CHASE(loop_ldarx, PROLOGUE LDARX_L2 LD)
CHASE(loop_l2, PROLOGUE LD_L2 LD)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    mem_head = build_ring(mem_ring, MEM_BYTES);

    TIMED(0, loop_s0(n));
    TIMED(1, loop_s4(n));
    TIMED(2, loop_far_l2(n));
    TIMED(3, loop_far_mem(n));
    TIMED(4, loop_mem(n));
    TIMED(5, loop_mem_free(n));
    TIMED(6, loop_ldarx(n));
    TIMED(7, loop_l2(n));

    put(2 * LOOPS, mem_head);
    put(2 * LOOPS + 1, REPS);
    put(2 * LOOPS + 2, ITER_LOG2);
    put(2 * LOOPS + 3, 0x50505448ull);

    write_frame();
    return 0;
}
