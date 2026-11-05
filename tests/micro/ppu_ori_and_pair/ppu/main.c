/* PPU probe: the registers and CR0 that an "ori" followed by an "and"
 * leave. One case chains the and on the ori's result with the and
 * recording, one has the nop ("ori 0,0,0") before a recording and, one
 * overwrites one register twice, one records a zero result, one branches
 * over the ori into the and, one has the and write the ori's source, and
 * one records under XER[SO].
 *
 * Before each case CR0 is set to 0xF and XER to 0 (or to SO alone in
 * the last case), so an and that does not record leaves CR0 at 0xF.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * A = 0x0123456789ABCDEF, N = 0x8000000000000001, B = 0xF0F0F0F0FFFF8001,
 * C = 0x00000000CAFEF00D, Z = 0xFEDCBA9876543210 (the complement of A|1).
 *
 * Payload (big-endian u32 words; each register is high word then low;
 * each CR0 word is the 4-bit field LT GT EQ SO in its low bits):
 *   0  failed step (0 = none)   1  its return code
 *   2  r4 = A, r6 = B; ori 3,4,0x80f0 ; and. 5,3,6: r3 high   3  r3 low
 *   4  the same case: r5 high   5  r5 low   6  CR0
 *   7  r4 = N, r6 = B; nop ; and. 5,4,6: r5 high   8  r5 low   9  CR0
 *  10  r4 = A, r6 = B; ori 4,4,0 ; and 4,4,6: r4 high  11  r4 low  12  CR0
 *  13  r4 = A, r7 = Z; ori 3,4,1 ; and. 5,3,7: r3 high  14  r3 low
 *  15  the same case: r5 high  16  r5 low  17  CR0
 *  18  r3 = C, r4 = A, r6 = B; "b 1f ; ori 3,4,0xffff ; 1: and 5,3,6":
 *      r3 high  19  r3 low
 *  20  the same case: r5 high  21  r5 low
 *  22  r4 = A, r6 = B; ori 3,4,0xf0 ; and 4,3,6: r3 high  23  r3 low
 *  24  the same case: r4 high  25  r4 low
 *  26  r4 = A, r6 = B, XER = SO; ori 3,4,1 ; and. 5,3,6: r3 high  27  r3 low
 *  28  the same case: r5 high  29  r5 low  30  CR0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 31

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

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

static void put(unsigned int word, unsigned long long v)
{
    result[word] = (unsigned int)(v >> 32);
    result[word + 1] = (unsigned int)v;
}

static void put_cr0(unsigned int word, unsigned long long cr)
{
    result[word] = (unsigned int)((cr >> 28) & 0xF);
}

int main(void)
{
    volatile unsigned long long va = 0x0123456789ABCDEFULL;
    volatile unsigned long long vn = 0x8000000000000001ULL;
    volatile unsigned long long vb = 0xF0F0F0F0FFFF8001ULL;
    volatile unsigned long long vc = 0x00000000CAFEF00DULL;
    volatile unsigned long long vz = 0xFEDCBA9876543210ULL;
    volatile unsigned long long vcr = 0xF0000000ULL;
    volatile unsigned long long vso = 0x80000000ULL;
    unsigned long long a = va;
    unsigned long long n = vn;
    unsigned long long b = vb;
    unsigned long long c = vc;
    unsigned long long z = vz;
    unsigned long long crf = vcr;
    unsigned long long so = vso;
    unsigned long long zero = 0;
    unsigned long long r, s, cr;

    memset((void *)result, 0, sizeof(result));

    __asm__ volatile ("mtcrf 0x80,%5\n\t"
                      "mtxer %6\n\t"
                      "mr 4,%3\n\t"
                      "mr 6,%4\n\t"
                      "ori 3,4,0x80f0\n\t"
                      "and. 5,3,6\n\t"
                      "mfcr %2\n\t"
                      "mr %0,3\n\t"
                      "mr %1,5"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(b), "r"(crf), "r"(zero)
                      : "r3", "r4", "r5", "r6", "cr0", "xer");
    put(2, r);
    put(4, s);
    put_cr0(6, cr);

    __asm__ volatile ("mtcrf 0x80,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 4,%2\n\t"
                      "mr 6,%3\n\t"
                      "nop\n\t"
                      "and. 5,4,6\n\t"
                      "mfcr %1\n\t"
                      "mr %0,5"
                      : "=&r"(s), "=&r"(cr)
                      : "r"(n), "r"(b), "r"(crf), "r"(zero)
                      : "r4", "r5", "r6", "cr0", "xer");
    put(7, s);
    put_cr0(9, cr);

    __asm__ volatile ("mtcrf 0x80,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 4,%2\n\t"
                      "mr 6,%3\n\t"
                      "ori 4,4,0\n\t"
                      "and 4,4,6\n\t"
                      "mfcr %1\n\t"
                      "mr %0,4"
                      : "=&r"(r), "=&r"(cr)
                      : "r"(a), "r"(b), "r"(crf), "r"(zero)
                      : "r4", "r6", "cr0", "xer");
    put(10, r);
    put_cr0(12, cr);

    __asm__ volatile ("mtcrf 0x80,%5\n\t"
                      "mtxer %6\n\t"
                      "mr 4,%3\n\t"
                      "mr 7,%4\n\t"
                      "ori 3,4,1\n\t"
                      "and. 5,3,7\n\t"
                      "mfcr %2\n\t"
                      "mr %0,3\n\t"
                      "mr %1,5"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(z), "r"(crf), "r"(zero)
                      : "r3", "r4", "r5", "r7", "cr0", "xer");
    put(13, r);
    put(15, s);
    put_cr0(17, cr);

    __asm__ volatile ("mr 3,%2\n\t"
                      "mr 4,%3\n\t"
                      "mr 6,%4\n\t"
                      "b 1f\n\t"
                      "ori 3,4,0xffff\n"
                      "1:\tand 5,3,6\n\t"
                      "mr %0,3\n\t"
                      "mr %1,5"
                      : "=&r"(r), "=&r"(s)
                      : "r"(c), "r"(a), "r"(b)
                      : "r3", "r4", "r5", "r6");
    put(18, r);
    put(20, s);

    __asm__ volatile ("mr 4,%2\n\t"
                      "mr 6,%3\n\t"
                      "ori 3,4,0xf0\n\t"
                      "and 4,3,6\n\t"
                      "mr %0,3\n\t"
                      "mr %1,4"
                      : "=&r"(r), "=&r"(s)
                      : "r"(a), "r"(b)
                      : "r3", "r4", "r6");
    put(22, r);
    put(24, s);

    __asm__ volatile ("mtcrf 0x80,%5\n\t"
                      "mtxer %6\n\t"
                      "mr 4,%3\n\t"
                      "mr 6,%4\n\t"
                      "ori 3,4,1\n\t"
                      "and. 5,3,6\n\t"
                      "mfcr %2\n\t"
                      "mtxer %7\n\t"
                      "mr %0,3\n\t"
                      "mr %1,5"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(b), "r"(crf), "r"(so), "r"(zero)
                      : "r3", "r4", "r5", "r6", "cr0", "xer");
    put(26, r);
    put(28, s);
    put_cr0(30, cr);

    write_frame();
    return 0;
}
