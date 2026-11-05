/* PPU probe: the arguments the main thread starts with.
 *
 * The start stub (ppu/entry_crt0.S) stores r1 and r3 to r6 at the
 * first instruction. The probe records them, the argv and envp slots
 * they name, and the string argv[0] names.
 *
 * Payload (big-endian u32 words):
 *   0  DONE ("ARGV") once every word is written
 *   1  r3 (argc), low word
 *   2  r4 (argv), low word
 *   3  r5 (envp), low word
 *   4  r6, low word
 *   5  r1, low word
 *   6, 7  argv[0], high and low words (0 when r4 is 0)
 *   8, 9  argv[1], high and low words (0 when r4 is 0)
 *   10, 11  envp[0], high and low words (0 when r5 is 0)
 *   12  the length of the string argv[0] names (0 when it is null)
 *   13  r1 in main, low word
 *   14, 15  0
 *   16..31  that string's first 63 bytes and a NUL, zero-padded
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 32
#define DONE 0x41524756u /* "ARGV" */

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

/* r1, r3, r4, r5 and r6 at the first instruction; entry_crt0.S fills it. */
unsigned long long cgov_entry_args[5] __attribute__((aligned(8)));

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
    CGOV_OUT_FILE_WRITE((const void *)result, sizeof(result));
}

/* The 8-byte slot at the 32-bit address `at`. */
static unsigned long long slot(unsigned long long at, unsigned int index)
{
    return ((volatile unsigned long long *)(unsigned long)(unsigned int)at)[index];
}

static void put_slot(unsigned int word, unsigned long long value)
{
    result[word] = (unsigned int)(value >> 32);
    result[word + 1] = (unsigned int)value;
}

int main(void)
{
    unsigned long long r1 = cgov_entry_args[0];
    unsigned long long argv = cgov_entry_args[2];
    unsigned long long envp = cgov_entry_args[3];
    unsigned long long arg0 = 0;
    unsigned long long r1_main;
    unsigned int i;

    __asm__ volatile ("mr %0, 1" : "=r"(r1_main));
    memset((void *)result, 0, sizeof(result));
    result[13] = (unsigned int)r1_main;
    result[1] = (unsigned int)cgov_entry_args[1];
    result[2] = (unsigned int)argv;
    result[3] = (unsigned int)envp;
    result[4] = (unsigned int)cgov_entry_args[4];
    result[5] = (unsigned int)r1;
    if (argv != 0) {
        arg0 = slot(argv, 0);
        put_slot(6, arg0);
        put_slot(8, slot(argv, 1));
    }
    if (envp != 0)
        put_slot(10, slot(envp, 0));
    if (arg0 != 0) {
        const volatile char *s = (const volatile char *)(unsigned long)(unsigned int)arg0;
        volatile unsigned char *out = (volatile unsigned char *)&result[16];
        for (i = 0; i < 63 && s[i] != 0; i++)
            out[i] = (unsigned char)s[i];
        result[12] = i;
    }
    result[0] = DONE;
    write_frame();
    return 0;
}
