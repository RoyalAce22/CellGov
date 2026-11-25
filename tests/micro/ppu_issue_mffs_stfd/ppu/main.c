/* PPU probe: how far an stfd sits from an mffs sets the wait of the next
 * mffs. Each loop runs a fixed body ITER times between two time-base
 * reads. The probe runs each loop REPS times and records the smallest
 * and the largest delta. f14 starts zero. Each loop counts in CTR and
 * closes with bdnz.
 *
 * A loop "@0" starts on a 32-byte boundary. A loop "@4" starts 4 bytes
 * past one: a nop before the loop's label fills the 4 bytes and runs
 * once.
 *
 * "li" is a run of li cycling li r3,1 to li r10,1. "stfd" is
 * stfd f14,0(r11), with r11 the address of a 128-byte buffer.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mffs f0; stfd                @0
 *   1  mffs f0; stfd                @4
 *   2  mffs f0; 1 li; stfd          @0
 *   3  mffs f0; 1 li; stfd          @4
 *   4  mffs f0; 2 li; stfd          @0
 *   5  mffs f0; 2 li; stfd          @4
 *   6  mffs f0; 4 li; stfd          @0
 *   7  mffs f0; 4 li; stfd          @4
 *   8  mffs f0; 8 li; stfd          @0
 *   9  mffs f0; 8 li; stfd          @4
 *  10  mffs f0; 16 li; stfd         @0
 *  11  mffs f0; 16 li; stfd         @4
 *  12  stfd; mffs f0                @0
 *  13  stfd; mffs f0                @4
 *  14  1 li; stfd; mffs f0          @0
 *  15  1 li; stfd; mffs f0          @4
 *  16  4 li; stfd; mffs f0          @0
 *  17  4 li; stfd; mffs f0          @4
 *  18  mffs f0                      @0
 *  19  mffs f0                      @4
 *   word
 *  40  REPS
 *  41  log2 of ITER
 *  42  0x5050465a ("PPFZ"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS        20
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
#define STFD    "stfd 14,0(11)\n"
#define D1      "li 3,1\n"
#define D2      D1 "li 4,1\n"
#define D4      D2 "li 5,1\n" "li 6,1\n"
#define D8      D4 "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n"
#define D16     D8 D8

/* A loop whose label sits `pad` past a 32-byte boundary. r11 holds the
 * buffer's address. */
#define MLOOP(name, pad, body)                                         \
    static u64 __attribute__((noinline)) name(u64 n)                   \
    {                                                                  \
        u64 t0 = timebase();                                           \
        __asm__ volatile (                                             \
            "mr 11,%[p]\n"                                             \
            "mtctr %[n]\n"                                             \
            ".p2align 5\n" pad "1:\n" body "bdnz 1b\n"                 \
            : : [n] "r"(n), [p] "r"(buf)                               \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11",  \
              "ctr", "fr0", "memory");                                 \
        return timebase() - t0;                                        \
    }

#define AT0     ""
#define AT4     "nop\n"

MLOOP(loop_m_s_at0, AT0, MFFS STFD)
MLOOP(loop_m_s_at4, AT4, MFFS STFD)
MLOOP(loop_m_1_s_at0, AT0, MFFS D1 STFD)
MLOOP(loop_m_1_s_at4, AT4, MFFS D1 STFD)
MLOOP(loop_m_2_s_at0, AT0, MFFS D2 STFD)
MLOOP(loop_m_2_s_at4, AT4, MFFS D2 STFD)
MLOOP(loop_m_4_s_at0, AT0, MFFS D4 STFD)
MLOOP(loop_m_4_s_at4, AT4, MFFS D4 STFD)
MLOOP(loop_m_8_s_at0, AT0, MFFS D8 STFD)
MLOOP(loop_m_8_s_at4, AT4, MFFS D8 STFD)
MLOOP(loop_m_16_s_at0, AT0, MFFS D16 STFD)
MLOOP(loop_m_16_s_at4, AT4, MFFS D16 STFD)
MLOOP(loop_s_m_at0, AT0, STFD MFFS)
MLOOP(loop_s_m_at4, AT4, STFD MFFS)
MLOOP(loop_1_s_m_at0, AT0, D1 STFD MFFS)
MLOOP(loop_1_s_m_at4, AT4, D1 STFD MFFS)
MLOOP(loop_4_s_m_at0, AT0, D4 STFD MFFS)
MLOOP(loop_4_s_m_at4, AT4, D4 STFD MFFS)
MLOOP(loop_m_at0, AT0, MFFS)
MLOOP(loop_m_at4, AT4, MFFS)

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

    TIMED(0, loop_m_s_at0(n));
    TIMED(1, loop_m_s_at4(n));
    TIMED(2, loop_m_1_s_at0(n));
    TIMED(3, loop_m_1_s_at4(n));
    TIMED(4, loop_m_2_s_at0(n));
    TIMED(5, loop_m_2_s_at4(n));
    TIMED(6, loop_m_4_s_at0(n));
    TIMED(7, loop_m_4_s_at4(n));
    TIMED(8, loop_m_8_s_at0(n));
    TIMED(9, loop_m_8_s_at4(n));
    TIMED(10, loop_m_16_s_at0(n));
    TIMED(11, loop_m_16_s_at4(n));
    TIMED(12, loop_s_m_at0(n));
    TIMED(13, loop_s_m_at4(n));
    TIMED(14, loop_1_s_m_at0(n));
    TIMED(15, loop_1_s_m_at4(n));
    TIMED(16, loop_4_s_m_at0(n));
    TIMED(17, loop_4_s_m_at4(n));
    TIMED(18, loop_m_at0(n));
    TIMED(19, loop_m_at4(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x5050465aull);

    write_frame();
    return 0;
}
