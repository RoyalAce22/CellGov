/* PPU probe: the registers and CR0 that two adjacent "rldicl" leave,
 * raw or written as their extended mnemonics "srdi" and "clrldi". One
 * case chains the second half on the first half's result with only the
 * second recording, one overwrites one register twice, one records only
 * the first half, one records both halves, one branches over the first
 * half into the second, one has the second half write the first half's
 * source, and one records both halves with XER[SO] set.
 *
 * Before each case CR0 is set to 0xF and XER to 0 (or to SO alone in
 * the last case), so a half that does not record leaves CR0 at 0xF.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * A = 0x0123456789ABCDEF, N = 0x8000000000000001, C = 0x00000000CAFEF00D.
 *
 * Payload (big-endian u32 words; each register is high word then low;
 * each CR0 word is the 4-bit field LT GT EQ SO in its low bits):
 *   0  failed step (0 = none)   1  its return code
 *   2  r4 = A; rldicl 3,4,8,4 ; rldicl. 5,3,0,32: r3 high   3  r3 low
 *   4  the same case: r5 high   5  r5 low   6  CR0
 *   7  r4 = A; srdi 3,4,4 ; clrldi 3,3,32: r3 high   8  r3 low   9  CR0
 *  10  r4 = N; rldicl. 3,4,0,0 ; rldicl 5,3,62,63: r3 high   11  r3 low
 *  12  the same case: r5 high  13  r5 low  14  CR0
 *  15  r4 = N; rldicl. 3,4,0,0 ; rldicl. 5,3,62,63: r3 high  16  r3 low
 *  17  the same case: r5 high  18  r5 low  19  CR0
 *  20  r4 = A, r5 = C; "b 1f ; rldicl 5,4,8,4 ; 1: rldicl 6,5,0,32":
 *      r5 high   21  r5 low
 *  22  the same case: r6 high  23  r6 low
 *  24  r4 = A; rldicl 3,4,63,0 ; rldicl 4,3,1,0: r3 high   25  r3 low
 *  26  the same case: r4 high  27  r4 low
 *  28  r4 = A, XER = SO; srdi. 3,4,4 ; clrldi. 5,3,32: r3 high  29  r3 low
 *  30  the same case: r5 high  31  r5 low  32  CR0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 33

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
    volatile unsigned long long vc = 0x00000000CAFEF00DULL;
    volatile unsigned long long vcr = 0xF0000000ULL;
    volatile unsigned long long vso = 0x80000000ULL;
    unsigned long long a = va;
    unsigned long long n = vn;
    unsigned long long c = vc;
    unsigned long long crf = vcr;
    unsigned long long so = vso;
    unsigned long long zero = 0;
    unsigned long long r, s, cr;

    memset((void *)result, 0, sizeof(result));

    __asm__ volatile ("mtcrf 0x80,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 4,%3\n\t"
                      "rldicl 3,4,8,4\n\t"
                      "rldicl. 5,3,0,32\n\t"
                      "mfcr %2\n\t"
                      "mr %0,3\n\t"
                      "mr %1,5"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(crf), "r"(zero)
                      : "r3", "r4", "r5", "cr0", "xer");
    put(2, r);
    put(4, s);
    put_cr0(6, cr);

    __asm__ volatile ("mtcrf 0x80,%3\n\t"
                      "mtxer %4\n\t"
                      "mr 4,%2\n\t"
                      "srdi 3,4,4\n\t"
                      "clrldi 3,3,32\n\t"
                      "mfcr %1\n\t"
                      "mr %0,3"
                      : "=&r"(r), "=&r"(cr)
                      : "r"(a), "r"(crf), "r"(zero)
                      : "r3", "r4", "cr0", "xer");
    put(7, r);
    put_cr0(9, cr);

    __asm__ volatile ("mtcrf 0x80,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 4,%3\n\t"
                      "rldicl. 3,4,0,0\n\t"
                      "rldicl 5,3,62,63\n\t"
                      "mfcr %2\n\t"
                      "mr %0,3\n\t"
                      "mr %1,5"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(n), "r"(crf), "r"(zero)
                      : "r3", "r4", "r5", "cr0", "xer");
    put(10, r);
    put(12, s);
    put_cr0(14, cr);

    __asm__ volatile ("mtcrf 0x80,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 4,%3\n\t"
                      "rldicl. 3,4,0,0\n\t"
                      "rldicl. 5,3,62,63\n\t"
                      "mfcr %2\n\t"
                      "mr %0,3\n\t"
                      "mr %1,5"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(n), "r"(crf), "r"(zero)
                      : "r3", "r4", "r5", "cr0", "xer");
    put(15, r);
    put(17, s);
    put_cr0(19, cr);

    __asm__ volatile ("mr 4,%2\n\t"
                      "mr 5,%3\n\t"
                      "b 1f\n\t"
                      "rldicl 5,4,8,4\n"
                      "1:\trldicl 6,5,0,32\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(a), "r"(c)
                      : "r4", "r5", "r6");
    put(20, r);
    put(22, s);

    __asm__ volatile ("mr 4,%2\n\t"
                      "rldicl 3,4,63,0\n\t"
                      "rldicl 4,3,1,0\n\t"
                      "mr %0,3\n\t"
                      "mr %1,4"
                      : "=&r"(r), "=&r"(s)
                      : "r"(a)
                      : "r3", "r4");
    put(24, r);
    put(26, s);

    __asm__ volatile ("mtcrf 0x80,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 4,%3\n\t"
                      "srdi. 3,4,4\n\t"
                      "clrldi. 5,3,32\n\t"
                      "mfcr %2\n\t"
                      "mtxer %6\n\t"
                      "mr %0,3\n\t"
                      "mr %1,5"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(crf), "r"(so), "r"(zero)
                      : "r3", "r4", "r5", "cr0", "xer");
    put(28, r);
    put(30, s);
    put_cr0(32, cr);

    write_frame();
    return 0;
}
