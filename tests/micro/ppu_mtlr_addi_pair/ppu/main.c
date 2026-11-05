/* PPU probe: the LR and the 64-bit register that an "mtlr" followed by
 * an "addi" (or "li", which is addi with RA = 0) leave. One case is the
 * plain pair, one uses li, one has the addi overwrite the mtlr source,
 * one branches over the mtlr into the addi, one moves r0 to LR before
 * li, one wraps the add past the signed maximum, and one wraps the add
 * past all ones into its own base.
 *
 * The probe sends no system call; word 0 stays 0.
 *
 * A = 0x0123456789ABCDEF, B = 0xFEDCBA9876543210, C = 0x00000000CAFEF00D.
 *
 * Payload (big-endian u32 words; each register is high word then low):
 *   0  failed step (0 = none)   1  its return code
 *   2  r3 = A, r4 = B; mtlr 3 ; addi 5,4,-2: LR high     3  LR low
 *   4  the same case: r5 high                            5  r5 low
 *   6  r3 = A, r5 = B; mtlr 3 ; li 5,-1: LR high         7  LR low
 *   8  the same case: r5 high                            9  r5 low
 *  10  r3 = A; mtlr 3 ; addi 3,3,16: LR high            11  LR low
 *  12  the same case: r3 high                           13  r3 low
 *  14  r3 = A, r4 = B, LR = C; "b 1f ; mtlr 3 ; 1: addi 5,4,1": LR high
 *                                                       15  LR low
 *  16  the same case: r5 high                           17  r5 low
 *  18  r0 = A, r5 = B; mtlr 0 ; li 5,7: LR high         19  LR low
 *  20  the same case: r5 high                           21  r5 low
 *  22  r3 = A, r4 = 0x7FFFFFFFFFFFFFFF; mtlr 3 ; addi 5,4,1: LR high
 *                                                       23  LR low
 *  24  the same case: r5 high                           25  r5 low
 *  26  r3 = A, r4 = ~0; mtlr 3 ; addi 4,4,1: LR high    27  LR low
 *  28  the same case: r4 high                           29  r4 low
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define WORDS 30

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

int main(void)
{
    volatile unsigned long long va = 0x0123456789ABCDEFULL;
    volatile unsigned long long vb = 0xFEDCBA9876543210ULL;
    volatile unsigned long long vc = 0x00000000CAFEF00DULL;
    volatile unsigned long long vmax = 0x7FFFFFFFFFFFFFFFULL;
    volatile unsigned long long vones = 0xFFFFFFFFFFFFFFFFULL;
    unsigned long long a = va;
    unsigned long long b = vb;
    unsigned long long c = vc;
    unsigned long long max = vmax;
    unsigned long long ones = vones;
    unsigned long long lr;
    unsigned long long r;

    memset((void *)result, 0, sizeof(result));

    __asm__ volatile ("mr 3,%2\n\t"
                      "mr 4,%3\n\t"
                      "mtlr 3\n\t"
                      "addi 5,4,-2\n\t"
                      "mflr %0\n\t"
                      "mr %1,5"
                      : "=&r"(lr), "=&r"(r) : "r"(a), "r"(b)
                      : "r3", "r4", "r5", "lr");
    put(2, lr);
    put(4, r);

    __asm__ volatile ("mr 3,%2\n\t"
                      "mr 5,%3\n\t"
                      "mtlr 3\n\t"
                      "li 5,-1\n\t"
                      "mflr %0\n\t"
                      "mr %1,5"
                      : "=&r"(lr), "=&r"(r) : "r"(a), "r"(b)
                      : "r3", "r5", "lr");
    put(6, lr);
    put(8, r);

    __asm__ volatile ("mr 3,%2\n\t"
                      "mtlr 3\n\t"
                      "addi 3,3,16\n\t"
                      "mflr %0\n\t"
                      "mr %1,3"
                      : "=&r"(lr), "=&r"(r) : "r"(a)
                      : "r3", "lr");
    put(10, lr);
    put(12, r);

    __asm__ volatile ("mr 3,%2\n\t"
                      "mr 4,%3\n\t"
                      "mr 6,%4\n\t"
                      "mtlr 6\n\t"
                      "b 1f\n\t"
                      "mtlr 3\n"
                      "1:\taddi 5,4,1\n\t"
                      "mflr %0\n\t"
                      "mr %1,5"
                      : "=&r"(lr), "=&r"(r) : "r"(a), "r"(b), "r"(c)
                      : "r3", "r4", "r5", "r6", "lr");
    put(14, lr);
    put(16, r);

    __asm__ volatile ("mr 0,%2\n\t"
                      "mr 5,%3\n\t"
                      "mtlr 0\n\t"
                      "li 5,7\n\t"
                      "mflr %0\n\t"
                      "mr %1,5"
                      : "=&r"(lr), "=&r"(r) : "r"(a), "r"(b)
                      : "r0", "r5", "lr");
    put(18, lr);
    put(20, r);

    __asm__ volatile ("mr 3,%2\n\t"
                      "mr 4,%3\n\t"
                      "mtlr 3\n\t"
                      "addi 5,4,1\n\t"
                      "mflr %0\n\t"
                      "mr %1,5"
                      : "=&r"(lr), "=&r"(r) : "r"(a), "r"(max)
                      : "r3", "r4", "r5", "lr");
    put(22, lr);
    put(24, r);

    __asm__ volatile ("mr 3,%2\n\t"
                      "mr 4,%3\n\t"
                      "mtlr 3\n\t"
                      "addi 4,4,1\n\t"
                      "mflr %0\n\t"
                      "mr %1,4"
                      : "=&r"(lr), "=&r"(r) : "r"(a), "r"(ones)
                      : "r3", "r4", "lr");
    put(26, lr);
    put(28, r);

    write_frame();
    return 0;
}
