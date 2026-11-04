/* PPU probe: a branch whose target is the second of two adjacent
 * instructions runs the instruction at the target. Each case branches
 * over the first instruction of a pair and records what the second one
 * did. The pairs are the ones an interpreter may fuse into one
 * operation: lwz+cmpwi, li+stw, mflr+stw, lwz+mtlr, mflr+std, ld+mtlr,
 * std+std, cmpwi+bc and cmpw+bc. The last case reaches the second
 * instruction through bctr instead of b.
 *
 * Every case starts from r3 = 9, r4 = the data buffer, and a data buffer
 * of 0xEE bytes; words 2 and 3 of the buffer (offset 8) hold 0x11223344
 * and 0x55667788, the value a skipped first-instruction load would have
 * read.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2  lwz+cmpwi: CR field 0 after "cmpwi 0,3,5" (a nibble)
 *   3  lwz+cmpwi: r3 (low word)
 *   4  li+stw: the word stw stored
 *   5  mflr+stw: the word stw stored
 *   6  lwz+mtlr: LR (low word) after mtlr
 *   7  mflr+std: high word std stored   8  its low word
 *   9  ld+mtlr: LR (low word) after mtlr
 *  10  std+std: buffer word 0 after the case (first std skipped)
 *  11  std+std: high word of the second std   12  its low word
 *  13  cmpwi+bc: r6 (0 = the bc was taken)
 *  14  cmpw+bc: r6 (0 = the bc was taken)
 *  15  li+stw through bctr: the word stw stored
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 16

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int data[8] __attribute__((aligned(128)));
static volatile unsigned int out[2] __attribute__((aligned(8)));

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
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

static void reset(void)
{
    unsigned int i;
    for (i = 0; i < 8; i++)
        data[i] = 0xEEEEEEEEu;
    data[2] = 0x11223344u;
    data[3] = 0x55667788u;
    out[0] = 0;
    out[1] = 0;
    __sync_synchronize();
}

/* r3 = 9 and r4 = &data ahead of each case. */
#define PROLOGUE "li 3,9\n\tmr 4,%0\n\t"

int main(void)
{
    memset((void *)result, 0, sizeof(result));

    reset();
    __asm__ volatile (PROLOGUE
                      "b 1f\n\t"
                      "lwz 3,8(4)\n"
                      "1:\tcmpwi 0,3,5\n\t"
                      "mfcr 5\n\t"
                      "stw 5,0(%1)\n\t"
                      "stw 3,4(%1)"
                      : : "b"(data), "b"(out) : "r3", "r4", "r5", "cr0", "memory");
    result[2] = (out[0] >> 28) & 0xF;
    result[3] = out[1];

    reset();
    __asm__ volatile (PROLOGUE
                      "b 1f\n\t"
                      "li 3,7\n"
                      "1:\tstw 3,0(4)"
                      : : "b"(data), "b"(out) : "r3", "r4", "memory");
    result[4] = data[0];

    reset();
    __asm__ volatile (PROLOGUE
                      "b 1f\n\t"
                      "mflr 3\n"
                      "1:\tstw 3,0(4)"
                      : : "b"(data), "b"(out) : "r3", "r4", "memory");
    result[5] = data[0];

    reset();
    __asm__ volatile (PROLOGUE
                      "b 1f\n\t"
                      "lwz 3,8(4)\n"
                      "1:\tmtlr 3\n\t"
                      "mflr 5\n\t"
                      "stw 5,0(%1)"
                      : : "b"(data), "b"(out) : "r3", "r4", "r5", "lr", "memory");
    result[6] = out[0];

    reset();
    __asm__ volatile (PROLOGUE
                      "b 1f\n\t"
                      "mflr 3\n"
                      "1:\tstd 3,0(4)"
                      : : "b"(data), "b"(out) : "r3", "r4", "memory");
    result[7] = data[0];
    result[8] = data[1];

    reset();
    __asm__ volatile (PROLOGUE
                      "b 1f\n\t"
                      "ld 3,8(4)\n"
                      "1:\tmtlr 3\n\t"
                      "mflr 5\n\t"
                      "stw 5,0(%1)"
                      : : "b"(data), "b"(out) : "r3", "r4", "r5", "lr", "memory");
    result[9] = out[0];

    reset();
    __asm__ volatile (PROLOGUE
                      "lis 5,0x0102\n\t"
                      "ori 5,5,0x0304\n\t"
                      "sldi 5,5,32\n\t"
                      "oris 5,5,0x0506\n\t"
                      "ori 5,5,0x0708\n\t"
                      "b 1f\n\t"
                      "std 3,0(4)\n"
                      "1:\tstd 5,8(4)"
                      : : "b"(data), "b"(out) : "r3", "r4", "r5", "memory");
    result[10] = data[0];
    result[11] = data[2];
    result[12] = data[3];

    reset();
    __asm__ volatile (PROLOGUE
                      "li 6,0\n\t"
                      "b 1f\n\t"
                      "cmpwi 0,3,5\n"
                      "1:\tbc 20,0,2f\n\t"
                      "li 6,1\n"
                      "2:\tstw 6,0(%1)"
                      : : "b"(data), "b"(out) : "r3", "r4", "r6", "cr0", "memory");
    result[13] = out[0];

    reset();
    __asm__ volatile (PROLOGUE
                      "li 5,7\n\t"
                      "li 6,0\n\t"
                      "b 1f\n\t"
                      "cmpw 0,3,5\n"
                      "1:\tbc 20,0,2f\n\t"
                      "li 6,1\n"
                      "2:\tstw 6,0(%1)"
                      : : "b"(data), "b"(out) : "r3", "r4", "r5", "r6", "cr0", "memory");
    result[14] = out[0];

    reset();
    __asm__ volatile (PROLOGUE
                      "bl 0f\n"
                      "0:\tmflr 7\n\t"
                      "addi 7,7,1f-0b\n\t"
                      "mtctr 7\n\t"
                      "bctr\n\t"
                      "li 3,7\n"
                      "1:\tstw 3,0(4)"
                      : : "b"(data), "b"(out) : "r3", "r4", "r7", "lr", "ctr", "memory");
    result[15] = data[0];

    write_frame();
    return 0;
}
