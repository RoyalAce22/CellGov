/* PPU probe: whether an mtvscr stalls the VMX and FP load and store forms
 * ppu_issue_vscr_mem does not time. Each loop runs a fixed body ITER
 * times between two time-base reads. The probe runs each loop REPS times
 * and records the smallest and the largest delta. The loop branch is
 * bdnz. Every vector register starts zero, so mtvscr v0 writes a zero
 * VSCR. Each load and store addresses one 128-byte-aligned buffer the
 * probe zeroes first; lfdu with a zero displacement leaves its base
 * unchanged.
 *
 * Each even loop times one instruction alone, and the odd loop after it
 * the same instruction after an mtvscr.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  lvsl v1,0,p
 *   1  mtvscr v0; lvsl v1,0,p
 *   2  lvebx v1,0,p
 *   3  mtvscr v0; lvebx v1,0,p
 *   4  lvlx v1,0,p
 *   5  mtvscr v0; lvlx v1,0,p
 *   6  stvebx v20,0,p
 *   7  mtvscr v0; stvebx v20,0,p
 *   8  lfs f1,0(p)
 *   9  mtvscr v0; lfs f1,0(p)
 *  10  stfs f2,0(p)
 *  11  mtvscr v0; stfs f2,0(p)
 *  12  lfdu f1,0(p)
 *  13  mtvscr v0; lfdu f1,0(p)
 *  14  stfiwx f2,0,p
 *  15  mtvscr v0; stfiwx f2,0,p
 *   word
 *  32  REPS
 *  33  log2 of ITER
 *  34  0x5050564d ("PPVM"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     35
#define PAYLOAD   (WORDS * 8)
#define REPS      5
#define ITER_LOG2 18

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
#define LVSL   "lvsl 1,0,%[p]\n"
#define LVEBX  "lvebx 1,0,%[p]\n"
#define LVLX   "lvlx 1,0,%[p]\n"
#define STVEBX "stvebx 20,0,%[p]\n"
#define LFS    "lfs 1,0(%[p])\n"
#define STFS   "stfs 2,0(%[p])\n"
#define LFDU   "lfdu 1,0(%[p])\n"
#define STFIWX "stfiwx 2,0,%[p]\n"

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : : [n] "r"(n), [p] "b"(buf)                           \
            : "ctr", "memory", "v0", "v1", "v20", "fr1", "fr2");   \
        return timebase() - t0;                                    \
    }

MLOOP(loop_lvsl, LVSL)
MLOOP(loop_mtvscr_lvsl, MTVSCR LVSL)
MLOOP(loop_lvebx, LVEBX)
MLOOP(loop_mtvscr_lvebx, MTVSCR LVEBX)
MLOOP(loop_lvlx, LVLX)
MLOOP(loop_mtvscr_lvlx, MTVSCR LVLX)
MLOOP(loop_stvebx, STVEBX)
MLOOP(loop_mtvscr_stvebx, MTVSCR STVEBX)
MLOOP(loop_lfs, LFS)
MLOOP(loop_mtvscr_lfs, MTVSCR LFS)
MLOOP(loop_stfs, STFS)
MLOOP(loop_mtvscr_stfs, MTVSCR STFS)
MLOOP(loop_lfdu, LFDU)
MLOOP(loop_mtvscr_lfdu, MTVSCR LFDU)
MLOOP(loop_stfiwx, STFIWX)
MLOOP(loop_mtvscr_stfiwx, MTVSCR STFIWX)

int main(void)
{
    unsigned i;
    const u64 n = 1ull << ITER_LOG2;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < sizeof buf; i++)
        buf[i] = 0;
    /* Every vector register the loops read starts zero, and so does f2. */
    __asm__ volatile (
        "vxor 0,0,0\n" "vxor 1,1,1\n" "vxor 20,20,20\n"
        "lfd 2,0(%[p])\n"
        : : [p] "b"(buf) : "v0", "v1", "v20", "fr2", "memory");

    TIMED(0, loop_lvsl(n));
    TIMED(1, loop_mtvscr_lvsl(n));
    TIMED(2, loop_lvebx(n));
    TIMED(3, loop_mtvscr_lvebx(n));
    TIMED(4, loop_lvlx(n));
    TIMED(5, loop_mtvscr_lvlx(n));
    TIMED(6, loop_stvebx(n));
    TIMED(7, loop_mtvscr_stvebx(n));
    TIMED(8, loop_lfs(n));
    TIMED(9, loop_mtvscr_lfs(n));
    TIMED(10, loop_stfs(n));
    TIMED(11, loop_mtvscr_stfs(n));
    TIMED(12, loop_lfdu(n));
    TIMED(13, loop_mtvscr_lfdu(n));
    TIMED(14, loop_stfiwx(n));
    TIMED(15, loop_mtvscr_stfiwx(n));

    put(32, REPS);
    put(33, ITER_LOG2);
    put(34, 0x5050564dull);

    write_frame();
    return 0;
}
