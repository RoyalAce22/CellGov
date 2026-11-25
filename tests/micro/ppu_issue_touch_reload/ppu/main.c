/* PPU probe: what a second dcbt of a line does to a later load when the
 * first touch's copy has left the L1 and the first touch still brings
 * the line from memory. Each loop runs a fixed body ITER times between
 * two time-base reads, REPS times, and records the smallest and the
 * largest delta. The loop branch is bdnz. r4 walks an 8 MB ring of one
 * doubleword per 128-byte line, each node holding the address of the
 * next, in line order, so a load reaches memory unless a touch brought
 * its line in.
 *
 * r20 holds an 8 KB-aligned 64 KB conflict buffer. The L1 picks a line's
 * set from address bits 7 to 12, and a user page is 64 KB or 1 MB, so
 * the effective and the real address agree in those bits. The conflict
 * part puts in r10 the buffer's line in the L1 set of r4's line, then
 * touches the eight buffer lines 8 KB apart in that set. The conflict
 * buffer stays in the L2, so each of those touches hits the L2 and fills
 * the L1, and eight of them leave no room in the set's four ways for the
 * line r4 touched.
 *
 *   C = rlwinm 10,4,0,19,24; add 10,10,20;
 *       8 x (dcbt 0,r10; addi 10,10,8192)
 *
 *   loop
 *   0  dcbt 0,r4;            ld r4,0(r4)
 *   1  dcbt 0,r4; C; nop;       ld r4,0(r4)
 *   2  dcbt 0,r4; C; dcbt 0,r4; ld r4,0(r4)
 *   3  C;                    ld r4,0(r4)
 *
 * Loops 1 and 2 differ in one word: loop 2 touches r4's line a second
 * time, after C, where loop 1 has a nop. The ring's pointer carries over
 * from run to run.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   word
 *   8  the ring's final pointer (keeps each loop live)
 *   9  REPS
 *  10  log2 of ITER
 *  11  0x50505452 ("PPTR"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     12
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 12

#define LINE           128
#define MEM_BYTES      (8u * 1024u * 1024u)
#define CONFLICT_BYTES (64u * 1024u)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static unsigned char mem_ring[MEM_BYTES] __attribute__((aligned(128)));
static unsigned char conflict[CONFLICT_BYTES] __attribute__((aligned(8192)));
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

#define DCBT_R4  ".long 0x7c00222c\n"
#define DCBT_R10 ".long 0x7c00522c\n"
#define LD       "ld 4,0(4)\n"
#define NOP      "nop\n"
#define TOUCH_NEXT_IN_SET DCBT_R10 "addi 10,10,8192\n"
#define CONFLICTS                                                  \
    "rlwinm 10,4,0,19,24\n" "add 10,10,20\n"                       \
    TOUCH_NEXT_IN_SET TOUCH_NEXT_IN_SET TOUCH_NEXT_IN_SET          \
    TOUCH_NEXT_IN_SET TOUCH_NEXT_IN_SET TOUCH_NEXT_IN_SET          \
    TOUCH_NEXT_IN_SET TOUCH_NEXT_IN_SET

#define CHASE(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        register u64 p __asm__("4") = mem_head;                    \
        register u64 c __asm__("20") = EA(conflict);               \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : "+r"(p) : [n] "r"(n), "r"(c) : "r10", "ctr", "memory"); \
        mem_head = p;                                              \
        return timebase() - t0;                                    \
    }

CHASE(loop_touch, DCBT_R4 LD)
CHASE(loop_conflict_nop, DCBT_R4 CONFLICTS NOP LD)
CHASE(loop_conflict_retouch, DCBT_R4 CONFLICTS DCBT_R4 LD)
CHASE(loop_conflict_only, CONFLICTS LD)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    mem_head = build_ring(mem_ring, MEM_BYTES);

    TIMED(0, loop_touch(n));
    TIMED(1, loop_conflict_nop(n));
    TIMED(2, loop_conflict_retouch(n));
    TIMED(3, loop_conflict_only(n));

    put(8, mem_head);
    put(9, REPS);
    put(10, ITER_LOG2);
    put(11, 0x50505452ull);

    write_frame();
    return 0;
}
