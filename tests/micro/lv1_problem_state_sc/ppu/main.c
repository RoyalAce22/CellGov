/* PPU probe: what a user-mode `sc 1` gets back.
 *
 * A game's PPU thread runs in problem state. An `sc` with LEV = 1 enters
 * the hypervisor from either processor state. lv1's vector answers
 * these calls before any handler runs:
 * - a number at or past the end of its hypercall table gets one value
 *   in r3 and leaves r4 to r10 as they were;
 * - a problem-state call to an entry that is not problem-state callable
 *   gets another value in r3, with r4 to r10 cleared.
 *
 * The probe sends only numbers whose answer changes no state even when
 * the vector lets the call through:
 * - 256, 0xFFFF and 0x8000000000000000: past the end of the table;
 * - 14: an entry of the live 4.93 table that holds the invalid-call
 *   function;
 * - 74: get_logical_partition_id, a handler that only reads.
 * It never sends 255 (lv1's panic call) or any number that allocates,
 * maps or writes.
 *
 * Before each call, r3 to r10 hold marks: MARK(reg, case) =
 * 0xC0DE000000000000 | reg << 8 | case, for reg 3 to 10.
 *
 * Payload, big-endian u64 words:
 *   0      NCASES
 *   8      per case in CASES order, 72 bytes each: the number sent in
 *          r11, then r3 to r10 as the call returned them
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define W_SC1 0x44000022 /* sc with LEV = 1 */
#define LEV_LOW(w) (((w) >> 5) & 1)
_Static_assert(LEV_LOW(W_SC1) == 1, "LEV 1");

#define STR_(x) #x
#define STR(x) STR_(x)

#define MARK(reg, c) (0xC0DE000000000000ull | ((u64)(reg) << 8) | (u64)(c))

#define NREGS 8 /* r3 to r10 */

/* One `sc 1` with number `num` and the case's marks in r3 to r10. The
 * hypervisor's return path may change r0, r11, r12, CR, LR, CTR and
 * XER; the clobbers name them. */
static void sc1(u64 num, unsigned c, u64 regs[NREGS])
{
    register u64 r3 __asm__("3") = MARK(3, c);
    register u64 r4 __asm__("4") = MARK(4, c);
    register u64 r5 __asm__("5") = MARK(5, c);
    register u64 r6 __asm__("6") = MARK(6, c);
    register u64 r7 __asm__("7") = MARK(7, c);
    register u64 r8 __asm__("8") = MARK(8, c);
    register u64 r9 __asm__("9") = MARK(9, c);
    register u64 r10 __asm__("10") = MARK(10, c);
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        ".long " STR(W_SC1) "\n"
        : "+r"(r3), "+r"(r4), "+r"(r5), "+r"(r6), "+r"(r7), "+r"(r8),
          "+r"(r9), "+r"(r10), "+r"(r11)
        :
        : "r0", "r12", "lr", "ctr", "xer", "cr0", "cr1", "cr2", "cr3",
          "cr4", "cr5", "cr6", "cr7", "memory"
    );
    regs[0] = r3;
    regs[1] = r4;
    regs[2] = r5;
    regs[3] = r6;
    regs[4] = r7;
    regs[5] = r8;
    regs[6] = r9;
    regs[7] = r10;
}

static const u64 CASES[] = {
    256,
    0xFFFF,
    0x8000000000000000ull,
    14,
    74,
};
#define NCASES (sizeof(CASES) / sizeof(CASES[0]))

#define OFF_CASES 8
#define CASE_SIZE (8 + NREGS * 8)
#define PAYLOAD   (OFF_CASES + NCASES * CASE_SIZE)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void put64(unsigned off, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        out[off + i] = (unsigned char)(v >> (56 - 8 * i));
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

int main(void)
{
    u64 regs[NREGS];
    unsigned i, r;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    put64(0, NCASES);
    for (i = 0; i < NCASES; i++) {
        unsigned base = OFF_CASES + i * CASE_SIZE;
        sc1(CASES[i], i, regs);
        put64(base, CASES[i]);
        for (r = 0; r < NREGS; r++)
            put64(base + 8 + r * 8, regs[r]);
    }

    write_frame();
    return 0;
}
