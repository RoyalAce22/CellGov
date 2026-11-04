/* PPU probe: whether an mtvscr stalls the VMX and FP loads and stores
 * after it. Each loop runs a fixed body ITER times between two time-base
 * reads. The probe runs each loop REPS times and records the smallest
 * and the largest delta. The loop branch is bdnz. Every vector register
 * starts zero, so mtvscr v0 writes a zero VSCR. Each load and store
 * addresses one 128-byte-aligned buffer the probe zeroes first.
 *
 * Loops 1, 3, 5 and 7 time each load or store alone, and loops 2, 4, 6
 * and 8 the same instruction after an mtvscr. Loop 9 repeats the
 * mtvscr-then-vector loop of ppu_issue_vscr as a reference. Loop 10 puts
 * four independent addi after the lvx, and loop 11 a vaddubm that uses
 * the lvx result, to tell a stalled issue from a stalled VSU half.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  mtvscr v0
 *   1  lvx v1,0,p
 *   2  mtvscr v0; lvx v1,0,p
 *   3  stvx v20,0,p
 *   4  mtvscr v0; stvx v20,0,p
 *   5  lfd f1,0(p)
 *   6  mtvscr v0; lfd f1,0(p)
 *   7  stfd f2,0(p)
 *   8  mtvscr v0; stfd f2,0(p)
 *   9  mtvscr v0; 4 independent vaddubm
 *  10  mtvscr v0; lvx v1,0,p; 4 independent addi
 *  11  mtvscr v0; lvx v1,0,p; vaddubm v2,v1,v1
 *   word
 *  24  REPS
 *  25  log2 of ITER
 *  26  0x5050564d ("PPVM"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS     27
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

#define VADD4                                                   \
    "vaddubm 1,20,21\n" "vaddubm 2,20,21\n"                     \
    "vaddubm 3,20,21\n" "vaddubm 4,20,21\n"
#define ADDI4                                                   \
    "addi 14,14,1\n" "addi 15,15,1\n"                           \
    "addi 16,16,1\n" "addi 17,17,1\n"

#define LVX  "lvx 1,0,%[p]\n"
#define STVX "stvx 20,0,%[p]\n"
#define LFD  "lfd 1,0(%[p])\n"
#define STFD "stfd 2,0(%[p])\n"
#define MTVSCR "mtvscr 0\n"

#define MLOOP(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            LOOP_BEGIN body LOOP_END                               \
            : : [n] "r"(n), [p] "b"(buf)                           \
            : "r14", "r15", "r16", "r17", "ctr", "memory",         \
              "v0", "v1", "v2", "v3", "v4", "v20", "v21",          \
              "fr1", "fr2");                                       \
        return timebase() - t0;                                    \
    }

MLOOP(loop_mtvscr, MTVSCR)
MLOOP(loop_lvx, LVX)
MLOOP(loop_mtvscr_lvx, MTVSCR LVX)
MLOOP(loop_stvx, STVX)
MLOOP(loop_mtvscr_stvx, MTVSCR STVX)
MLOOP(loop_lfd, LFD)
MLOOP(loop_mtvscr_lfd, MTVSCR LFD)
MLOOP(loop_stfd, STFD)
MLOOP(loop_mtvscr_stfd, MTVSCR STFD)
MLOOP(loop_mtvscr_vadd, MTVSCR VADD4)
MLOOP(loop_mtvscr_lvx_addi, MTVSCR LVX ADDI4)
MLOOP(loop_mtvscr_lvx_use, MTVSCR LVX "vaddubm 2,1,1\n")

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
        "vxor 0,0,0\n" "vxor 1,1,1\n" "vxor 20,20,20\n" "vxor 21,21,21\n"
        "lfd 2,0(%[p])\n"
        : : [p] "b"(buf) : "v0", "v1", "v20", "v21", "fr2", "memory");

    TIMED(0, loop_mtvscr(n));
    TIMED(1, loop_lvx(n));
    TIMED(2, loop_mtvscr_lvx(n));
    TIMED(3, loop_stvx(n));
    TIMED(4, loop_mtvscr_stvx(n));
    TIMED(5, loop_lfd(n));
    TIMED(6, loop_mtvscr_lfd(n));
    TIMED(7, loop_stfd(n));
    TIMED(8, loop_mtvscr_stfd(n));
    TIMED(9, loop_mtvscr_vadd(n));
    TIMED(10, loop_mtvscr_lvx_addi(n));
    TIMED(11, loop_mtvscr_lvx_use(n));

    put(24, REPS);
    put(25, ITER_LOG2);
    put(26, 0x5050564dull);

    write_frame();
    return 0;
}
