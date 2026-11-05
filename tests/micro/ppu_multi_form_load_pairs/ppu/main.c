/* PPU probe: the registers and CR0 that adjacent "ld rT,d(r1) ; addi",
 * "lwz ; or" and "or ; lwz" leave, with the "addi" raw or as "li" and
 * each "or" raw or as "mr".
 *
 * ld ; addi: the addi reads the ld's result; li with r0 nonzero; the
 * addi reads r1; the addi overwrites the ld's target; the ld writes r0
 * and the addi names RA = 0; a branch into the addi.
 * lwz ; or: the or reads the loaded word and records; mr of the loaded
 * word; the lwz overwrites its own base; a branch into the or.
 * or ; lwz: the lwz base is the or's result; the or records and the lwz
 * reads another base; the or reads and writes the lwz base (or 7,7,4);
 * the lwz writes the or's source (mr 6,5 ; lwz 5,d(6)).
 *
 * The ld cases first store D0 at -16(r1) and D1 at -8(r1), inside the
 * protected area below the stack pointer, and never move r1.
 *
 * Before each recording case CR0 is set to 0xF and XER to 0.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * buf holds three doublewords: D0 = 0x0123456789ABCDEF,
 * D1 = 0x8000000000000001, D2 = 0xFEDCBA9876543210. H is buf with its low
 * 16 bits cleared and L is those 16 bits, so H | L is buf.
 * A = 0x0123456789ABCDEF, N = 0x8000000000000001, B = 0xF0F0F0F0FFFF8001,
 * C = 0x00000000CAFEF00D.
 *
 * Payload (big-endian u32 words; each register is high word then low;
 * each CR0 word is the 4-bit field LT GT EQ SO in its low bits):
 *   0  failed step (0 = none)   1  its return code
 *   2  ld 5,-16(1) ; addi 6,5,-4: r5 (2-3), r6 (4-5)
 *   6  r0 = 0x100; ld 5,-8(1) ; li 6,-4: r5 (6-7), r6 (8-9)
 *  10  ld 5,-16(1) ; addi 6,1,16: r6 - r1 (10-11)
 *  12  ld 5,-16(1) ; addi 5,5,-4: r5 (12-13)
 *  14  ld 0,-8(1) ; addi 6,0,1: r0 (14-15), r6 (16-17)
 *  18  r7 = buf, r4 = B; lwz 5,0(7) ; or. 6,5,4: r5 (18-19), r6 (20-21), CR0 (22)
 *  23  r7 = buf; lwz 5,4(7) ; mr 6,5: r5 (23-24), r6 (25-26)
 *  27  r7 = buf, r4 = C; lwz 7,8(7) ; or 6,7,4: r7 (27-28), r6 (29-30)
 *  31  r8 = H, r9 = L; or 5,8,9 ; lwz 6,4(5): r5 - buf (31-32), r6 (33-34)
 *  35  r3 = A, r4 = N, r7 = buf; or. 5,4,3 ; lwz 6,8(7): r5 (35-36), r6 (37-38), CR0 (39)
 *  40  r7 = H, r4 = L; or 7,7,4 ; lwz 6,12(7): r7 - buf (40-41), r6 (42-43)
 *  44  r5 = buf; mr 6,5 ; lwz 5,16(6): r6 - buf (44-45), r5 (46-47)
 *  48  r7 = buf, r4 = A, r5 = C; "b 1f ; lwz 5,0(7) ; 1: or 6,5,4": r5 (48-49), r6 (50-51)
 *  52  r5 = C; "b 1f ; ld 5,-16(1) ; 1: addi 6,5,1": r5 (52-53), r6 (54-55)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 56

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));

static volatile unsigned long long buf[3] __attribute__((aligned(16))) = {
    0x0123456789ABCDEFULL,
    0x8000000000000001ULL,
    0xFEDCBA9876543210ULL,
};

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
    volatile unsigned long long vcr = 0xF0000000ULL;
    unsigned long long a = va;
    unsigned long long n = vn;
    unsigned long long b = vb;
    unsigned long long c = vc;
    unsigned long long crf = vcr;
    unsigned long long d0 = buf[0];
    unsigned long long d1 = buf[1];
    unsigned long long base = (unsigned long long)(unsigned long)buf;
    unsigned long long hi = base & ~0xFFFFULL;
    unsigned long long lo = base & 0xFFFFULL;
    unsigned long long zero = 0;
    unsigned long long r, s, cr;

    memset((void *)result, 0, sizeof(result));

    /* ld rT,d(r1) ; addi */

    __asm__ volatile ("std %2,-16(1)\n\t"
                      "std %3,-8(1)\n\t"
                      "ld 5,-16(1)\n\t"
                      "addi 6,5,-4\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(d0), "r"(d1)
                      : "r5", "r6", "memory");
    put(2, r);
    put(4, s);

    __asm__ volatile ("std %2,-16(1)\n\t"
                      "std %3,-8(1)\n\t"
                      "li 0,0x100\n\t"
                      "ld 5,-8(1)\n\t"
                      "li 6,-4\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(d0), "r"(d1)
                      : "r0", "r5", "r6", "memory");
    put(6, r);
    put(8, s);

    __asm__ volatile ("std %1,-16(1)\n\t"
                      "std %2,-8(1)\n\t"
                      "ld 5,-16(1)\n\t"
                      "addi 6,1,16\n\t"
                      "subf %0,1,6"
                      : "=&r"(r)
                      : "r"(d0), "r"(d1)
                      : "r5", "r6", "memory");
    put(10, r);

    __asm__ volatile ("std %1,-16(1)\n\t"
                      "std %2,-8(1)\n\t"
                      "ld 5,-16(1)\n\t"
                      "addi 5,5,-4\n\t"
                      "mr %0,5"
                      : "=&r"(r)
                      : "r"(d0), "r"(d1)
                      : "r5", "memory");
    put(12, r);

    __asm__ volatile ("std %2,-16(1)\n\t"
                      "std %3,-8(1)\n\t"
                      "ld 0,-8(1)\n\t"
                      "addi 6,0,1\n\t"
                      "mr %0,0\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(d0), "r"(d1)
                      : "r0", "r6", "memory");
    put(14, r);
    put(16, s);

    /* lwz ; or */

    __asm__ volatile ("mtcrf 0x80,%5\n\t"
                      "mtxer %6\n\t"
                      "mr 7,%3\n\t"
                      "mr 4,%4\n\t"
                      "lwz 5,0(7)\n\t"
                      "or. 6,5,4\n\t"
                      "mfcr %2\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(base), "r"(b), "r"(crf), "r"(zero)
                      : "r4", "r5", "r6", "r7", "cr0", "xer", "memory");
    put(18, r);
    put(20, s);
    put_cr0(22, cr);

    __asm__ volatile ("mr 7,%2\n\t"
                      "lwz 5,4(7)\n\t"
                      "mr 6,5\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(base)
                      : "r5", "r6", "r7", "memory");
    put(23, r);
    put(25, s);

    __asm__ volatile ("mr 7,%2\n\t"
                      "mr 4,%3\n\t"
                      "lwz 7,8(7)\n\t"
                      "or 6,7,4\n\t"
                      "mr %0,7\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(base), "r"(c)
                      : "r4", "r6", "r7", "memory");
    put(27, r);
    put(29, s);

    /* or ; lwz */

    __asm__ volatile ("mr 8,%2\n\t"
                      "mr 9,%3\n\t"
                      "or 5,8,9\n\t"
                      "lwz 6,4(5)\n\t"
                      "subf %0,%4,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(hi), "r"(lo), "r"(base)
                      : "r5", "r6", "r8", "r9", "memory");
    put(31, r);
    put(33, s);

    __asm__ volatile ("mtcrf 0x80,%6\n\t"
                      "mtxer %7\n\t"
                      "mr 3,%3\n\t"
                      "mr 4,%4\n\t"
                      "mr 7,%5\n\t"
                      "or. 5,4,3\n\t"
                      "lwz 6,8(7)\n\t"
                      "mfcr %2\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(n), "r"(base), "r"(crf), "r"(zero)
                      : "r3", "r4", "r5", "r6", "r7", "cr0", "xer", "memory");
    put(35, r);
    put(37, s);
    put_cr0(39, cr);

    __asm__ volatile ("mr 7,%2\n\t"
                      "mr 4,%3\n\t"
                      "or 7,7,4\n\t"
                      "lwz 6,12(7)\n\t"
                      "subf %0,%4,7\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(hi), "r"(lo), "r"(base)
                      : "r4", "r6", "r7", "memory");
    put(40, r);
    put(42, s);

    __asm__ volatile ("mr 5,%2\n\t"
                      "mr 6,5\n\t"
                      "lwz 5,16(6)\n\t"
                      "subf %0,%2,6\n\t"
                      "mr %1,5"
                      : "=&r"(r), "=&r"(s)
                      : "r"(base)
                      : "r5", "r6", "memory");
    put(44, r);
    put(46, s);

    /* a branch into the second half */

    __asm__ volatile ("mr 7,%2\n\t"
                      "mr 4,%3\n\t"
                      "mr 5,%4\n\t"
                      "b 1f\n\t"
                      "lwz 5,0(7)\n"
                      "1:\tor 6,5,4\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(base), "r"(a), "r"(c)
                      : "r4", "r5", "r6", "r7", "memory");
    put(48, r);
    put(50, s);

    __asm__ volatile ("std %3,-16(1)\n\t"
                      "std %4,-8(1)\n\t"
                      "mr 5,%2\n\t"
                      "b 1f\n\t"
                      "ld 5,-16(1)\n"
                      "1:\taddi 6,5,1\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(c), "r"(d0), "r"(d1)
                      : "r5", "r6", "memory");
    put(52, r);
    put(54, s);

    write_frame();
    return 0;
}
