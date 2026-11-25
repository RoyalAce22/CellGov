/* PPU probe: whether an lwsync, an isync or a sync after a store to a
 * line the caches do not hold holds the instructions after it until the
 * line arrives.
 *
 * X is the first line of a 256-byte buffer the probe owns; X + 0x80 is
 * its second line, which no store reaches. `stw v,0(x)` writes X's first
 * word with the value already there. After the barrier, CHAIN dependent
 * `addi` take longer than a line's trip from memory, so a barrier that
 * does not wait for the line lets the chain run under the fetch.
 *
 * Cases (each a loop of ITER iterations; f is the flushed line):
 *   0  dcbf f; sync; stw v,0(x); lwsync; CHAIN addi    f = X
 *   1  dcbf f; sync; stw v,0(x); lwsync; CHAIN addi    f = X + 0x80
 *   2  dcbf f; sync; stw v,0(x); isync;  CHAIN addi    f = X
 *   3  dcbf f; sync; stw v,0(x); isync;  CHAIN addi    f = X + 0x80
 *   4  dcbf f; sync; stw v,0(x); sync;   CHAIN addi    f = X
 *   5  dcbf f; sync; stw v,0(x); sync;   CHAIN addi    f = X + 0x80
 *
 * One untimed run of every case, then REPS repetitions, each one timed
 * run of every case in case order, between two time base reads.
 *
 * Payload (big-endian u32 words):
 *   0  REPS    1  ITER    2  CHAIN    3  X's low word
 *   4 + 2c, 5 + 2c  case c's smallest and largest time base ticks a run
 *  16  X's first word is V after the runs (1 = yes)
 *  17  0x50534D42 ("PSMB"): the probe wrote every word
 * The probe makes no system call before its output, so no step can fail.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS  18
#define CASES  6
#define REPS   8
#define ITER   256
#define CHAIN  256
#define V      0x53544F52u

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int buf[64] __attribute__((aligned(128)));
static volatile unsigned long sink;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = sizeof(result);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)result, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

static inline unsigned long long timebase(void)
{
    unsigned long long tb;
    __asm__ volatile ("sync\n\tmftb %0" : "=r"(tb) : : "memory");
    return tb;
}

#define STR2(x) #x
#define STR(x) STR2(x)

/* One case loop: flush `f`, store `v` at `x`, then BARRIER and the
 * chain. */
#define CASE_LOOP(name, barrier)                                        \
static unsigned int __attribute__((noinline))                           \
name(unsigned long f, unsigned long x, unsigned long v)                 \
{                                                                       \
    unsigned long t = 0;                                                \
    unsigned long long t0 = timebase();                                 \
    __asm__ volatile ("mtctr %4\n"                                      \
                      ".p2align 5\n"                                    \
                      "1:\n\t"                                          \
                      "dcbf 0,%1\n\t"                                   \
                      "sync\n\t"                                        \
                      "stw %3,0(%2)\n\t"                                \
                      barrier "\n\t"                                    \
                      ".rept " STR(CHAIN) "\n\t"                        \
                      "addi %0,%0,1\n\t"                                \
                      ".endr\n\t"                                       \
                      "bdnz 1b"                                         \
                      : "+b"(t)                                         \
                      : "r"(f), "b"(x), "r"(v), "r"((unsigned long)ITER) \
                      : "ctr", "memory");                               \
    t0 = timebase() - t0;                                               \
    sink += t;                                                          \
    return (unsigned int)t0;                                            \
}

CASE_LOOP(store_lwsync, "lwsync")
CASE_LOOP(store_isync, "isync")
CASE_LOOP(store_sync, "sync")

static unsigned int run(unsigned int c, unsigned long x)
{
    unsigned long f = (c & 1) ? x + 0x80 : x;
    switch (c >> 1) {
    case 0: return store_lwsync(f, x, V);
    case 1: return store_isync(f, x, V);
    default: return store_sync(f, x, V);
    }
}

int main(void)
{
    unsigned int c, r;
    unsigned long x = (unsigned long)(const void *)buf;

    memset((void *)result, 0, sizeof(result));
    result[0] = REPS;
    result[1] = ITER;
    result[2] = CHAIN;
    result[3] = (unsigned int)x;

    buf[0] = V;
    __sync_synchronize();

    for (c = 0; c < CASES; c++) {
        result[4 + 2 * c] = 0xFFFFFFFFu;
        run(c, x);
    }
    for (r = 0; r < REPS; r++) {
        for (c = 0; c < CASES; c++) {
            unsigned int t = run(c, x);
            if (t < result[4 + 2 * c])
                result[4 + 2 * c] = t;
            if (t > result[5 + 2 * c])
                result[5 + 2 * c] = t;
        }
    }
    result[16] = buf[0] == V;
    result[17] = 0x50534D42u;
    write_frame();
    return 0;
}
