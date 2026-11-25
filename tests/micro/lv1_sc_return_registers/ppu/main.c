/* PPU probe: what lv1's return paths leave in r0, r11, r12, CR, XER,
 * LR and CTR after a user-mode `sc 1`.
 *
 * A game's PPU thread runs in problem state. lv1's `sc` vector answers
 * these calls itself:
 * - a number at or past the end of its hypercall table returns through
 *   `rfid`;
 * - a problem-state call to an entry that is not problem-state
 *   callable returns through `hrfid`.
 *
 * The probe sends only numbers whose answer changes no state, the same
 * numbers as the lv1_problem_state_sc probe:
 * - 256, 0xFFFF and 0x8000000000000000: past the end of the table;
 * - 14: an entry of the live 4.93 table that holds the invalid-call
 *   function;
 * - 74: get_logical_partition_id, which lv1 refuses from problem state.
 * It never sends 255 (lv1's panic call) or any number that allocates,
 * maps or writes.
 *
 * Each number goes twice: once with XER at 0, once with XER[SO] and
 * XER[CA] set. Before each call:
 * - r0, r3 and r12 hold marks: MARK(reg, case) =
 *   0xC0DE000000000000 | reg << 8 | case;
 * - r11 holds the number;
 * - CR holds CR_IN, whose fields are none of the values a compare sets;
 * - LR and CTR hold marks for registers 32 and 33.
 *
 * Payload, big-endian u64 words:
 *   0      NCASES
 *   8      per case in CASES order, 88 bytes each:
 *          the number, the XER sent, HERE (the address 20 bytes before
 *          the `sc`), then r0, r3, r11, r12, CR, XER, LR and CTR as the
 *          call returned them
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

/* CR0 to CR7 = 6, E, 7, D, A, C, B, F: a compare sets one of LT, GT
 * and EQ, so it never leaves 6, 7, A, B, C, D, E or F in a field. */
#define CR_IN 0x6E7DACBFull

#define XER_SO 0x80000000ull
#define XER_CA 0x20000000ull

#define NREC 8 /* r0, r3, r11, r12, CR, XER, LR, CTR */

/* One `sc 1` with number `num`, XER `xer_in` and the case's marks.
 * HERE is the address of the `mflr`; the `sc` is five words later. */
static void sc1(u64 num, u64 xer_in, unsigned c, u64 *here_out, u64 rec[NREC])
{
    register u64 r0 __asm__("0") = MARK(0, c);
    register u64 r3 __asm__("3") = MARK(3, c);
    register u64 r11 __asm__("11") = num;
    register u64 r12 __asm__("12") = MARK(12, c);
    u64 lr_in = MARK(32, c);
    u64 ctr_in = MARK(33, c);
    u64 cr_in = CR_IN;
    u64 here, cr, xer, lr, ctr;
    __asm__ volatile (
        "bl 1f\n"
        "1: mflr %[here]\n"
        "mtlr %[lr_in]\n"
        "mtctr %[ctr_in]\n"
        "mtcrf 0xff, %[cr_in]\n"
        "mtxer %[xer_in]\n"
        ".long " STR(W_SC1) "\n"
        "mfcr %[cr]\n"
        "mfxer %[xer]\n"
        "mflr %[lr]\n"
        "mfctr %[ctr]\n"
        : "+r"(r0), "+r"(r3), "+r"(r11), "+r"(r12),
          [here] "=&r"(here), [cr] "=&r"(cr), [xer] "=&r"(xer),
          [lr] "=&r"(lr), [ctr] "=&r"(ctr)
        : [lr_in] "r"(lr_in), [ctr_in] "r"(ctr_in), [cr_in] "r"(cr_in),
          [xer_in] "r"(xer_in)
        : "r4", "r5", "r6", "r7", "r8", "r9", "r10", "lr", "ctr", "xer",
          "cr0", "cr1", "cr2", "cr3", "cr4", "cr5",
          "cr6", "cr7", "memory"
    );
    *here_out = here;
    rec[0] = r0;
    rec[1] = r3;
    rec[2] = r11;
    rec[3] = r12;
    rec[4] = cr;
    rec[5] = xer;
    rec[6] = lr;
    rec[7] = ctr;
}

static const u64 NUMBERS[] = {
    256,
    0xFFFF,
    0x8000000000000000ull,
    14,
    74,
};
#define NNUMBERS (sizeof(NUMBERS) / sizeof(NUMBERS[0]))
#define NCASES (2 * NNUMBERS)

#define OFF_CASES 8
#define CASE_SIZE (3 * 8 + NREC * 8)
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
    u64 rec[NREC];
    u64 here;
    unsigned i, r;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    put64(0, NCASES);
    for (i = 0; i < NCASES; i++) {
        unsigned base = OFF_CASES + i * CASE_SIZE;
        u64 num = NUMBERS[i % NNUMBERS];
        u64 xer_in = (i < NNUMBERS) ? 0 : (XER_SO | XER_CA);
        sc1(num, xer_in, i, &here, rec);
        put64(base, num);
        put64(base + 8, xer_in);
        put64(base + 16, here);
        for (r = 0; r < NREC; r++)
            put64(base + 24 + r * 8, rec[r]);
    }

    write_frame();
    return 0;
}
