/* PPU probe: what the overflow-enable and recording forms of a signed
 * divide cost on the most negative value by -1, and what divw costs on it
 * when the high words of its operands are not sign extensions. Each loop
 * runs eight divides of the same two registers into r17, so each divide
 * waits for the unpipelined fixed-point divider the one before it holds.
 *
 *   loop  body (8 a pass, 1 << 14 passes, bdnz)
 *   0     addi   r17,r17,1     (control: the loop alone)
 *   1     divw   r17,r15,r16   r15 = 0x0000000080000000, r16 = 0x00000000ffffffff
 *   2     divw   r17,r15,r16   r15 = 0x0000000080000000, r16 = 1
 *   3     divwo  r17,r15,r16   r15 = 0xffffffff80000000, r16 = -1
 *   4     divwo  r17,r15,r16   r15 = 0xffffffff80000000, r16 = 1
 *   5     divw.  r17,r15,r16   r15 = 0xffffffff80000000, r16 = -1
 *   6     divw.  r17,r15,r16   r15 = 0xffffffff80000000, r16 = 1
 *   7     divdo  r17,r15,r16   r15 = 0x8000000000000000, r16 = -1
 *   8     divdo  r17,r15,r16   r15 = 0x8000000000000000, r16 = 1
 *   9     divd.  r17,r15,r16   r15 = 0x8000000000000000, r16 = -1
 *   10    divd.  r17,r15,r16   r15 = 0x8000000000000000, r16 = 1
 *   11    divw   r17,r15,r16   r15 = 0x1234567880000000, r16 = 0x9abcdef0ffffffff
 *
 * Every loop clears XER after its bdnz, so a sticky SO from an
 * overflow-enable loop never reaches the next.
 *
 * Payload: big-endian u64 words.
 *   2L, 2L + 1   loop L's smallest and largest time-base delta of REPS runs
 *   24           the sum of every loop's r17 (keeps each loop live)
 *   25           REPS
 *   26           passes a run
 *   27           0x50504446 ("PPDF"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define LOOPS   12
#define WORDS   28
#define PAYLOAD (WORDS * 8)
#define REPS    11
#define PASSES  (1u << 14)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile u64 sink;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define R2(x) x x
#define R4(x) R2(x) R2(x)
#define R8(x) R4(x) R4(x)

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
#define LOOP_END   "bdnz 1b\n" "li 0,0\n" "mtxer 0\n"

/* A loop of `body` with r15 = a, r16 = b, r17 = 0; returns its ticks. */
#define DIV_LOOP(name, body)                                           \
    static u64 __attribute__((noinline)) name(u64 a, u64 b, u64 n)     \
    {                                                                  \
        u64 r = 0;                                                     \
        u64 t0 = timebase();                                           \
        __asm__ volatile (                                             \
            "mr 15,%[a]\n"                                             \
            "mr 16,%[b]\n"                                             \
            "li 17,0\n"                                                \
            LOOP_BEGIN body LOOP_END                                   \
            "mr %[r],17\n"                                             \
            : [r] "=r"(r)                                              \
            : [a] "r"(a), [b] "r"(b), [n] "r"(n)                       \
            : "r0", "r15", "r16", "r17", "ctr", "xer", "cr0");         \
        sink += r;                                                     \
        return timebase() - t0;                                        \
    }

DIV_LOOP(loop_addi, R8("addi 17,17,1\n"))
DIV_LOOP(loop_divw, R8("divw 17,15,16\n"))
DIV_LOOP(loop_divwo, R8("divwo 17,15,16\n"))
DIV_LOOP(loop_divw_rc, R8("divw. 17,15,16\n"))
DIV_LOOP(loop_divdo, R8("divdo 17,15,16\n"))
DIV_LOOP(loop_divd_rc, R8("divd. 17,15,16\n"))

#define LOW_MIN   0x0000000080000000ull
#define LOW_M1    0x00000000ffffffffull
#define WORD_MIN  0xffffffff80000000ull
#define DWORD_MIN 0x8000000000000000ull
#define MINUS_ONE 0xffffffffffffffffull
#define HIGH_MIN  0x1234567880000000ull
#define HIGH_M1   0x9abcdef0ffffffffull

static u64 run_loop(unsigned l)
{
    switch (l) {
    case 0:  return loop_addi(0, 0, PASSES);
    case 1:  return loop_divw(LOW_MIN, LOW_M1, PASSES);
    case 2:  return loop_divw(LOW_MIN, 1, PASSES);
    case 3:  return loop_divwo(WORD_MIN, MINUS_ONE, PASSES);
    case 4:  return loop_divwo(WORD_MIN, 1, PASSES);
    case 5:  return loop_divw_rc(WORD_MIN, MINUS_ONE, PASSES);
    case 6:  return loop_divw_rc(WORD_MIN, 1, PASSES);
    case 7:  return loop_divdo(DWORD_MIN, MINUS_ONE, PASSES);
    case 8:  return loop_divdo(DWORD_MIN, 1, PASSES);
    case 9:  return loop_divd_rc(DWORD_MIN, MINUS_ONE, PASSES);
    case 10: return loop_divd_rc(DWORD_MIN, 1, PASSES);
    default: return loop_divw(HIGH_MIN, HIGH_M1, PASSES);
    }
}

int main(void)
{
    unsigned i, l, r;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

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
    put(24, sink);
    put(25, REPS);
    put(26, PASSES);
    put(27, 0x50504446ull);

    write_frame();
    return 0;
}
