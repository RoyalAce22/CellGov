/* PPU probe: the cache-block instructions leave memory as they found it,
 * and the touch and data-stream hints run to completion on an address
 * no page holds.
 *
 * Part 1 fills two 128-byte lines with a pattern, then runs dcbst on the
 * first line, dcbf on the second and icbi on the first, and reads both
 * lines back.
 *
 * Part 2 runs dcbt, dcbtst, dst, dstt, dstst, dststt, dss and dssall with
 * an effective address in page 0, which no page holds, and records a
 * marker after each. A fault there ends the process before the frame is
 * written.
 *
 * Payload (big-endian u32 words):
 *   0  failed step (0 = none)   1  its return code
 *   2..9    words 0, 1, 30, 31 of line 0, then of line 1, after part 1
 *   10..17  marker n + 1 after hint n of part 2, in the order above
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define HINTS 8
#define WORDS (2 + 8 + HINTS)
#define PATTERN 0x5A5AC3C3u

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int lines[64] __attribute__((aligned(128)));

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

int main(void)
{
    unsigned int i;
    unsigned long long unheld = 0x100;
    unsigned long long stream = 0x00200101ull; /* block size 2, count 1, stride 0x101 */

    memset((void *)result, 0, sizeof(result));

    for (i = 0; i < 64; i++)
        lines[i] = PATTERN ^ i;
    __asm__ volatile ("dcbst 0,%0" : : "r"(&lines[0]) : "memory");
    __asm__ volatile ("dcbf 0,%0" : : "r"(&lines[32]) : "memory");
    __asm__ volatile ("icbi 0,%0" : : "r"(&lines[0]) : "memory");
    __asm__ volatile ("sync\n\tisync" : : : "memory");
    result[2] = lines[0];
    result[3] = lines[1];
    result[4] = lines[30];
    result[5] = lines[31];
    result[6] = lines[32];
    result[7] = lines[33];
    result[8] = lines[62];
    result[9] = lines[63];

    __asm__ volatile ("dcbt 0,%0" : : "r"(unheld) : "memory");
    result[10] = 1;
    __asm__ volatile ("dcbtst 0,%0" : : "r"(unheld) : "memory");
    result[11] = 2;
    __asm__ volatile ("dst %0,%1,0" : : "b"(unheld), "r"(stream) : "memory");
    result[12] = 3;
    __asm__ volatile ("dstt %0,%1,1" : : "b"(unheld), "r"(stream) : "memory");
    result[13] = 4;
    __asm__ volatile ("dstst %0,%1,2" : : "b"(unheld), "r"(stream) : "memory");
    result[14] = 5;
    __asm__ volatile ("dststt %0,%1,3" : : "b"(unheld), "r"(stream) : "memory");
    result[15] = 6;
    __asm__ volatile ("dss 0" : : : "memory");
    result[16] = 7;
    __asm__ volatile ("dssall" : : : "memory");
    result[17] = 8;

    write_frame();
    return 0;
}
