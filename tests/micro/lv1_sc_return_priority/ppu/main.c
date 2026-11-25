/* PPU probe: whether the thread priority a game thread sets reaches the
 * r0 that lv1's return gives after a user-mode `sc 1`.
 *
 * lv1's `sc` vector saves TSRL at entry and loads the saved value into
 * r0 on both returns. TSRL[TP] holds the thread's priority. A thread
 * asks for another priority with one of the `or` forms:
 * - `or 1,1,1`: low;
 * - `or 2,2,2`: medium;
 * - `or 3,3,3`: high.
 * The processor ignores a change that the privilege state may not make,
 * so each form is a plain no-op where it is not allowed.
 *
 * The probe sends only numbers whose answer changes no state, numbers
 * the lv1_problem_state_sc and lv1_sc_return_registers probes sent:
 * - 256: past the end of the table, the rfid return;
 * - 74: get_logical_partition_id, which lv1 refuses from problem state
 *   through the hrfid return.
 *
 * Each case runs its form, if any, right before the `sc`, and `or 2,2,2`
 * right after it, so the thread is back at medium before the next case.
 *
 * Payload, big-endian u64 words:
 *   0      NCASES
 *   8      per case in CASES order, 32 bytes each:
 *          the number, the form (0 none, 1 low, 2 medium, 3 high),
 *          then r0 and r3 as the call returned them
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

/* One `sc 1` with number `num` after the priority form `FORM`, then
 * `or 2,2,2`. r0 and r3 hold the case's marks before the call. */
#define DEFINE_SC1(name, FORM)                                          \
static void name(u64 num, unsigned c, u64 *r0_out, u64 *r3_out)         \
{                                                                       \
    register u64 r0 __asm__("0") = MARK(0, c);                          \
    register u64 r3 __asm__("3") = MARK(3, c);                          \
    register u64 r11 __asm__("11") = num;                               \
    __asm__ volatile (                                                  \
        FORM                                                            \
        ".long " STR(W_SC1) "\n"                                        \
        "or 2,2,2\n"                                                    \
        : "+r"(r0), "+r"(r3), "+r"(r11)                                 \
        :                                                               \
        : "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r12",             \
          "lr", "ctr", "xer", "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", \
          "cr6", "cr7", "memory"                                        \
    );                                                                  \
    *r0_out = r0;                                                       \
    *r3_out = r3;                                                       \
}

DEFINE_SC1(sc1_none, "")
DEFINE_SC1(sc1_low, "or 1,1,1\n")
DEFINE_SC1(sc1_medium, "or 2,2,2\n")
DEFINE_SC1(sc1_high, "or 3,3,3\n")

typedef void (*sc1_fn)(u64, unsigned, u64 *, u64 *);

static const sc1_fn FORMS[] = { sc1_none, sc1_low, sc1_medium, sc1_high };
#define NFORMS (sizeof(FORMS) / sizeof(FORMS[0]))

static const u64 NUMBERS[] = { 256, 74 };
#define NNUMBERS (sizeof(NUMBERS) / sizeof(NUMBERS[0]))
#define NCASES (NFORMS * NNUMBERS)

#define OFF_CASES 8
#define CASE_SIZE 32
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
    u64 r0, r3;
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    put64(0, NCASES);
    for (i = 0; i < NCASES; i++) {
        unsigned base = OFF_CASES + i * CASE_SIZE;
        unsigned form = i / NNUMBERS;
        u64 num = NUMBERS[i % NNUMBERS];
        FORMS[form](num, i, &r0, &r3);
        put64(base, num);
        put64(base + 8, form);
        put64(base + 16, r0);
        put64(base + 24, r3);
    }

    write_frame();
    return 0;
}
