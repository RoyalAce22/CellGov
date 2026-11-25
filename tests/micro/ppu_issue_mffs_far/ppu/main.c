/* PPU probe: an fadd or an stfd 20 to 48 li from an mffs, before or
 * after it, and the li alone. Each loop runs a fixed body ITER times
 * between two time-base reads. The probe runs each loop REPS times and
 * records the smallest and the largest delta. f14 starts zero. Each loop
 * starts on a 32-byte boundary, counts in CTR and closes with bdnz.
 *
 * "li" is a run of li cycling li r3,1 to li r10,1. "fadd" is
 * fadd f1,f14,f14. "stfd" is stfd f14,0(r11), with r11 the address of a
 * 128-byte buffer.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  24 li; fadd; mffs f0
 *   1  24 li; stfd; mffs f0
 *   2  32 li; fadd; mffs f0
 *   3  32 li; stfd; mffs f0
 *   4  mffs f0; 24 li; fadd
 *   5  mffs f0; 24 li; stfd
 *   6  mffs f0; 32 li; fadd
 *   7  mffs f0; 32 li; stfd
 *   8  48 li; fadd; mffs f0
 *   9  48 li; stfd; mffs f0
 *  10  mffs f0; 48 li; fadd
 *  11  mffs f0; 48 li; stfd
 *  12  mffs f0; 20 li; fadd
 *  13  mffs f0; 20 li; stfd
 *  14  mffs f0; 32 li
 *  15  48 li
 *   word
 *  32  REPS
 *  33  log2 of ITER
 *  34  0x5050465a ("PPFZ"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS        16
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
#define STFD    "stfd 14,0(11)\n"
#define D4      "li 3,1\n" "li 4,1\n" "li 5,1\n" "li 6,1\n"
#define D8      D4 "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n"
#define D16     D8 D8
#define D20     D16 D4
#define D24     D16 D8
#define D32     D16 D16
#define D48     D32 D16

/* A loop on a 32-byte boundary. r11 holds the buffer's address. */
#define MLOOP(name, body)                                              \
    static u64 __attribute__((noinline)) name(u64 n)                   \
    {                                                                  \
        u64 t0 = timebase();                                           \
        __asm__ volatile (                                             \
            "mr 11,%[p]\n"                                             \
            "mtctr %[n]\n"                                             \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                     \
            : : [n] "r"(n), [p] "r"(buf)                               \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11",  \
              "ctr", "fr0", "fr1", "memory");                          \
        return timebase() - t0;                                        \
    }

MLOOP(loop_24_f_m, D24 FADD MFFS)
MLOOP(loop_24_s_m, D24 STFD MFFS)
MLOOP(loop_32_f_m, D32 FADD MFFS)
MLOOP(loop_32_s_m, D32 STFD MFFS)
MLOOP(loop_m_24_f, MFFS D24 FADD)
MLOOP(loop_m_24_s, MFFS D24 STFD)
MLOOP(loop_m_32_f, MFFS D32 FADD)
MLOOP(loop_m_32_s, MFFS D32 STFD)
MLOOP(loop_48_f_m, D48 FADD MFFS)
MLOOP(loop_48_s_m, D48 STFD MFFS)
MLOOP(loop_m_48_f, MFFS D48 FADD)
MLOOP(loop_m_48_s, MFFS D48 STFD)
MLOOP(loop_m_20_f, MFFS D20 FADD)
MLOOP(loop_m_20_s, MFFS D20 STFD)
MLOOP(loop_m_32, MFFS D32)
MLOOP(loop_48, D48)

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

    TIMED(0, loop_24_f_m(n));
    TIMED(1, loop_24_s_m(n));
    TIMED(2, loop_32_f_m(n));
    TIMED(3, loop_32_s_m(n));
    TIMED(4, loop_m_24_f(n));
    TIMED(5, loop_m_24_s(n));
    TIMED(6, loop_m_32_f(n));
    TIMED(7, loop_m_32_s(n));
    TIMED(8, loop_48_f_m(n));
    TIMED(9, loop_48_s_m(n));
    TIMED(10, loop_m_48_f(n));
    TIMED(11, loop_m_48_s(n));
    TIMED(12, loop_m_20_f(n));
    TIMED(13, loop_m_20_s(n));
    TIMED(14, loop_m_32(n));
    TIMED(15, loop_48(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x5050465aull);

    write_frame();
    return 0;
}
