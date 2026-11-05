/* PPU probe: what dcbt, dcbtst and dcbf do to a later load. Each loop
 * runs a fixed body ITER times between two time-base reads, REPS times,
 * and records the smallest and the largest delta. The loop branch is
 * bdnz. r4 walks a ring of one doubleword per 128-byte line, each node
 * holding the address of the next, in line order.
 *
 * Loops 0 to 3 walk an 8 MB ring, so a load reaches memory unless a
 * touch brought its line in. Loops 4 to 6 walk a 16 KB ring, which the
 * L1 holds, so a load hits unless a flush took its line out.
 *
 *   loop
 *   0  8 x  ld r4,0(r4)                                    8 MB ring
 *   1  4 x  addi r10,r4,1024; dcbt 0,r10;   ld r4,0(r4)    8 MB ring
 *   2  4 x  addi r10,r4,1024; dcbtst 0,r10; ld r4,0(r4)    8 MB ring
 *   3  4 x  addi r10,r4,1024; ld r4,0(r4)                  8 MB ring
 *   4  4 x  dcbf 0,r4;  ld r4,0(r4)                        16 KB ring
 *   5  4 x  dcbst 0,r4; ld r4,0(r4)                        16 KB ring
 *   6  8 x  ld r4,0(r4)                                    16 KB ring
 *
 * Each ring's pointer carries over from run to run.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   word
 *  14  the sum of both rings' final pointers (keeps each loop live)
 *  15  REPS
 *  16  log2 of ITER
 *  17  0x50505446 ("PPTF"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     18
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 12

#define LINE      128
#define MEM_BYTES (8u * 1024u * 1024u)
#define L1_BYTES  (16u * 1024u)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static unsigned char mem_ring[MEM_BYTES] __attribute__((aligned(128)));
static unsigned char l1_ring[L1_BYTES] __attribute__((aligned(128)));
static u64 mem_head;
static u64 l1_head;

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

#define R4(x) x x x x
#define R8(x) R4(x) R4(x)

#define CHASE(name, head, body)                                    \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        register u64 p __asm__("4") = head;                        \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : "+r"(p) : [n] "r"(n) : "r10", "ctr", "memory");      \
        head = p;                                                  \
        return timebase() - t0;                                    \
    }

CHASE(loop_mem_ld, mem_head, R8("ld 4,0(4)\n"))
CHASE(loop_mem_dcbt, mem_head, R4("addi 10,4,1024\n" "dcbt 0,10\n" "ld 4,0(4)\n"))
CHASE(loop_mem_dcbtst, mem_head, R4("addi 10,4,1024\n" "dcbtst 0,10\n" "ld 4,0(4)\n"))
CHASE(loop_mem_addi, mem_head, R4("addi 10,4,1024\n" "ld 4,0(4)\n"))
CHASE(loop_l1_dcbf, l1_head, R4("dcbf 0,4\n" "ld 4,0(4)\n"))
CHASE(loop_l1_dcbst, l1_head, R4("dcbst 0,4\n" "ld 4,0(4)\n"))
CHASE(loop_l1_ld, l1_head, R8("ld 4,0(4)\n"))

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    mem_head = build_ring(mem_ring, MEM_BYTES);
    l1_head = build_ring(l1_ring, L1_BYTES);

    TIMED(0, loop_mem_ld(n));
    TIMED(1, loop_mem_dcbt(n));
    TIMED(2, loop_mem_dcbtst(n));
    TIMED(3, loop_mem_addi(n));
    TIMED(4, loop_l1_dcbf(n));
    TIMED(5, loop_l1_dcbst(n));
    TIMED(6, loop_l1_ld(n));

    put(14, mem_head + l1_head);
    put(15, REPS);
    put(16, ITER_LOG2);
    put(17, 0x50505446ull);

    write_frame();
    return 0;
}
