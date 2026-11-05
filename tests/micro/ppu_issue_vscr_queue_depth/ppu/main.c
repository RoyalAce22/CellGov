/* PPU probe: how many vector instructions an mtvscr fence holds before
 * IS2 stalls the fixed-point work after them, and whether a vector or FP
 * load the fence holds takes the load-hit-store recycle of a store
 * before it. Each loop runs a fixed body ITER times between two
 * time-base reads. The probe runs each loop REPS times and records the
 * smallest and the largest delta. The loop branch is bdnz. Every vector
 * register starts zero, so mtvscr v0 writes a zero VSCR. Each load and
 * store addresses one 128-byte-aligned buffer the probe zeroes first.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mtvscr v0; 8 independent vaddubm; 16 independent addi
 *   1  mtvscr v0; 10 independent vaddubm; 16 independent addi
 *   2  mtvscr v0; 12 independent vaddubm; 16 independent addi
 *   3  mtvscr v0; 16 independent vaddubm; 16 independent addi
 *   4  mtvscr v0; 24 independent vaddubm; 16 independent addi
 *   5  mtvscr v0; 24 independent vaddubm
 *   6  mtvscr v0; stw r14,0(p); lfd f1,0(p)
 *   7  stw r14,0(p); lfd f1,0(p)
 *   8  mtvscr v0; lfd f1,0(p)
 *   9  mtvscr v0; lvx v1,0,p; 4 independent addi; vaddubm v2,v1,v1
 *   word
 *  20  REPS
 *  21  log2 of ITER
 *  22  0x50505644 ("PPVD"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     23
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 16

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static unsigned char buf[128] __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

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

#define MTVSCR "mtvscr 0\n"
#define V(k) "vaddubm " #k ",20,21\n"
#define VADD8  V(1) V(2) V(3) V(4) V(5) V(6) V(7) V(8)
#define VADD10 VADD8 V(9) V(10)
#define VADD12 VADD10 V(11) V(12)
#define VADD16 VADD12 V(13) V(14) V(15) V(16)
#define VADD24 VADD16 V(17) V(18) V(19) V(22) V(23) V(24) V(25) V(26)
#define ADDI4                                                   \
    "addi 15,15,1\n" "addi 16,16,1\n"                           \
    "addi 17,17,1\n" "addi 18,18,1\n"
#define ADDI16 ADDI4 ADDI4 ADDI4 ADDI4

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : : [n] "r"(n), [p] "b"(buf)                           \
            : "r14", "r15", "r16", "r17", "r18", "ctr", "memory",  \
              "v0", "v1", "v2", "v3", "v4", "v5", "v6", "v7",      \
              "v8", "v9", "v10", "v11", "v12", "v13", "v14",       \
              "v15", "v16", "v17", "v18", "v19", "v20", "v21",     \
              "v22", "v23", "v24", "v25", "v26", "fr1");           \
        return timebase() - t0;                                    \
    }

#define STW "stw 14,0(%[p])\n"
#define LFD "lfd 1,0(%[p])\n"

MLOOP(loop_v8, MTVSCR VADD8 ADDI16)
MLOOP(loop_v10, MTVSCR VADD10 ADDI16)
MLOOP(loop_v12, MTVSCR VADD12 ADDI16)
MLOOP(loop_v16, MTVSCR VADD16 ADDI16)
MLOOP(loop_v24, MTVSCR VADD24 ADDI16)
MLOOP(loop_v24_alone, MTVSCR VADD24)
MLOOP(loop_fenced_lhs, MTVSCR STW LFD)
MLOOP(loop_lhs, STW LFD)
MLOOP(loop_fenced_lfd, MTVSCR LFD)
MLOOP(loop_lvx_use, MTVSCR "lvx 1,0,%[p]\n" ADDI4 "vaddubm 2,1,1\n")

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* Every vector register the loops read starts zero, and so does r14. */
    __asm__ volatile (
        "vxor 0,0,0\n" "vxor 20,20,20\n" "vxor 21,21,21\n" "li 14,0\n"
        : : : "v0", "v20", "v21", "r14");

    TIMED(0, loop_v8(n));
    TIMED(1, loop_v10(n));
    TIMED(2, loop_v12(n));
    TIMED(3, loop_v16(n));
    TIMED(4, loop_v24(n));
    TIMED(5, loop_v24_alone(n));
    TIMED(6, loop_fenced_lhs(n));
    TIMED(7, loop_lhs(n));
    TIMED(8, loop_fenced_lfd(n));
    TIMED(9, loop_lvx_use(n));

    put(20, REPS);
    put(21, ITER_LOG2);
    put(22, 0x50505644ull);

    write_frame();
    return 0;
}
