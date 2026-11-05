/* PPU probe: a store over an instruction the process has not run,
 * pushed to memory but with no `icbi` or `isync`, then a call.
 *
 * The text holds eight functions, each `li r3,0; blr` at the start of
 * its own 128-byte line. Trial i stores `li r3,1` over function i's
 * `li` without calling it first, runs `dcbst` and `sync` on the line,
 * and calls it. The call's r3 is the trial's result: 1 runs the new
 * instruction, 0 the old one. A progress record naming the trial goes to the result file
 * before each store, so a run that ends inside a trial names it.
 *
 * Payload (big-endian u32 words):
 *   0      DONE ("CPTD") once every trial ran, PROGRESS ("PROG") in a
 *          progress record
 *   1      the trial a progress record names, from 1 (0 otherwise)
 *   2      the low 32 bits of function 0's address
 *   3      the number of trials run
 *   4..11  each trial's result
 *   12..15 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 16
#define TRIALS 8
#define DONE     0x43505444u /* "CPTD" */
#define PROGRESS 0x50524F47u /* "PROG" */
#define LI_R3_1  0x38600001u
#define LINE_WORDS 32

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

/* Eight `li r3,0; blr` functions, one per 128-byte text line. */
__asm__(
    ".section .text\n"
    ".align 7\n"
    ".globl cgov_patch_fns\n"
    "cgov_patch_fns:\n"
    ".rept 8\n"
    "li 3,0\n"
    "blr\n"
    ".align 7\n"
    ".endr\n");

extern unsigned int cgov_patch_fns[];

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

/* The r3 the function at `fn` returns. */
static unsigned int call_fn(const unsigned int *fn)
{
    unsigned long r3;
    __asm__ volatile(
        "mtctr %1\n\t"
        "bctrl\n\t"
        "mr %0,3\n\t"
        : "=r"(r3)
        : "r"(fn)
        : "ctr", "lr", "r3", "memory");
    return (unsigned int)r3;
}

/* Store `word` at `at`, then push its line to memory with no
 * instruction-cache invalidate and no context synchronization. */
static void patch(unsigned int *at, unsigned int word)
{
    *(volatile unsigned int *)at = word;
    __asm__ volatile(
        "dcbst 0,%0\n\t"
        "sync\n\t"
        :
        : "r"(at)
        : "memory");
}

int main(void)
{
    unsigned int i;

    memset((void *)result, 0, sizeof(result));
    result[2] = (unsigned int)(unsigned long)cgov_patch_fns;
    for (i = 0; i < TRIALS; i++) {
        unsigned int *fn = cgov_patch_fns + LINE_WORDS * i;
        result[0] = PROGRESS;
        result[1] = i + 1;
        result[3] = i;
        write_file();
        patch(fn, LI_R3_1);
        result[4 + i] = call_fn(fn);
    }
    result[0] = DONE;
    result[1] = 0;
    result[3] = TRIALS;
    write_frame();
    return 0;
}
