/* PPU probe: how long a drain after a store waits for it, by the store's
 * form and by the distance between them. The drains are mfvscr and
 * mffs; each stalls at issue until every older instruction completes.
 * Each loop runs a fixed body ITER times between two time-base reads.
 * The probe runs each loop REPS times and records the smallest and the
 * largest delta. Every store addresses one 128-byte-aligned buffer the
 * probe zeroes first; r29 holds its address. f14, r14 and v1 start zero.
 * Each loop counts in CTR and closes with bdnz.
 *
 * "li" is a run of li cycling li r3,1 to li r10,1.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mfvscr v3; 16 li; std r14,0(r29)
 *   1  mfvscr v3; 16 li; stw r14,0(r29)
 *   2  mfvscr v3; 16 li; stb r14,0(r29)
 *   3  mfvscr v3; 16 li; stfd f14,0(r29)
 *   4  mfvscr v3; 16 li; stfs f14,0(r29)
 *   5  mfvscr v3; 16 li; stfiwx f14,0,r29
 *   6  mfvscr v3; 16 li; stvx v1,0,r29
 *   7  mfvscr v3; 16 li; std r14,0(r29); std r14,8(r29)
 *   8  mfvscr v3; 16 li; std r14,0(r29); 4 li
 *   9  mfvscr v3; 16 li; std r14,0(r29); 8 li
 *  10  mfvscr v3; 16 li; std r14,0(r29); 16 li
 *  11  mfvscr v3; 16 li; std r14,0(r29); 32 li
 *  12  mfvscr v3; 16 li; stfd f14,0(r29); 8 li
 *  13  mfvscr v3; 16 li; stfd f14,0(r29); 16 li
 *  14  mfvscr v3; 16 li; stfd f14,0(r29); 32 li
 *  15  mfvscr v3; 16 li; stfd f14,0(r29); 48 li
 *  16  mffs f0
 *  17  16 li; mffs f0
 *  18  16 li; std r14,0(r29); mffs f0
 *  19  16 li; stfd f14,0(r29); mffs f0
 *  20  32 li; mffs f0
 *  21  32 li; std r14,0(r29); mffs f0
 *  22  32 li; stfd f14,0(r29); mffs f0
 *  23  std r14,0(r29); 8 li; mffs f0
 *  24  mfvscr v3; 16 li
 *  25  mfvscr v3; 32 li
 *   word
 *  52  REPS
 *  53  log2 of ITER
 *  54  0x50505344 ("PPSD"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS        26
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

#define STD     "std 14,0(29)\n"
#define STD8    "std 14,8(29)\n"
#define STW     "stw 14,0(29)\n"
#define STB     "stb 14,0(29)\n"
#define STFD    "stfd 14,0(29)\n"
#define STFS    "stfs 14,0(29)\n"
#define STFIWX  "stfiwx 14,0,29\n"
#define STVX    "stvx 1,0,29\n"
#define MFVSCR  "mfvscr 3\n"
#define MFFS    "mffs 0\n"
#define D4      "li 3,1\n" "li 4,1\n" "li 5,1\n" "li 6,1\n"
#define D8      D4 "li 7,1\n" "li 8,1\n" "li 9,1\n" "li 10,1\n"
#define D16     D8 D8
#define D32     D16 D16
#define D48     D32 D16

#define MLOOP(name, body)                                              \
    static u64 __attribute__((noinline)) name(u64 n)                   \
    {                                                                  \
        u64 t0 = timebase();                                           \
        __asm__ volatile (                                             \
            "mr 29,%[p]\n" "mtctr %[n]\n"                              \
            ".p2align 5\n" "1:\n" body "bdnz 1b\n"                     \
            : : [n] "r"(n), [p] "r"(buf)                               \
            : "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",         \
              "r29", "ctr", "memory", "fr0", "v3");                    \
        return timebase() - t0;                                        \
    }

MLOOP(loop_v16_std, MFVSCR D16 STD)
MLOOP(loop_v16_stw, MFVSCR D16 STW)
MLOOP(loop_v16_stb, MFVSCR D16 STB)
MLOOP(loop_v16_stfd, MFVSCR D16 STFD)
MLOOP(loop_v16_stfs, MFVSCR D16 STFS)
MLOOP(loop_v16_stfiwx, MFVSCR D16 STFIWX)
MLOOP(loop_v16_stvx, MFVSCR D16 STVX)
MLOOP(loop_v16_std_std, MFVSCR D16 STD STD8)
MLOOP(loop_v16_std_4, MFVSCR D16 STD D4)
MLOOP(loop_v16_std_8, MFVSCR D16 STD D8)
MLOOP(loop_v16_std_16, MFVSCR D16 STD D16)
MLOOP(loop_v16_std_32, MFVSCR D16 STD D32)
MLOOP(loop_v16_stfd_8, MFVSCR D16 STFD D8)
MLOOP(loop_v16_stfd_16, MFVSCR D16 STFD D16)
MLOOP(loop_v16_stfd_32, MFVSCR D16 STFD D32)
MLOOP(loop_v16_stfd_48, MFVSCR D16 STFD D48)
MLOOP(loop_mffs, MFFS)
MLOOP(loop_16_mffs, D16 MFFS)
MLOOP(loop_16_std_mffs, D16 STD MFFS)
MLOOP(loop_16_stfd_mffs, D16 STFD MFFS)
MLOOP(loop_32_mffs, D32 MFFS)
MLOOP(loop_32_std_mffs, D32 STD MFFS)
MLOOP(loop_32_stfd_mffs, D32 STFD MFFS)
MLOOP(loop_std_8_mffs, STD D8 MFFS)
MLOOP(loop_v16, MFVSCR D16)
MLOOP(loop_v32, MFVSCR D32)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* f14, r14 and v1 start zero. */
    __asm__ volatile (
        "lfd 14,0(%[p])\n" "li 14,0\n" "lvx 1,0,%[p]\n"
        : : [p] "b"(buf) : "fr14", "r14", "v1", "memory");

    TIMED(0, loop_v16_std(n));
    TIMED(1, loop_v16_stw(n));
    TIMED(2, loop_v16_stb(n));
    TIMED(3, loop_v16_stfd(n));
    TIMED(4, loop_v16_stfs(n));
    TIMED(5, loop_v16_stfiwx(n));
    TIMED(6, loop_v16_stvx(n));
    TIMED(7, loop_v16_std_std(n));
    TIMED(8, loop_v16_std_4(n));
    TIMED(9, loop_v16_std_8(n));
    TIMED(10, loop_v16_std_16(n));
    TIMED(11, loop_v16_std_32(n));
    TIMED(12, loop_v16_stfd_8(n));
    TIMED(13, loop_v16_stfd_16(n));
    TIMED(14, loop_v16_stfd_32(n));
    TIMED(15, loop_v16_stfd_48(n));
    TIMED(16, loop_mffs(n));
    TIMED(17, loop_16_mffs(n));
    TIMED(18, loop_16_std_mffs(n));
    TIMED(19, loop_16_stfd_mffs(n));
    TIMED(20, loop_32_mffs(n));
    TIMED(21, loop_32_std_mffs(n));
    TIMED(22, loop_32_stfd_mffs(n));
    TIMED(23, loop_std_8_mffs(n));
    TIMED(24, loop_v16(n));
    TIMED(25, loop_v32(n));

    put(2 * LOOPS, REPS);
    put(2 * LOOPS + 1, ITER_LOG2);
    put(2 * LOOPS + 2, 0x50505344ull);

    write_frame();
    return 0;
}
