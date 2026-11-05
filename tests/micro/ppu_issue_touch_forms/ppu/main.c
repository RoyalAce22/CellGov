/* PPU probe: what the other dcbt and dcbtst forms, a touch of a line the
 * L2 holds, a burst of touches, and dcbst or dcbf of a modified line do
 * to a later load. Each loop runs a fixed body ITER times between two
 * time-base reads, REPS times, and records the smallest and the largest
 * delta. The loop branch is bdnz. r4 walks a ring of one doubleword per
 * 128-byte line, each node holding the address of the next, in line
 * order.
 *
 * Loops 0 to 5 walk an 8 MB ring, so a load reaches memory unless a
 * touch brought its line in. Loops 6 and 7 walk a 256 KB ring, which the
 * L2 holds and the L1 does not. Loops 8 to 10 walk a 16 KB ring, which
 * the L1 holds; each body first stores to the line's second doubleword,
 * so the line is modified when dcbst or dcbf meets it.
 *
 * The dcbt and dcbtst forms with a TH field are written as words:
 *   dcbt   0,r10,TH  = 0x7c00522c | TH << 21
 *   dcbtst 0,r10,TH  = 0x7c0051ec | TH << 21
 *
 *   loop
 *   0  4 x  addi r10,r4,1024; dcbt 0,r10,8;   ld r4,0(r4)        8 MB
 *   1  4 x  addi r10,r4,1024; dcbt 0,r10,10;  ld r4,0(r4)        8 MB
 *   2  4 x  addi r10,r4,1024; dcbtst 0,r10,10; ld r4,0(r4)       8 MB
 *   3  4 x  addi r10,r4,1024; dcbt 0,r10,1;   ld r4,0(r4)        8 MB
 *   4  4 x  addi r10,r4,1024; dcbt 0,r10; addis r11,r4,64;
 *           dcbt 0,r11; ld r4,0(r4)                              8 MB
 *   5  4 x  addi r10,r4,1024; ld r4,0(r4)                        8 MB
 *   6  4 x  addi r10,r4,1024; dcbt 0,r10;     ld r4,0(r4)        256 KB
 *   7  4 x  addi r10,r4,1024; ld r4,0(r4)                        256 KB
 *   8  4 x  std r4,8(r4); dcbst 0,r4; ld r4,0(r4)                16 KB
 *   9  4 x  std r4,8(r4); dcbf 0,r4;  ld r4,0(r4)                16 KB
 *  10  4 x  std r4,8(r4); ld r4,0(r4)                            16 KB
 *
 * Loop 4's second touch is 4 MB ahead, a line the walk loads only after
 * both caches have dropped it, so each load sends two touches to memory.
 * Each ring's pointer carries over from run to run.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   word
 *  22  the sum of the three rings' final pointers (keeps each loop live)
 *  23  REPS
 *  24  log2 of ITER
 *  25  0x5050544f ("PPTO"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     26
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 12

#define LINE      128
#define MEM_BYTES (8u * 1024u * 1024u)
#define L2_BYTES  (256u * 1024u)
#define L1_BYTES  (16u * 1024u)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static unsigned char mem_ring[MEM_BYTES] __attribute__((aligned(128)));
static unsigned char l2_ring[L2_BYTES] __attribute__((aligned(128)));
static unsigned char l1_ring[L1_BYTES] __attribute__((aligned(128)));
static u64 mem_head;
static u64 l2_head;
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

#define AHEAD       "addi 10,4,1024\n"
#define DCBT_TH8    ".long 0x7d00522c\n"
#define DCBT_TH10   ".long 0x7d40522c\n"
#define DCBTST_TH10 ".long 0x7d4051ec\n"
#define DCBT_TH1    ".long 0x7c20522c\n"
#define DCBT_R10    ".long 0x7c00522c\n"
#define DCBT_R11    ".long 0x7c005a2c\n"
#define LD          "ld 4,0(4)\n"
#define STD         "std 4,8(4)\n"

#define CHASE(name, head, body)                                    \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        register u64 p __asm__("4") = head;                        \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : "+r"(p) : [n] "r"(n) : "r10", "r11", "ctr", "memory"); \
        head = p;                                                  \
        return timebase() - t0;                                    \
    }

CHASE(loop_mem_th8, mem_head, R4(AHEAD DCBT_TH8 LD))
CHASE(loop_mem_th10, mem_head, R4(AHEAD DCBT_TH10 LD))
CHASE(loop_mem_tst10, mem_head, R4(AHEAD DCBTST_TH10 LD))
CHASE(loop_mem_th1, mem_head, R4(AHEAD DCBT_TH1 LD))
CHASE(loop_mem_two, mem_head, R4(AHEAD DCBT_R10 "addis 11,4,64\n" DCBT_R11 LD))
CHASE(loop_mem_addi, mem_head, R4(AHEAD LD))
CHASE(loop_l2_dcbt, l2_head, R4(AHEAD DCBT_R10 LD))
CHASE(loop_l2_addi, l2_head, R4(AHEAD LD))
CHASE(loop_l1_dcbst, l1_head, R4(STD "dcbst 0,4\n" LD))
CHASE(loop_l1_dcbf, l1_head, R4(STD "dcbf 0,4\n" LD))
CHASE(loop_l1_std, l1_head, R4(STD LD))

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    mem_head = build_ring(mem_ring, MEM_BYTES);
    l2_head = build_ring(l2_ring, L2_BYTES);
    l1_head = build_ring(l1_ring, L1_BYTES);

    TIMED(0, loop_mem_th8(n));
    TIMED(1, loop_mem_th10(n));
    TIMED(2, loop_mem_tst10(n));
    TIMED(3, loop_mem_th1(n));
    TIMED(4, loop_mem_two(n));
    TIMED(5, loop_mem_addi(n));
    TIMED(6, loop_l2_dcbt(n));
    TIMED(7, loop_l2_addi(n));
    TIMED(8, loop_l1_dcbst(n));
    TIMED(9, loop_l1_dcbf(n));
    TIMED(10, loop_l1_std(n));

    put(22, mem_head + l2_head + l1_head);
    put(23, REPS);
    put(24, ITER_LOG2);
    put(25, 0x5050544full);

    write_frame();
    return 0;
}
