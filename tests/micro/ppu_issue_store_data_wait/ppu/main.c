/* PPU probe: whether a store, a store string or a store multiple waits
 * for a load into the first register it stores. Each loop runs four pairs
 * ITER times between two time-base reads. The probe runs each loop REPS
 * times and records the smallest and the largest delta. The loop branch
 * is bdnz. The load reads the second line and the store writes the first.
 *
 *   loop  each pair                              what the second reads
 *   0     lwz r24,0(q)   stw r24,0(p)            r24: the loaded register
 *   1     lwz r23,0(q)   stw r24,0(p)            r24: not loaded (control)
 *   2     lwz r24,0(q)   add r3,r24,r24          r24: the loaded register
 *   3     lwz r23,0(q)   add r3,r24,r24          r24: not loaded (control)
 *   4     lwz r24,0(q)   stswi r24,p,32          r24: the first source
 *   5     lwz r23,0(q)   stswi r24,p,32          r24: not loaded (control)
 *   6     lwz r24,0(q)   stmw r24,0(p)           r24: the first source
 *   7     lwz r23,0(q)   stmw r24,0(p)           r24: not loaded (control)
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   16    the sum of r3 over the runs (keeps the loops live)
 *   17    REPS
 *   18    ITER
 *   19    0x50505344 ("PPSD"): the probe wrote every word
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

/* One timed loop of `body` with `p` the first line and `q` the second;
 * returns its ticks. */
#define PAIRS(name, body)                                          \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 r = 0;                                                 \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "li 3,0\n"                                             \
            LOOP_BEGIN R4(body) LOOP_END                           \
            "mr %[r],3\n"                                          \
            : [r] "=r"(r)                                          \
            : [n] "r"(n), [p] "b"(EA(&lines[0])),                  \
              [q] "b"(EA(&lines[32]))                              \
            : "r3", "r23", CLOBBER_R24, "ctr", "memory");          \
        sink += r;                                                 \
        return timebase() - t0;                                    \
    }

PAIRS(loop_stw_loaded, "lwz 24,0(%[q])\n" "stw 24,0(%[p])\n")
PAIRS(loop_stw_none, "lwz 23,0(%[q])\n" "stw 24,0(%[p])\n")
PAIRS(loop_add_loaded, "lwz 24,0(%[q])\n" "add 3,24,24\n")
PAIRS(loop_add_none, "lwz 23,0(%[q])\n" "add 3,24,24\n")
PAIRS(loop_stswi_first, "lwz 24,0(%[q])\n" "stswi 24,%[p],32\n")
PAIRS(loop_stswi_none, "lwz 23,0(%[q])\n" "stswi 24,%[p],32\n")
PAIRS(loop_stmw_first, "lwz 24,0(%[q])\n" "stmw 24,0(%[p])\n")
PAIRS(loop_stmw_none, "lwz 23,0(%[q])\n" "stmw 24,0(%[p])\n")

static u64 run_loop(unsigned l)
{
    switch (l) {
    case 0: return loop_stw_loaded(ITER);
    case 1: return loop_stw_none(ITER);
    case 2: return loop_add_loaded(ITER);
    case 3: return loop_add_none(ITER);
    case 4: return loop_stswi_first(ITER);
    case 5: return loop_stswi_none(ITER);
    case 6: return loop_stmw_first(ITER);
    default: return loop_stmw_none(ITER);
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
    put(19, 0x50505344ull);

    write_frame();
    return 0;
}
