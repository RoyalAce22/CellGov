/* PPU probe: whether an instruction waits for a register a load string,
 * a store multiple or an overflow-enabled multiply touches past the
 * registers it names first. Each loop runs four pairs ITER times between
 * two time-base reads. The probe runs each loop REPS times and records
 * the smallest and the largest delta. The loop branch is bdnz.
 *
 *   loop  each pair                              what the second reads
 *   0     lswi r31,p,8   add r3,r0,r0            r0: the wrapped target
 *   1     lswi r31,p,8   add r3,r31,r31          r31: the first target
 *   2     lswi r31,p,4   add r3,r0,r0            r0: no target (control)
 *   3     mullwo r3,r4,r5   mfxer r6             XER: the multiply sets OV
 *   4     mullw r3,r4,r5    mfxer r6             XER: untouched (control)
 *   5     lwz r31,0(q)   stmw r24,0(p)           r31: the eighth source
 *   6     lwz r23,0(q)   stmw r24,0(p)           r23: no source (control)
 *   7     lwz r24,0(q)   stmw r24,0(p)           r24: the first source
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   16    the sum of r3 over the runs (keeps the loops live)
 *   17    REPS
 *   18    ITER
 *   19    0x50505752 ("PPWR"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS   8
#define WORDS   20
#define PAYLOAD (WORDS * 8)
#define REPS    11
#define ITER    (1u << 18)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile unsigned int lines[64] __attribute__((aligned(128)));
static volatile u64 sink;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

#define R2(x) x x
#define R4(x) R2(x) R2(x)

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

#define CLOBBER_R24 "r24", "r25", "r26", "r27", "r28", "r29", "r30", "r31"

/* One timed loop of `body` with r4 = 3, r5 = 5, `p` the first line and
 * `q` the second; returns its ticks. */
#define PAIRS(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 r = 0;                                                 \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "li 4,3\n"                                             \
            "li 5,5\n"                                             \
            "li 3,0\n"                                             \
            LOOP_BEGIN R4(body) LOOP_END                           \
            "mr %[r],3\n"                                          \
            : [r] "=r"(r)                                          \
            : [n] "r"(n), [p] "b"(EA(&lines[0])),                  \
              [q] "b"(EA(&lines[32]))                              \
            : "r0", "r3", "r4", "r5", "r6", "r23", CLOBBER_R24,    \
              "ctr", "xer", "memory");                             \
        sink += r;                                                 \
        return timebase() - t0;                                    \
    }

PAIRS(loop_lswi_wrapped, "lswi 31,%[p],8\n" "add 3,0,0\n")
PAIRS(loop_lswi_first, "lswi 31,%[p],8\n" "add 3,31,31\n")
PAIRS(loop_lswi_none, "lswi 31,%[p],4\n" "add 3,0,0\n")
PAIRS(loop_mullwo_xer, "mullwo 3,4,5\n" "mfxer 6\n")
PAIRS(loop_mullw_xer, "mullw 3,4,5\n" "mfxer 6\n")
PAIRS(loop_stmw_last, "lwz 31,0(%[q])\n" "stmw 24,0(%[p])\n")
PAIRS(loop_stmw_none, "lwz 23,0(%[q])\n" "stmw 24,0(%[p])\n")
PAIRS(loop_stmw_first, "lwz 24,0(%[q])\n" "stmw 24,0(%[p])\n")

static u64 run_loop(unsigned l)
{
    switch (l) {
    case 0: return loop_lswi_wrapped(ITER);
    case 1: return loop_lswi_first(ITER);
    case 2: return loop_lswi_none(ITER);
    case 3: return loop_mullwo_xer(ITER);
    case 4: return loop_mullw_xer(ITER);
    case 5: return loop_stmw_last(ITER);
    case 6: return loop_stmw_none(ITER);
    default: return loop_stmw_first(ITER);
    }
}

int main(void)
{
    unsigned i, l, r;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    for (i = 0; i < 64; i++)
        lines[i] = i;

    for (l = 0; l < LOOPS; l++) {
        u64 lo = ~0ull, hi = 0;
        for (r = 0; r < REPS; r++) {
            u64 t = run_loop(l);
            if (t < lo)
                lo = t;
            if (t > hi)
                hi = t;
        }
        put(2 * l, lo);
        put(2 * l + 1, hi);
    }
    put(16, sink);
    put(17, REPS);
    put(18, ITER);
    put(19, 0x50505752ull);

    write_frame();
    return 0;
}
