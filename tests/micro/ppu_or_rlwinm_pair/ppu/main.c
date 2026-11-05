/* PPU probe: what an adjacent "or rA,rS,rB ; rlwinm rA2,rS2,SH,MB,ME"
 * leaves -- the or's target, the rotate's target and, for the recording
 * form, CR. The cases are a recording rotate of the value the or just
 * wrote, the mr form with a wrapping mask, a recording rotate with a
 * negative 64-bit result, a recording rotate with a zero result and
 * XER[SO] = 1, the mr form with slwi, a rotate into the or's own target,
 * a rotate of another register, and a branch into the rotate that runs it
 * alone.
 *
 * Every case loads its inputs into r6 and r7 first, and r5, r8 or r9
 * where it names a start value. Each recording case starts with every CR
 * field 0 and XER 0, except where the payload names XER[SO] = 1.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   r6 = 0x0102030400000000, r7 = 0x0000000005060708,
 *   "or 5,6,7 ; rlwinm. 8,5,8,0,23":
 *      2-3 r5    4-5 r8    6 CR
 *   r6 = 0x0102030405060708, "mr 5,6 ; rlwinm 8,5,8,24,7":
 *      7-8 r8
 *   r6 = 0x0000000080000000, r7 = 1, "or 5,6,7 ; rlwinm. 8,5,0,1,0":
 *      9-10 r8  11 CR
 *   r6 = 0xABCDEF0000000000, r7 = 0x0000000000FFFFFF, XER[SO] = 1,
 *   "or 5,6,7 ; rlwinm. 8,5,0,0,7":
 *     12-13 r8  14 CR
 *   r6 = 0xFEDCBA9876543210, "mr 5,6 ; slwi 8,5,3":
 *     15-16 r8
 *   r6 = 0x0123456700000000, r7 = 0x0000000089ABCDEF,
 *   "or 5,6,7 ; rlwinm 5,5,16,0,31":
 *     17-18 r5
 *   r6 = 0x10, r7 = 0x20, r9 = 0x1122334455667788,
 *   "or 5,6,7 ; rlwinm 8,9,4,0,27":
 *     19-20 r5  21-22 r8
 *   r5 = 0xDEADBEEFDEADBEEF, r6 = 0x10, r7 = 0x20, r8 = 0x88,
 *   "b 1f ; or 5,6,7 ; 1: rlwinm 8,5,4,0,27":
 *     23-24 r5  25-26 r8
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 27

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

#define CR_CLOBBERS "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", "cr6", "cr7"

int main(void)
{
    unsigned long long r5, r8, cr;
    unsigned long long zero = 0;

    memset((void *)result, 0, sizeof(result));

    __asm__ volatile ("mtcrf 255,%5\n\t"
                      "mtxer %5\n\t"
                      "mr 6,%3\n\t"
                      "mr 7,%4\n\t"
                      "or 5,6,7\n\t"
                      "rlwinm. 8,5,8,0,23\n\t"
                      "mfcr %2\n\t"
                      "mr %0,5\n\t"
                      "mr %1,8"
                      : "=&r"(r5), "=&r"(r8), "=&r"(cr)
                      : "r"(0x0102030400000000ull), "r"(0x0000000005060708ull),
                        "r"(zero)
                      : "r5", "r6", "r7", "r8", "xer", CR_CLOBBERS);
    put(2, r5);
    put(4, r8);
    result[6] = (unsigned int)cr;

    __asm__ volatile ("mr 6,%1\n\t"
                      "mr 5,6\n\t"
                      "rlwinm 8,5,8,24,7\n\t"
                      "mr %0,8"
                      : "=&r"(r8)
                      : "r"(0x0102030405060708ull)
                      : "r5", "r6", "r8");
    put(7, r8);

    __asm__ volatile ("mtcrf 255,%4\n\t"
                      "mtxer %4\n\t"
                      "mr 6,%2\n\t"
                      "mr 7,%3\n\t"
                      "or 5,6,7\n\t"
                      "rlwinm. 8,5,0,1,0\n\t"
                      "mfcr %1\n\t"
                      "mr %0,8"
                      : "=&r"(r8), "=&r"(cr)
                      : "r"(0x0000000080000000ull), "r"(1ull), "r"(zero)
                      : "r5", "r6", "r7", "r8", "xer", CR_CLOBBERS);
    put(9, r8);
    result[11] = (unsigned int)cr;

    __asm__ volatile ("mtcrf 255,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 6,%2\n\t"
                      "mr 7,%3\n\t"
                      "or 5,6,7\n\t"
                      "rlwinm. 8,5,0,0,7\n\t"
                      "mfcr %1\n\t"
                      "mtxer %4\n\t"
                      "mr %0,8"
                      : "=&r"(r8), "=&r"(cr)
                      : "r"(0xABCDEF0000000000ull), "r"(0x0000000000FFFFFFull),
                        "r"(zero), "r"(0x80000000ull)
                      : "r5", "r6", "r7", "r8", "xer", CR_CLOBBERS);
    put(12, r8);
    result[14] = (unsigned int)cr;

    __asm__ volatile ("mr 6,%1\n\t"
                      "mr 5,6\n\t"
                      "slwi 8,5,3\n\t"
                      "mr %0,8"
                      : "=&r"(r8)
                      : "r"(0xFEDCBA9876543210ull)
                      : "r5", "r6", "r8");
    put(15, r8);

    __asm__ volatile ("mr 6,%1\n\t"
                      "mr 7,%2\n\t"
                      "or 5,6,7\n\t"
                      "rlwinm 5,5,16,0,31\n\t"
                      "mr %0,5"
                      : "=&r"(r5)
                      : "r"(0x0123456700000000ull), "r"(0x0000000089ABCDEFull)
                      : "r5", "r6", "r7");
    put(17, r5);

    __asm__ volatile ("li 6,0x10\n\t"
                      "li 7,0x20\n\t"
                      "mr 9,%2\n\t"
                      "or 5,6,7\n\t"
                      "rlwinm 8,9,4,0,27\n\t"
                      "mr %0,5\n\t"
                      "mr %1,8"
                      : "=&r"(r5), "=&r"(r8)
                      : "r"(0x1122334455667788ull)
                      : "r5", "r6", "r7", "r8", "r9");
    put(19, r5);
    put(21, r8);

    __asm__ volatile ("mr 5,%2\n\t"
                      "li 6,0x10\n\t"
                      "li 7,0x20\n\t"
                      "li 8,0x88\n\t"
                      "b 1f\n\t"
                      "or 5,6,7\n"
                      "1:\trlwinm 8,5,4,0,27\n\t"
                      "mr %0,5\n\t"
                      "mr %1,8"
                      : "=&r"(r5), "=&r"(r8)
                      : "r"(0xDEADBEEFDEADBEEFull)
                      : "r5", "r6", "r7", "r8");
    put(23, r5);
    put(25, r8);

    write_frame();
    return 0;
}
