/* PPU probe: when a touch reload evicts its victim from the L1, and which
 * line it evicts. Each loop runs a fixed body ITER times between two
 * time-base reads, REPS times, and records the smallest and the largest
 * delta. The loop branch is bdnz.
 *
 * The L1 picks a line's set from address bits 7 to 12 and the L2 from
 * bits 7 to 15. A user page is 64 KB or 1 MB, so the effective and the
 * real address agree in those bits. Every buffer is 64 KB aligned.
 *
 *   B0..B3  four lines of a 32 KB conflict buffer, 8 KB apart, in L1 set
 *           37 (odd). Each holds zero, so `ld 11,k(12); add 12,10,11`
 *           loads one and keeps r12 = B0 (r10) dependent on the load.
 *           They stay in the L2: no other line here shares their L2 sets.
 *   B4      a line of an 8 MB buffer in the same L1 set, 64 KB apart from
 *           run to run, in an L2 set the B lines do not use. Each touch
 *           of it reaches memory.
 *   O       lines of a 4 MB buffer 256 bytes apart: even L1 and L2 sets,
 *           which the B lines and B4 do not use. Each touch reaches
 *           memory.
 *
 *   CHAIN = 4 x (ld 11,8192k(12); add 12,10,11)       B0, B1, B2, B3
 *   T4    = dcbt r24,r25; addis 25,25,1; rlwinm 25,25,0,9,31
 *   O(n)  = n x (dcbt r28,r27; addi 27,27,256); rlwinm 27,27,0,10,31
 *   LD0   = ld 11,0(12); add 12,10,11                 B0
 *   W(k)  = k x mulld 12,12,15                        (r15 = 1)
 *
 * After CHAIN, B0 is the set's next victim. T4's reload will evict it.
 *
 *   loop
 *   0  CHAIN T4 O(0)  LD0 W(41)
 *   1  CHAIN T4 O(6)  LD0 W(71)
 *   2  CHAIN T4 O(7)  LD0 W(76)
 *   3  CHAIN T4 O(8)  LD0 W(81)
 *   4  CHAIN T4 O(12) LD0 W(101)
 *   5  CHAIN T4 O(16) LD0 W(121)
 *   6  CHAIN T4 O(32) LD0 W(201)
 *   7  CHAIN T4 nop nop W(56) LD0
 *   8  CHAIN T4 LD0     W(56) LD0
 *   9  CHAIN nop x 5    W(56) LD0
 *
 * Loops 0 to 6 load B0 with T4's reload and n other touch reloads on
 * their way. W(5n + 41) lets the n + 1 touches go, about 70 cycles each,
 * before the next run. Loop 7 loads B0 after T4's reload returns. Loop 8
 * hits B0 before T4's reload returns, which moves the replacement tree
 * off B0, and loads B0 again after it returns. Loop 9 has no touch.
 *
 * The buffers' offsets carry over from run to run.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   word
 *  20  the low 16 bits of the three buffers' addresses, ORed (0)
 *  21  REPS
 *  22  log2 of ITER
 *  23  0x50505252 ("PPRR"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     24
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 12

#define LINE      128
#define SET_OFF   (37u * LINE)
#define B4_OFF    (32u * 1024u + SET_OFF)
#define CONFLICT_BYTES (32u * 1024u)
#define B4_BYTES  (8u * 1024u * 1024u + 64u * 1024u)
#define O_BYTES   (4u * 1024u * 1024u + 64u * 1024u)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static unsigned char conflict[CONFLICT_BYTES] __attribute__((aligned(65536)));
static unsigned char b4_buf[B4_BYTES] __attribute__((aligned(65536)));
static unsigned char o_buf[O_BYTES] __attribute__((aligned(65536)));
static u64 b4_off;
static u64 o_off;

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

#define CHAIN                                                      \
    "ld 11,0(12)\n"     "add 12,10,11\n"                           \
    "ld 11,8192(12)\n"  "add 12,10,11\n"                           \
    "ld 11,16384(12)\n" "add 12,10,11\n"                           \
    "ld 11,24576(12)\n" "add 12,10,11\n"
/* dcbt r24,r25 */
#define T4 ".long 0x7c18ca2c\n" "addis 25,25,1\n" "rlwinm 25,25,0,9,31\n"
/* n x dcbt r28,r27 */
#define O(n)                                                       \
    ".rept " #n "\n" ".long 0x7c1cda2c\n" "addi 27,27,256\n" ".endr\n" \
    "rlwinm 27,27,0,10,31\n"
#define LD0 "ld 11,0(12)\n" "add 12,10,11\n"
#define W(k) ".rept " #k "\n" "mulld 12,12,15\n" ".endr\n"
#define NOP "nop\n"

#define PROBE(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        register u64 b0 __asm__("10") = EA(conflict) + SET_OFF;    \
        register u64 p __asm__("12") = EA(conflict) + SET_OFF;     \
        register u64 one __asm__("15") = 1;                        \
        register u64 b4 __asm__("24") = EA(b4_buf) + B4_OFF;       \
        register u64 b4o __asm__("25") = b4_off;                   \
        register u64 oo __asm__("27") = o_off;                     \
        register u64 ob __asm__("28") = EA(o_buf);                 \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : "+r"(p), "+r"(b4o), "+r"(oo)                         \
            : [n] "r"(n), "r"(b0), "r"(one), "r"(b4), "r"(ob)      \
            : "r11", "ctr", "memory");                             \
        b4_off = b4o;                                              \
        o_off = oo;                                                \
        return timebase() - t0;                                    \
    }

PROBE(loop_o0,  CHAIN T4 O(0)  LD0 W(41))
PROBE(loop_o6,  CHAIN T4 O(6)  LD0 W(71))
PROBE(loop_o7,  CHAIN T4 O(7)  LD0 W(76))
PROBE(loop_o8,  CHAIN T4 O(8)  LD0 W(81))
PROBE(loop_o12, CHAIN T4 O(12) LD0 W(101))
PROBE(loop_o16, CHAIN T4 O(16) LD0 W(121))
PROBE(loop_o32, CHAIN T4 O(32) LD0 W(201))
PROBE(loop_after, CHAIN T4 NOP NOP W(56) LD0)
PROBE(loop_hit, CHAIN T4 LD0 W(56) LD0)
PROBE(loop_none, CHAIN NOP NOP NOP NOP NOP W(56) LD0)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    TIMED(0, loop_o0(n));
    TIMED(1, loop_o6(n));
    TIMED(2, loop_o7(n));
    TIMED(3, loop_o8(n));
    TIMED(4, loop_o12(n));
    TIMED(5, loop_o16(n));
    TIMED(6, loop_o32(n));
    TIMED(7, loop_after(n));
    TIMED(8, loop_hit(n));
    TIMED(9, loop_none(n));

    put(20, (EA(conflict) | EA(b4_buf) | EA(o_buf)) & 0xFFFFull);
    put(21, REPS);
    put(22, ITER_LOG2);
    put(23, 0x50505252ull);

    write_frame();
    return 0;
}
