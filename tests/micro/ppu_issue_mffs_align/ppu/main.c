/* PPU probe: how the 8-byte slot of an mffs and an fadd sets the wait
 * between them. Each loop runs a fixed body ITER times between two
 * time-base reads. The probe runs each loop REPS times and records the
 * smallest and the largest delta. f14 starts zero. Each loop counts in
 * CTR and closes with bdnz.
 *
 * A loop "@0" starts on a 32-byte boundary. A loop "@4" starts 4 bytes
 * past one: a nop before the loop's label fills the 4 bytes and runs
 * once. So each instruction of an @4 loop sits in the other half of its
 * 8-byte pair from the same instruction in an @0 loop.
 *
 * "li" is a run of li cycling li r3,1 to li r10,1. "fadd" is
 * fadd f1,f14,f14. "b" is a taken branch to the next instruction.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mffs f0; fadd                @4
 *   1  mffs f0; 1 li; fadd          @4
 *   2  mffs f0; 2 li; fadd          @4
 *   3  mffs f0; 4 li; fadd          @4
 *   4  mffs f0; 8 li; fadd          @4
 *   5  mffs f0; 16 li; fadd         @4
 *   6  fadd; mffs f0                @4
 *   7  1 li; fadd; mffs f0          @4
 *   8  2 li; fadd; mffs f0          @4
 *   9  4 li; fadd; mffs f0          @4
 *  10  8 li; fadd; mffs f0          @4
 *  11  16 li; fadd; mffs f0         @4
 *  12  fadd; 1 li; mffs f0          @4
 *  13  fadd; 4 li; mffs f0          @4
 *  14  fadd; 16 li; mffs f0         @4
 *  15  mffs f0; b; fadd             @4
 *  16  mffs f0                      @0
 *  17  mffs f0                      @4
 *  18  mffs f0; 16 li               @0
 *  19  mffs f0; 16 li               @4
 *  20  mffs f0; 4 li                @0
 *  21  mffs f0; 4 li                @4
 *  22  mffs f0; 1 li                @0
 *  23  mffs f0; 1 li                @4
 *  24  mffs f0; 12 li; fadd         @0
 *  25  mffs f0; 12 li; fadd         @4
 *  26  12 li; fadd; mffs f0         @0
 *  27  12 li; fadd; mffs f0         @4
 *  28  mffs f0; 6 li; fadd          @0
 *  29  mffs f0; 6 li; fadd          @4
 *  30  6 li; fadd; mffs f0          @0
 *  31  6 li; fadd; mffs f0          @4
 *   word
 *  64  REPS
 *  65  log2 of ITER
 *  66  0x5050465a ("PPFZ"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS        32
#define WORDS        (2 * LOOPS + 3)
#define PAYLOAD      (WORDS * 8)
#define REPS         5
#define ITER_LOG2    16

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

#define MFFS    "mffs 0\n"
#define FADD    "fadd 1,14,14\n"
#define B       "b 2f\n" "2:\n"
#define D1      "li 3,1\n"
#define D2      D1 "li 4,1\n"
#define D4      D2 "li 5,1\n" "li 6,1\n"
#define D6      D4 "li 7,1\n" "li 8,1\n"
#define D8      D4 "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n"
#define D12     D8 D4
#define D16     D8 D8

/* A loop whose label sits `pad` past a 32-byte boundary. */
#define MLOOP(name, pad, body)                                         \
    static u64 __attribute__((noinline)) name(u64 n)                   \
    {                                                                  \
        u64 t0 = timebase();                                           \
        __asm__ volatile (                                             \
            "mtctr %[n]\n"                                             \
            ".p2align 5\n" pad "1:\n" body "bdnz 1b\n"                 \
            : : [n] "r"(n)                                             \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",         \
              "ctr", "fr0", "fr1");                                    \
        return timebase() - t0;                                        \
    }

#define AT0     ""
#define AT4     "nop\n"

MLOOP(loop_m_f, AT4, MFFS FADD)
MLOOP(loop_m_1_f, AT4, MFFS D1 FADD)
MLOOP(loop_m_2_f, AT4, MFFS D2 FADD)
MLOOP(loop_m_4_f, AT4, MFFS D4 FADD)
MLOOP(loop_m_8_f, AT4, MFFS D8 FADD)
MLOOP(loop_m_16_f, AT4, MFFS D16 FADD)
MLOOP(loop_f_m, AT4, FADD MFFS)
MLOOP(loop_1_f_m, AT4, D1 FADD MFFS)
MLOOP(loop_2_f_m, AT4, D2 FADD MFFS)
MLOOP(loop_4_f_m, AT4, D4 FADD MFFS)
MLOOP(loop_8_f_m, AT4, D8 FADD MFFS)
MLOOP(loop_16_f_m, AT4, D16 FADD MFFS)
MLOOP(loop_f_1_m, AT4, FADD D1 MFFS)
MLOOP(loop_f_4_m, AT4, FADD D4 MFFS)
MLOOP(loop_f_16_m, AT4, FADD D16 MFFS)
MLOOP(loop_m_b_f, AT4, MFFS B FADD)
MLOOP(loop_m_at0, AT0, MFFS)
MLOOP(loop_m_at4, AT4, MFFS)
MLOOP(loop_m_16_at0, AT0, MFFS D16)
MLOOP(loop_m_16_at4, AT4, MFFS D16)
MLOOP(loop_m_4_at0, AT0, MFFS D4)
MLOOP(loop_m_4_at4, AT4, MFFS D4)
MLOOP(loop_m_1_at0, AT0, MFFS D1)
MLOOP(loop_m_1_at4, AT4, MFFS D1)
MLOOP(loop_m_12_f_at0, AT0, MFFS D12 FADD)
MLOOP(loop_m_12_f_at4, AT4, MFFS D12 FADD)
MLOOP(loop_12_f_m_at0, AT0, D12 FADD MFFS)
MLOOP(loop_12_f_m_at4, AT4, D12 FADD MFFS)
MLOOP(loop_m_6_f_at0, AT0, MFFS D6 FADD)
MLOOP(loop_m_6_f_at4, AT4, MFFS D6 FADD)
MLOOP(loop_6_f_m_at0, AT0, D6 FADD MFFS)
MLOOP(loop_6_f_m_at4, AT4, D6 FADD MFFS)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* f14 starts zero. */
    __asm__ volatile ("lfd 14,0(%[p])\n" : : [p] "b"(buf) : "fr14", "memory");

    TIMED(0, loop_m_f(n));
    TIMED(1, loop_m_1_f(n));
    TIMED(2, loop_m_2_f(n));
    TIMED(3, loop_m_4_f(n));
    TIMED(4, loop_m_8_f(n));
    TIMED(5, loop_m_16_f(n));
    TIMED(6, loop_f_m(n));
    TIMED(7, loop_1_f_m(n));
    TIMED(8, loop_2_f_m(n));
    TIMED(9, loop_4_f_m(n));
    TIMED(10, loop_8_f_m(n));
    TIMED(11, loop_16_f_m(n));
    TIMED(12, loop_f_1_m(n));
    TIMED(13, loop_f_4_m(n));
    TIMED(14, loop_f_16_m(n));
    TIMED(15, loop_m_b_f(n));
    TIMED(16, loop_m_at0(n));
    TIMED(17, loop_m_at4(n));
    TIMED(18, loop_m_16_at0(n));
    TIMED(19, loop_m_16_at4(n));
    TIMED(20, loop_m_4_at0(n));
    TIMED(21, loop_m_4_at4(n));
    TIMED(22, loop_m_1_at0(n));
    TIMED(23, loop_m_1_at4(n));
    TIMED(24, loop_m_12_f_at0(n));
    TIMED(25, loop_m_12_f_at4(n));
    TIMED(26, loop_12_f_m_at0(n));
    TIMED(27, loop_12_f_m_at4(n));
    TIMED(28, loop_m_6_f_at0(n));
    TIMED(29, loop_m_6_f_at4(n));
    TIMED(30, loop_6_f_m_at0(n));
    TIMED(31, loop_6_f_m_at4(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x5050465aull);

    write_frame();
    return 0;
}
