/* PPU probe: a store into the process's own text segment.
 *
 * The probe reads a word of its own text and stores the same word
 * back, so the store changes no instruction. A
 * progress record naming the step goes to the result file first, so a
 * run that ends at the store leaves that record.
 *
 * Payload (big-endian u32 words):
 *   0  DONE ("STXD") once the store ran, PROGRESS ("PROG") in a
 *      progress record
 *   1  the step a progress record names (0 otherwise)
 *   2  the word read, and stored back
 *   3  the low 32 bits of the word's address
 *   4..7  0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 8
#define DONE     0x53545844u /* "STXD" */
#define PROGRESS 0x50524F47u /* "PROG" */

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

static void write_file(void)
{
    CGOV_OUT_FILE_WRITE((const void *)result, sizeof(result));
}

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
    write_file();
}

/* The address of a word in the text segment: the `mflr` that a `bl` to
 * the next instruction returns to. */
static unsigned int *text_word(void)
{
    unsigned long at;
    __asm__ volatile(
        "bl 1f\n"
        "1:\n\t"
        "mflr %0\n\t"
        : "=r"(at)
        :
        : "lr");
    return (unsigned int *)at;
}

int main(void)
{
    unsigned int *at;
    unsigned int word;

    memset((void *)result, 0, sizeof(result));
    at = text_word();
    word = *(volatile unsigned int *)at;
    result[2] = word;
    result[3] = (unsigned int)(unsigned long)at;

    result[0] = PROGRESS;
    result[1] = 1;
    write_file();
    *(volatile unsigned int *)at = word;

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
