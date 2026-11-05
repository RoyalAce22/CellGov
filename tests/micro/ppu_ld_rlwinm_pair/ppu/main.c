/* PPU probe: what an adjacent "ld rT,d(r1) ; rlwinm rA,rS,SH,MB,ME" leaves
 * -- the loaded register, the rotate's target and, for the recording form,
 * CR. The cases are a rotate of the value the load just wrote, a wrapping
 * mask, a recording rotate with a negative 64-bit result, a recording
 * rotate with a zero result and XER[SO] = 1, the slwi, srwi and clrlwi
 * forms, a rotate into the load's own target, a rotate of another
 * register, and a branch into the rotate that runs it alone.
 *
 * Each case first stores its doubleword at -8(r1) and then loads it from
 * there. Each recording case starts with every CR field 0 and XER 0,
 * except where the payload names XER[SO] = 1.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   rotate the loaded value: v = 0x0102030405060708,
 *   "ld 6,-8(1) ; rlwinm. 7,6,8,0,23":
 *      2-3 r6    4-5 r7    6 CR
 *   wrapping mask: v = 0x0102030405060708, "ld 6,-8(1) ; rlwinm 7,6,8,24,7":
 *      7-8 r7
 *   negative result: v = 0x0000000080000001, "ld 6,-8(1) ; rlwinm. 7,6,0,1,0":
 *      9-10 r7  11 CR
 *   zero result, XER[SO] = 1: v = 0xABCDEF0000FFFFFF,
 *   "ld 6,-8(1) ; rlwinm. 7,6,0,0,7":
 *     12-13 r7  14 CR
 *   v = 0xFEDCBA9876543210 for each of:
 *     "ld 6,-8(1) ; slwi 7,6,3":   15-16 r7
 *     "ld 6,-8(1) ; srwi 7,6,3":   17-18 r7
 *     "ld 6,-8(1) ; clrlwi 7,6,3": 19-20 r7
 *   rotate into the load's target: v = 0x0123456789ABCDEF,
 *   "ld 6,-8(1) ; rlwinm 6,6,16,0,31":
 *     21-22 r6
 *   rotate another register: v = 0xFFFFFFFFFFFFFFFF, r5 = 0x1122334455667788,
 *   "ld 6,-8(1) ; rlwinm 7,5,4,0,27":
 *     23-24 r6  25-26 r7
 *   a branch into the rotate: v = 0xDEADBEEFDEADBEEF, r6 = 0x66, r7 = 0x77,
 *   "b 1f ; ld 6,-8(1) ; 1: rlwinm 7,6,4,0,27":
 *     27-28 r6  29-30 r7
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

#define CR_CLOBBERS "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", "cr6", "cr7"

/* "ld 6,-8(1) ; <second>" with v stored at -8(r1); r7 out. */
#define LD_THEN(second, v, out)                                         \
    __asm__ volatile ("std %1,-8(1)\n\t"                                \
                      "ld 6,-8(1)\n\t"                                  \
                      second "\n\t"                                     \
                      "mr %0,7"                                         \
                      : "=&r"(out)                                      \
                      : "r"(v)                                          \
                      : "r6", "r7", "memory")

int main(void)
{
    unsigned long long r6, r7, cr;
    unsigned long long zero = 0;

    memset((void *)result, 0, sizeof(result));

    __asm__ volatile ("mtcrf 255,%4\n\t"
                      "mtxer %4\n\t"
                      "std %3,-8(1)\n\t"
                      "ld 6,-8(1)\n\t"
                      "rlwinm. 7,6,8,0,23\n\t"
                      "mfcr %2\n\t"
                      "mr %0,6\n\t"
                      "mr %1,7"
                      : "=&r"(r6), "=&r"(r7), "=&r"(cr)
                      : "r"(0x0102030405060708ull), "r"(zero)
                      : "r6", "r7", "xer", "memory", CR_CLOBBERS);
    put(2, r6);
    put(4, r7);
    result[6] = (unsigned int)cr;

    LD_THEN("rlwinm 7,6,8,24,7", 0x0102030405060708ull, r7);
    put(7, r7);

    __asm__ volatile ("mtcrf 255,%3\n\t"
                      "mtxer %3\n\t"
                      "std %2,-8(1)\n\t"
                      "ld 6,-8(1)\n\t"
                      "rlwinm. 7,6,0,1,0\n\t"
                      "mfcr %1\n\t"
                      "mr %0,7"
                      : "=&r"(r7), "=&r"(cr)
                      : "r"(0x0000000080000001ull), "r"(zero)
                      : "r6", "r7", "xer", "memory", CR_CLOBBERS);
    put(9, r7);
    result[11] = (unsigned int)cr;

    __asm__ volatile ("mtcrf 255,%3\n\t"
                      "mtxer %4\n\t"
                      "std %2,-8(1)\n\t"
                      "ld 6,-8(1)\n\t"
                      "rlwinm. 7,6,0,0,7\n\t"
                      "mfcr %1\n\t"
                      "mtxer %3\n\t"
                      "mr %0,7"
                      : "=&r"(r7), "=&r"(cr)
                      : "r"(0xABCDEF0000FFFFFFull), "r"(zero), "r"(0x80000000ull)
                      : "r6", "r7", "xer", "memory", CR_CLOBBERS);
    put(12, r7);
    result[14] = (unsigned int)cr;

    LD_THEN("slwi 7,6,3", 0xFEDCBA9876543210ull, r7);
    put(15, r7);
    LD_THEN("srwi 7,6,3", 0xFEDCBA9876543210ull, r7);
    put(17, r7);
    LD_THEN("clrlwi 7,6,3", 0xFEDCBA9876543210ull, r7);
    put(19, r7);

    __asm__ volatile ("std %1,-8(1)\n\t"
                      "ld 6,-8(1)\n\t"
                      "rlwinm 6,6,16,0,31\n\t"
                      "mr %0,6"
                      : "=&r"(r6)
                      : "r"(0x0123456789ABCDEFull)
                      : "r6", "memory");
    put(21, r6);

    __asm__ volatile ("mr 5,%3\n\t"
                      "std %2,-8(1)\n\t"
                      "ld 6,-8(1)\n\t"
                      "rlwinm 7,5,4,0,27\n\t"
                      "mr %0,6\n\t"
                      "mr %1,7"
                      : "=&r"(r6), "=&r"(r7)
                      : "r"(0xFFFFFFFFFFFFFFFFull), "r"(0x1122334455667788ull)
                      : "r5", "r6", "r7", "memory");
    put(23, r6);
    put(25, r7);

    __asm__ volatile ("li 6,0x66\n\t"
                      "li 7,0x77\n\t"
                      "std %2,-8(1)\n\t"
                      "b 1f\n\t"
                      "ld 6,-8(1)\n"
                      "1:\trlwinm 7,6,4,0,27\n\t"
                      "mr %0,6\n\t"
                      "mr %1,7"
                      : "=&r"(r6), "=&r"(r7)
                      : "r"(0xDEADBEEFDEADBEEFull)
                      : "r6", "r7", "memory");
    put(27, r6);
    put(29, r7);

    write_frame();
    return 0;
}
