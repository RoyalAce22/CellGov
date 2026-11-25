/* PPU probe: how many later touches of other lines in a load's L1 set
 * the load waits behind after a touch of its own line. The
 * ppu_issue_touch_set_pressure capture has the load wait behind 3 or 4
 * such touches and not behind 7 or 8. This probe times each count from
 * 0 to 8, the same counts with a second touch of the load's line, the
 * counts without the first touch, a wait that lets the first touch
 * return before the later touches, touches in other sets after the
 * later touches, and later touches spaced apart.
 *
 * Each loop runs a fixed body ITER times between two time-base reads,
 * REPS times, and records the smallest and the largest delta. The loop
 * branch is bdnz. r4 walks an 8 MB ring of one doubleword per 128-byte
 * line, each node holding the address of the next, in line order, so a
 * load reaches memory unless a touch brought its line in.
 *
 * r20 holds a 64 KB-aligned 320 KB conflict buffer, laid out as in the
 * ppu_issue_touch_set_pressure probe. The L1 picks a line's set from
 * address bits 7 to 12, and a user page is 64 KB or 1 MB, so the
 * effective and the real address agree in those bits. The prologue P
 * puts in r10 a buffer line in the L1 set of r4's line, in one of four
 * 64 KB groups that r4's address bits 13 and 14 pick. Each group's
 * lines in a set are touched once in four visits to that set, so a
 * touch misses the L1. The buffer stays in the L2, so each touch hits
 * the L2.
 *
 * r12 and r15 hold 1, and W(m) is m dependent `mulld 12,12,15`.
 *
 *   T     = dcbt 0,r4: the line the next load reads
 *   P     = rlwinm 10,4,0,19,24; rlwinm 11,4,3,14,15;
 *           add 10,10,11; add 10,10,20
 *   S(n)  = P; n x (dcbt 0,r10; addi 10,10,8192)
 *           n lines in r4's L1 set
 *   S'(n) = P; n x (dcbt 0,r10; addi 10,10,8192; W(2))
 *           S(n) with two mulld after each touch
 *   X(4)  = addi 10,10,8320; 4 x (dcbt 0,r10; addi 10,10,8320)
 *           4 lines in the 4 L1 sets after r4's
 *
 *   loop
 *   0-8   T; S(n); nop;      ld r4,0(r4)    n = 0, 1, ..., 8
 *   9-12  T; S(n); T;        ld r4,0(r4)    n = 3, 4, 5, 6
 *   13-16 S(n);              ld r4,0(r4)    n = 3, 5, 6, 7
 *   17    T; W(40);          ld r4,0(r4)
 *   18    T; W(40); S(3); nop; ld r4,0(r4)
 *   19    T; W(40); S(7); nop; ld r4,0(r4)
 *   20    T; S(3); X(4); nop; ld r4,0(r4)
 *   21    T; S'(4); nop;     ld r4,0(r4)
 *   22    T; S'(7); nop;     ld r4,0(r4)
 *
 * The ring's pointer carries over from run to run.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   word
 *  46  the ring's final pointer (keeps each loop live)
 *  47  REPS
 *  48  log2 of ITER
 *  49  0x50505451 ("PPTQ"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS     23
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

#define T        ".long 0x7c00222c\n"
#define DCBT_R10 ".long 0x7c00522c\n"
#define LD       "ld 4,0(4)\n"
#define NOP      "nop\n"
#define MUL      "mulld 12,12,15\n"
#define MUL2     MUL MUL
#define MUL4     MUL2 MUL2
#define MUL8     MUL4 MUL4
#define W40      MUL8 MUL8 MUL8 MUL8 MUL8
#define PROLOGUE                                                   \
    "rlwinm 10,4,0,19,24\n" "rlwinm 11,4,3,14,15\n"                \
    "add 10,10,11\n" "add 10,10,20\n"
#define SAME     DCBT_R10 "addi 10,10,8192\n"
#define SPACED   SAME MUL2
#define OTHER    DCBT_R10 "addi 10,10,8320\n"
#define SAME2    SAME SAME
#define SAME4    SAME2 SAME2

#define S0  PROLOGUE
#define S1  PROLOGUE SAME
#define S2  PROLOGUE SAME2
#define S3  PROLOGUE SAME2 SAME
#define S4  PROLOGUE SAME4
#define S5  PROLOGUE SAME4 SAME
#define S6  PROLOGUE SAME4 SAME2
#define S7  PROLOGUE SAME4 SAME2 SAME
#define S8  PROLOGUE SAME4 SAME4
#define SP4 PROLOGUE SPACED SPACED SPACED SPACED
#define SP7 SP4 SPACED SPACED SPACED
#define X4  "addi 10,10,8320\n" OTHER OTHER OTHER OTHER

#define CHASE(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        register u64 p __asm__("4") = mem_head;                    \
        register u64 c __asm__("20") = EA(conflict);               \
        register u64 m __asm__("12") = 1;                          \
        register u64 one __asm__("15") = 1;                        \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : "+r"(p), "+r"(m) : [n] "r"(n), "r"(c), "r"(one)      \
            : "r10", "r11", "ctr", "memory");                      \
        mem_head = p;                                              \
        return timebase() - t0;                                    \
    }

CHASE(loop_t_s0, T S0 NOP LD)
CHASE(loop_t_s1, T S1 NOP LD)
CHASE(loop_t_s2, T S2 NOP LD)
CHASE(loop_t_s3, T S3 NOP LD)
CHASE(loop_t_s4, T S4 NOP LD)
CHASE(loop_t_s5, T S5 NOP LD)
CHASE(loop_t_s6, T S6 NOP LD)
CHASE(loop_t_s7, T S7 NOP LD)
CHASE(loop_t_s8, T S8 NOP LD)
CHASE(loop_t_s3_t, T S3 T LD)
CHASE(loop_t_s4_t, T S4 T LD)
CHASE(loop_t_s5_t, T S5 T LD)
CHASE(loop_t_s6_t, T S6 T LD)
CHASE(loop_s3, S3 LD)
CHASE(loop_s5, S5 LD)
CHASE(loop_s6, S6 LD)
CHASE(loop_s7, S7 LD)
CHASE(loop_t_wait, T W40 LD)
CHASE(loop_t_wait_s3, T W40 S3 NOP LD)
CHASE(loop_t_wait_s7, T W40 S7 NOP LD)
CHASE(loop_t_s3_x4, T S3 X4 NOP LD)
CHASE(loop_t_sp4, T SP4 NOP LD)
CHASE(loop_t_sp7, T SP7 NOP LD)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    mem_head = build_ring(mem_ring, MEM_BYTES);

    TIMED(0, loop_t_s0(n));
    TIMED(1, loop_t_s1(n));
    TIMED(2, loop_t_s2(n));
    TIMED(3, loop_t_s3(n));
    TIMED(4, loop_t_s4(n));
    TIMED(5, loop_t_s5(n));
    TIMED(6, loop_t_s6(n));
    TIMED(7, loop_t_s7(n));
    TIMED(8, loop_t_s8(n));
    TIMED(9, loop_t_s3_t(n));
    TIMED(10, loop_t_s4_t(n));
    TIMED(11, loop_t_s5_t(n));
    TIMED(12, loop_t_s6_t(n));
    TIMED(13, loop_s3(n));
    TIMED(14, loop_s5(n));
    TIMED(15, loop_s6(n));
    TIMED(16, loop_s7(n));
    TIMED(17, loop_t_wait(n));
    TIMED(18, loop_t_wait_s3(n));
    TIMED(19, loop_t_wait_s7(n));
    TIMED(20, loop_t_s3_x4(n));
    TIMED(21, loop_t_sp4(n));
    TIMED(22, loop_t_sp7(n));

    put(2 * LOOPS, mem_head);
    put(2 * LOOPS + 1, REPS);
    put(2 * LOOPS + 2, ITER_LOG2);
    put(2 * LOOPS + 3, 0x50505451ull);

    write_frame();
    return 0;
}
