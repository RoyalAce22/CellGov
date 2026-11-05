/* PPU probe: the registers, CR and LR that adjacent "rldicr ; oris",
 * "or ; or", "or ; bc" and "or ; bctrl" leave, with each "or" raw or as
 * "mr" and the "rldicr" raw or as "sldi".
 *
 * rldicr ; oris: the oris reads the rldicr's result with the rldicr
 * recording; sldi then oris; the oris writes the rldicr's source; a
 * recorded zero result under XER[SO].
 * or ; or: chained with the second recording; mr then mr; both record
 * and the second wins; the second writes the first's source.
 * or ; bc: a recorded zero then beq; a recorded negative then blt; a
 * non-recording or then beq on the preset CR0; a recording or then beq
 * on the CR0 it wrote; a bdnz to its own address; a branch into the
 * second or of an or ; or.
 * or ; bctrl: CTR at a word boundary, and CTR with its low two bits set.
 *
 * Before each recording case CR0 is set to 0xF and XER to 0 (or to SO
 * alone in the XER[SO] case), so a half that does not record leaves CR0
 * at 0xF. The two beq cases on a preset CR set every CR field to EQ
 * (0x22222222). In the two bctrl cases T is the address of the label
 * after "li 7,1"; the probe records LR minus CTR as both read at T, so
 * the value does not depend on where the code loads.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * A = 0x0123456789ABCDEF, N = 0x8000000000000001, B = 0xF0F0F0F0FFFF8001,
 * C = 0x00000000CAFEF00D.
 *
 * Payload (big-endian u32 words; each register is high word then low;
 * each CR0 word is the 4-bit field LT GT EQ SO in its low bits; a CR
 * word is the whole 32-bit CR):
 *   0  failed step (0 = none)   1  its return code
 *   2  r4 = A; rldicr. 5,4,8,40 ; oris 6,5,0x8001: r5 (2-3), r6 (4-5), CR0 (6)
 *   7  r4 = A; sldi 5,4,4 ; oris 6,5,0x8001: r5 (7-8), r6 (9-10), CR0 (11)
 *  12  r4 = N; rldicr. 5,4,0,63 ; oris 4,4,0xffff: r5 (12-13), r4 (14-15), CR0 (16)
 *  17  r4 = A, XER = SO; rldicr. 5,4,0,0 ; oris 6,5,0: r5 (17-18), r6 (19-20), CR0 (21)
 *  22  r3 = A, r4 = B; or 5,4,3 ; or. 6,5,3: r5 (22-23), r6 (24-25), CR0 (26)
 *  27  r4 = B; mr 5,4 ; mr 6,5: r5 (27-28), r6 (29-30)
 *  31  r3 = A, r4 = N, r7 = 0; or. 5,4,3 ; or. 6,7,7: r5 (31-32), r6 (33-34), CR0 (35)
 *  36  r3 = A, r4 = B; or. 4,4,3 ; or 3,4,4: r4 (36-37), r3 (38-39), CR0 (40)
 *  41  r3 = r4 = 0, r7 = 0; "or. 5,4,3 ; beq 1f ; li 7,1 ; 1:": r7 (41-42), CR0 (43)
 *  44  r3 = A, r4 = N, r7 = 0; "or. 5,4,3 ; blt 1f ; li 7,1 ; 1:": r5 (44-45), r7 (46-47), CR0 (48)
 *  49  CR = 0x22222222, r3 = A, r4 = B, r7 = 0; "or 5,4,3 ; beq 1f ; li 7,1 ; 1:":
 *      r7 (49-50), CR (51)
 *  52  CR = 0x22222222, r3 = A, r4 = B, r7 = 0; "or. 5,4,3 ; beq 1f ; li 7,1 ; 1:":
 *      r7 (52-53), CR (54)
 *  55  r3 = A, r4 = B, CTR = 3; "or 5,4,3 ; 0: bdnz 0b": r5 (55-56), CTR (57-58)
 *  59  r3 = A, r4 = B, r5 = C; "b 1f ; or 5,4,3 ; 1: or 6,5,3": r5 (59-60), r6 (61-62)
 *  63  r3 = A, r4 = B, r7 = 0, CTR = T; "or. 5,4,3 ; bctrl ; li 7,1 ; T:":
 *      r5 (63-64), r7 (65-66), LR - CTR (67-68), CR0 (69)
 *  70  the same with CTR = T + 3: r7 (70-71), LR - CTR (72-73)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 74

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

#define CR_CLOBBERS "cr0", "cr1", "cr2", "cr3", "cr4", "cr5", "cr6", "cr7"

int main(void)
{
    volatile unsigned long long va = 0x0123456789ABCDEFULL;
    volatile unsigned long long vn = 0x8000000000000001ULL;
    volatile unsigned long long vb = 0xF0F0F0F0FFFF8001ULL;
    volatile unsigned long long vc = 0x00000000CAFEF00DULL;
    volatile unsigned long long vcr = 0xF0000000ULL;
    volatile unsigned long long veq = 0x22222222ULL;
    volatile unsigned long long vso = 0x80000000ULL;
    unsigned long long a = va;
    unsigned long long n = vn;
    unsigned long long b = vb;
    unsigned long long c = vc;
    unsigned long long crf = vcr;
    unsigned long long eq = veq;
    unsigned long long so = vso;
    unsigned long long zero = 0;
    unsigned long long r, s, t, cr;

    memset((void *)result, 0, sizeof(result));

    /* rldicr ; oris */

    __asm__ volatile ("mtcrf 0x80,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 4,%3\n\t"
                      "rldicr. 5,4,8,40\n\t"
                      "oris 6,5,0x8001\n\t"
                      "mfcr %2\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(crf), "r"(zero)
                      : "r4", "r5", "r6", "cr0", "xer");
    put(2, r);
    put(4, s);
    put_cr0(6, cr);

    __asm__ volatile ("mtcrf 0x80,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 4,%3\n\t"
                      "sldi 5,4,4\n\t"
                      "oris 6,5,0x8001\n\t"
                      "mfcr %2\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(crf), "r"(zero)
                      : "r4", "r5", "r6", "cr0", "xer");
    put(7, r);
    put(9, s);
    put_cr0(11, cr);

    __asm__ volatile ("mtcrf 0x80,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 4,%3\n\t"
                      "rldicr. 5,4,0,63\n\t"
                      "oris 4,4,0xffff\n\t"
                      "mfcr %2\n\t"
                      "mr %0,5\n\t"
                      "mr %1,4"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(n), "r"(crf), "r"(zero)
                      : "r4", "r5", "cr0", "xer");
    put(12, r);
    put(14, s);
    put_cr0(16, cr);

    __asm__ volatile ("mtcrf 0x80,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 4,%3\n\t"
                      "rldicr. 5,4,0,0\n\t"
                      "oris 6,5,0\n\t"
                      "mfcr %2\n\t"
                      "mtxer %6\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(crf), "r"(so), "r"(zero)
                      : "r4", "r5", "r6", "cr0", "xer");
    put(17, r);
    put(19, s);
    put_cr0(21, cr);

    /* or ; or */

    __asm__ volatile ("mtcrf 0x80,%5\n\t"
                      "mtxer %6\n\t"
                      "mr 3,%3\n\t"
                      "mr 4,%4\n\t"
                      "or 5,4,3\n\t"
                      "or. 6,5,3\n\t"
                      "mfcr %2\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(b), "r"(crf), "r"(zero)
                      : "r3", "r4", "r5", "r6", "cr0", "xer");
    put(22, r);
    put(24, s);
    put_cr0(26, cr);

    __asm__ volatile ("mr 4,%2\n\t"
                      "mr 5,4\n\t"
                      "mr 6,5\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(b)
                      : "r4", "r5", "r6");
    put(27, r);
    put(29, s);

    __asm__ volatile ("mtcrf 0x80,%5\n\t"
                      "mtxer %6\n\t"
                      "mr 3,%3\n\t"
                      "mr 4,%4\n\t"
                      "li 7,0\n\t"
                      "or. 5,4,3\n\t"
                      "or. 6,7,7\n\t"
                      "mfcr %2\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(n), "r"(crf), "r"(zero)
                      : "r3", "r4", "r5", "r6", "r7", "cr0", "xer");
    put(31, r);
    put(33, s);
    put_cr0(35, cr);

    __asm__ volatile ("mtcrf 0x80,%5\n\t"
                      "mtxer %6\n\t"
                      "mr 3,%3\n\t"
                      "mr 4,%4\n\t"
                      "or. 4,4,3\n\t"
                      "or 3,4,4\n\t"
                      "mfcr %2\n\t"
                      "mr %0,4\n\t"
                      "mr %1,3"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(b), "r"(crf), "r"(zero)
                      : "r3", "r4", "cr0", "xer");
    put(36, r);
    put(38, s);
    put_cr0(40, cr);

    /* or ; bc */

    __asm__ volatile ("mtcrf 0x80,%2\n\t"
                      "mtxer %3\n\t"
                      "li 3,0\n\t"
                      "li 4,0\n\t"
                      "li 7,0\n\t"
                      "or. 5,4,3\n\t"
                      "beq 0,1f\n\t"
                      "li 7,1\n"
                      "1:\tmfcr %1\n\t"
                      "mr %0,7"
                      : "=&r"(r), "=&r"(cr)
                      : "r"(crf), "r"(zero)
                      : "r3", "r4", "r5", "r7", "cr0", "xer");
    put(41, r);
    put_cr0(43, cr);

    __asm__ volatile ("mtcrf 0x80,%5\n\t"
                      "mtxer %6\n\t"
                      "mr 3,%3\n\t"
                      "mr 4,%4\n\t"
                      "li 7,0\n\t"
                      "or. 5,4,3\n\t"
                      "blt 0,1f\n\t"
                      "li 7,1\n"
                      "1:\tmfcr %2\n\t"
                      "mr %0,5\n\t"
                      "mr %1,7"
                      : "=&r"(r), "=&r"(s), "=&r"(cr)
                      : "r"(a), "r"(n), "r"(crf), "r"(zero)
                      : "r3", "r4", "r5", "r7", "cr0", "xer");
    put(44, r);
    put(46, s);
    put_cr0(48, cr);

    __asm__ volatile ("mtcrf 255,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 3,%2\n\t"
                      "mr 4,%3\n\t"
                      "li 7,0\n\t"
                      "or 5,4,3\n\t"
                      "beq 0,1f\n\t"
                      "li 7,1\n"
                      "1:\tmfcr %1\n\t"
                      "mr %0,7"
                      : "=&r"(r), "=&r"(cr)
                      : "r"(a), "r"(b), "r"(eq), "r"(zero)
                      : "r3", "r4", "r5", "r7", "xer", CR_CLOBBERS);
    put(49, r);
    result[51] = (unsigned int)cr;

    __asm__ volatile ("mtcrf 255,%4\n\t"
                      "mtxer %5\n\t"
                      "mr 3,%2\n\t"
                      "mr 4,%3\n\t"
                      "li 7,0\n\t"
                      "or. 5,4,3\n\t"
                      "beq 0,1f\n\t"
                      "li 7,1\n"
                      "1:\tmfcr %1\n\t"
                      "mr %0,7"
                      : "=&r"(r), "=&r"(cr)
                      : "r"(a), "r"(b), "r"(eq), "r"(zero)
                      : "r3", "r4", "r5", "r7", "xer", CR_CLOBBERS);
    put(52, r);
    result[54] = (unsigned int)cr;

    __asm__ volatile ("mr 3,%2\n\t"
                      "mr 4,%3\n\t"
                      "mtctr %4\n\t"
                      "or 5,4,3\n"
                      "0:\tbdnz 0b\n\t"
                      "mr %0,5\n\t"
                      "mfctr %1"
                      : "=&r"(r), "=&r"(t)
                      : "r"(a), "r"(b), "r"(3ull)
                      : "r3", "r4", "r5", "ctr");
    put(55, r);
    put(57, t);

    __asm__ volatile ("mr 3,%2\n\t"
                      "mr 4,%3\n\t"
                      "mr 5,%4\n\t"
                      "b 1f\n\t"
                      "or 5,4,3\n"
                      "1:\tor 6,5,3\n\t"
                      "mr %0,5\n\t"
                      "mr %1,6"
                      : "=&r"(r), "=&r"(s)
                      : "r"(a), "r"(b), "r"(c)
                      : "r3", "r4", "r5", "r6");
    put(59, r);
    put(61, s);

    /* or ; bctrl */

    __asm__ volatile ("mtcrf 0x80,%6\n\t"
                      "mtxer %7\n\t"
                      "mr 3,%4\n\t"
                      "mr 4,%5\n\t"
                      "li 7,0\n\t"
                      "bl 0f\n"
                      "0:\tmflr 9\n\t"
                      "addi 9,9,(1f-0b)\n\t"
                      "mtctr 9\n\t"
                      "or. 5,4,3\n\t"
                      "bctrl\n\t"
                      "li 7,1\n"
                      "1:\tmflr 8\n\t"
                      "mfctr 9\n\t"
                      "subf 8,9,8\n\t"
                      "mfcr %3\n\t"
                      "mr %0,5\n\t"
                      "mr %1,7\n\t"
                      "mr %2,8"
                      : "=&r"(r), "=&r"(s), "=&r"(t), "=&r"(cr)
                      : "r"(a), "r"(b), "r"(crf), "r"(zero)
                      : "r3", "r4", "r5", "r7", "r8", "r9", "lr", "ctr",
                        "cr0", "xer");
    put(63, r);
    put(65, s);
    put(67, t);
    put_cr0(69, cr);

    __asm__ volatile ("mr 3,%2\n\t"
                      "mr 4,%3\n\t"
                      "li 7,0\n\t"
                      "bl 0f\n"
                      "0:\tmflr 9\n\t"
                      "addi 9,9,(1f-0b)+3\n\t"
                      "mtctr 9\n\t"
                      "or 5,4,3\n\t"
                      "bctrl\n\t"
                      "li 7,1\n"
                      "1:\tmflr 8\n\t"
                      "mfctr 9\n\t"
                      "subf 8,9,8\n\t"
                      "mr %0,7\n\t"
                      "mr %1,8"
                      : "=&r"(r), "=&r"(t)
                      : "r"(a), "r"(b)
                      : "r3", "r4", "r5", "r7", "r8", "r9", "lr", "ctr");
    put(70, r);
    put(72, t);

    write_frame();
    return 0;
}
