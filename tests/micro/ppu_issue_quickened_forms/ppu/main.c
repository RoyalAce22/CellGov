/* PPU probe: the time of two instruction forms whose result a no-op or
 * a branch to the next instruction leaves unchanged. Each loop runs a
 * fixed body ITER times between two time-base reads. The probe runs each
 * loop REPS times and records the smallest and the largest delta. The
 * loop branch is bdnz. Each body is 16 instructions.
 *
 * Loops 0 and 1 put `ori 9,9,0` and then `nop` (`ori 0,0,0`) after each
 * add of a dependent add chain. Loops 2 to 4 pair a word compare with a
 * conditional branch to the next instruction. Loop 2's branch after
 * cmpwi is taken, loop 3's falls through, and loop 4's after cmpw is
 * taken.
 *
 * Payload: big-endian u64 words. Loop i's smallest delta is word 2i and
 * its largest word 2i + 1.
 *   loop
 *   0  8 x (add 9,9,15; ori 9,9,0)                 (1 << 18 iterations)
 *   1  8 x (add 9,9,15; nop)                       (1 << 18)
 *   2  8 x (cmpwi 14,0; beq to the next), r14 = 0  (1 << 20)
 *   3  8 x (cmpwi 14,0; bne to the next), r14 = 0  (1 << 20)
 *   4  8 x (cmpw 14,15; beq to the next), r14 = r15 = 0  (1 << 20)
 *   word
 *  10  the sum of loops 0 and 1's r9 (keeps each chain live)
 *  11  REPS
 *  12  0x50505146 ("PPQF"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS   16
#define PAYLOAD (WORDS * 8)
#define REPS    5

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

/* An add chain in r9 with `tail` after each add. */
#define ADD_THEN(name, tail)                                       \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 a;                                                     \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "li 9,1\n" "li 15,3\n"                                 \
            LOOP_BEGIN R8("add 9,9,15\n" tail "\n") LOOP_END       \
            "mr %[a],9\n"                                          \
            : [a] "=r"(a) : [n] "r"(n) : "r9", "r15", "ctr");      \
        sink += a;                                                 \
        return timebase() - t0;                                    \
    }

ADD_THEN(loop_add_ori, "ori 9,9,0")
ADD_THEN(loop_add_nop, "nop")

/* A compare and a branch to the next instruction, with r14 = r15 = 0. */
#define CMP_BRANCH(name, pair)                                     \
    static u64 __attribute__((noinline)) name(u64 n)               \
    {                                                              \
        u64 t0 = timebase();                                       \
        __asm__ volatile (                                         \
            "li 14,0\n" "li 15,0\n"                                \
            LOOP_BEGIN R8(pair " 2f\n" "2:\n") LOOP_END            \
            : : [n] "r"(n) : "r14", "r15", "cr0", "ctr");          \
        return timebase() - t0;                                    \
    }

CMP_BRANCH(loop_cmpwi_beq, "cmpwi 14,0\n" "beq")
CMP_BRANCH(loop_cmpwi_bne, "cmpwi 14,0\n" "bne")
CMP_BRANCH(loop_cmpw_beq, "cmpw 14,15\n" "beq")

int main(void)
{
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    TIMED(0, loop_add_ori(1u << 18));
    TIMED(1, loop_add_nop(1u << 18));
    TIMED(2, loop_cmpwi_beq(1u << 20));
    TIMED(3, loop_cmpwi_bne(1u << 20));
    TIMED(4, loop_cmpw_beq(1u << 20));

    put(10, sink);
    put(11, REPS);
    put(12, 0x50505146ull);

    write_frame();
    return 0;
}
