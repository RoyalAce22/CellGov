/* PPU probe: an `sc` word with reserved bits set.
 *
 * The SC form is OPCD 17, LEV in bits 20:26, 1 in bit 30, bit 31
 * reserved. LEV values above 1 are reserved and bits 20:25 of LEV are a
 * reserved field, so the question is whether the PPE reads LEV as bit 26
 * alone and ignores bit 31: whether each word below runs the ordinary
 * system call r11 names.
 *
 * Every word has bit 26 (LEV & 1) clear: no word here asks for the
 * hypervisor. The static asserts below hold that.
 *
 * Calls, in order, for each word W in WORDS (the plain `sc` first):
 *   sc 1 getpid through W, then sc 18 getppid through W.
 * Both calls read no argument and change no state.
 *
 * Payload, big-endian u64 words:
 *   0      index of the first word whose two answers differ from the
 *          plain `sc`'s, or 0xFFFFFFFF when none does
 *   8      NWORDS
 *   16     per word in WORDS order, 24 bytes each: the word, the r3
 *          getpid returned through it, the r3 getppid returned through it
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PROCESS_GETPID  1
#define SYS_PROCESS_GETPPID 18

#define W_PLAIN    0x44000002 /* sc */
#define W_BIT31    0x44000003 /* bit 31 set */
#define W_LEV2     0x44000042 /* LEV field 2: bit 25 set */
#define W_LEV7E    0x44000FC2 /* LEV field 0x7E: bits 20:25 set */
#define W_LEV7E_31 0x44000FC3 /* LEV field 0x7E and bit 31 */

#define LEV_LOW(w) (((w) >> 5) & 1)
_Static_assert(LEV_LOW(W_PLAIN) == 0, "bit 26 clear");
_Static_assert(LEV_LOW(W_BIT31) == 0, "bit 26 clear");
_Static_assert(LEV_LOW(W_LEV2) == 0, "bit 26 clear");
_Static_assert(LEV_LOW(W_LEV7E) == 0, "bit 26 clear");
_Static_assert(LEV_LOW(W_LEV7E_31) == 0, "bit 26 clear");

#define STR_(x) #x
#define STR(x) STR_(x)

/* One system call through `word`, under cgov_sc's register contract. */
#define RAW_SC(fn, word)                                                   \
    static long long fn(u64 num)                                          \
    {                                                                      \
        register u64 r3 __asm__("3") = 0xFFFFFFFFDEADBEEFull;              \
        register u64 r4 __asm__("4") = 0;                                  \
        register u64 r5 __asm__("5") = 0;                                  \
        register u64 r6 __asm__("6") = 0;                                  \
        register u64 r7 __asm__("7") = 0;                                  \
        register u64 r8 __asm__("8") = 0;                                  \
        register u64 r9 __asm__("9") = 0;                                  \
        register u64 r10 __asm__("10") = 0;                                \
        register u64 r11 __asm__("11") = num;                              \
        __asm__ volatile (                                                 \
            ".long " STR(word) "\n"                                        \
            : "+r"(r3), "+r"(r4), "+r"(r5), "+r"(r6), "+r"(r7), "+r"(r8),  \
              "+r"(r9), "+r"(r10), "+r"(r11)                               \
            :                                                              \
            : "r0", "r12", "lr", "ctr", "xer", "cr0", "cr1", "cr5", "cr6", \
              "cr7", "memory"                                              \
        );                                                                 \
        return (long long)r3;                                              \
    }

RAW_SC(sc_plain, W_PLAIN)
RAW_SC(sc_bit31, W_BIT31)
RAW_SC(sc_lev2, W_LEV2)
RAW_SC(sc_lev7e, W_LEV7E)
RAW_SC(sc_lev7e_31, W_LEV7E_31)

typedef long long (*sc_fn)(u64);

static const sc_fn FNS[] = { sc_plain, sc_bit31, sc_lev2, sc_lev7e, sc_lev7e_31 };
static const unsigned int WORDS[] = { W_PLAIN, W_BIT31, W_LEV2, W_LEV7E, W_LEV7E_31 };
#define NWORDS (sizeof(WORDS) / sizeof(WORDS[0]))

#define OFF_WORDS 16
#define PAYLOAD   (OFF_WORDS + NWORDS * 24)

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
    u64 pid[NWORDS], ppid[NWORDS];
    u64 first = 0xFFFFFFFFu;
    unsigned i;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    for (i = 0; i < NWORDS; i++) {
        pid[i] = (u64)FNS[i](SYS_PROCESS_GETPID);
        ppid[i] = (u64)FNS[i](SYS_PROCESS_GETPPID);
        if (first == 0xFFFFFFFFu && (pid[i] != pid[0] || ppid[i] != ppid[0]))
            first = i;
    }

    put64(0, first);
    put64(8, NWORDS);
    for (i = 0; i < NWORDS; i++) {
        put64(OFF_WORDS + i * 24, WORDS[i]);
        put64(OFF_WORDS + i * 24 + 8, pid[i]);
        put64(OFF_WORDS + i * 24 + 16, ppid[i]);
    }

    write_frame();
    return 0;
}
