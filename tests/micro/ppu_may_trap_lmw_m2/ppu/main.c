/* PPU probe: lmw r28 from an address two bytes past a word boundary.
 *
 * The probe fills a 32-byte window with 0x10, 0x11, ... and runs one
 * access at an offset into it. A progress record naming the step goes to
 * the result file before each step, so a run that ends at a step leaves
 * that record.
 *
 * Payload (big-endian u32 words):
 *   0  DONE ("ALGN") once the access ran, PROGRESS ("PROG") in a
 *      progress record
 *   1  the step a progress record names (0 otherwise): 1 the
 *      allocate (page probes only), 2 the access
 *   2  the low 32 bits of the access's effective address
 *   3  the allocate's return code (page probes only)
 *   4..7  what the access loaded: r28..r31 for lmw, the FPR's high and
 *      low words for lfd, the word for lwz
 *   8..15  the 32 bytes of the buffer around the access after it ran
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define KIND_LMW     1
#define PAGE         0
#define WINDOW_OFFSET 2

#define WORDS 16
#define DONE     0x414C474Eu /* "ALGN" */
#define PROGRESS 0x50524F47u /* "PROG" */

#define SYS_MEMORY_ALLOCATE 348
#define SYS_MEMORY_FREE     349
#define PAGE_SIZE_64K       0x200

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned char local[64] __attribute__((aligned(128)));
static volatile unsigned long long scratch __attribute__((aligned(8)));
static volatile unsigned int addr;

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

static void progress(unsigned int step)
{
    result[0] = PROGRESS;
    result[1] = step;
    write_file();
}

/* Fills the 32 bytes at `window` with 0x10, 0x11, ... */
static void fill(volatile unsigned char *window)
{
    unsigned int i;
    for (i = 0; i < 32; i++)
        window[i] = (unsigned char)(0x10 + i);
}

static void record_window(volatile unsigned char *window)
{
    unsigned int i;
    for (i = 0; i < 8; i++)
        result[8 + i] = ((unsigned int)window[4 * i] << 24) | ((unsigned int)window[4 * i + 1] << 16)
                      | ((unsigned int)window[4 * i + 2] << 8) | window[4 * i + 3];
}

int main(void)
{
    volatile unsigned char *window;
    unsigned long ea;

    memset((void *)result, 0, sizeof(result));
#if PAGE
    {
        s32 ret;
        progress(1);
        addr = 0;
        ret = syscall3_s32(SYS_MEMORY_ALLOCATE, 0x20000, PAGE_SIZE_64K,
                           (u64)(unsigned long)&addr);
        result[3] = (unsigned int)ret;
        if (ret != 0) {
            write_frame();
            return 0;
        }
        /* 16 bytes below the 64 KiB page boundary, 16 above. */
        window = (volatile unsigned char *)(unsigned long)(addr + 0x10000 - 16);
    }
#else
    window = local + 16;
#endif
    fill(window);
    ea = (unsigned long)window + WINDOW_OFFSET;
    result[2] = (unsigned int)ea;

    progress(2);
#if KIND_LMW
    {
        unsigned int r28, r29, r30, r31;
        __asm__ volatile(
            "lmw 28, 0(%4)\n\t"
            "mr %0, 28\n\t"
            "mr %1, 29\n\t"
            "mr %2, 30\n\t"
            "mr %3, 31\n\t"
            : "=&r"(r28), "=&r"(r29), "=&r"(r30), "=&r"(r31)
            : "b"(ea)
            : "r28", "r29", "r30", "r31", "memory");
        result[4] = r28;
        result[5] = r29;
        result[6] = r30;
        result[7] = r31;
    }
#elif KIND_STMW
    __asm__ volatile(
        "lis 28, 0xA0A1\n\tori 28, 28, 0xA2A3\n\t"
        "lis 29, 0xB0B1\n\tori 29, 29, 0xB2B3\n\t"
        "lis 30, 0xC0C1\n\tori 30, 30, 0xC2C3\n\t"
        "lis 31, 0xD0D1\n\tori 31, 31, 0xD2D3\n\t"
        "stmw 28, 0(%0)\n\t"
        :
        : "b"(ea)
        : "r28", "r29", "r30", "r31", "memory");
#elif KIND_LFD
    {
        unsigned long long bits;
        __asm__ volatile(
            "lfd 1, 0(%1)\n\t"
            "stfd 1, 0(%2)\n\t"
            "ld %0, 0(%2)\n\t"
            : "=&r"(bits)
            : "b"(ea), "b"((unsigned long)&scratch)
            : "fr1", "memory");
        result[4] = (unsigned int)(bits >> 32);
        result[5] = (unsigned int)bits;
    }
#elif KIND_STFD
    {
        static const unsigned long long pi = 0x400921FB54442D18ull;
        __asm__ volatile(
            "lfd 1, 0(%1)\n\t"
            "stfd 1, 0(%0)\n\t"
            :
            : "b"(ea), "b"((unsigned long)&pi)
            : "fr1", "memory");
    }
#elif KIND_LWZ
    {
        unsigned int word;
        __asm__ volatile("lwz %0, 0(%1)\n\t" : "=r"(word) : "b"(ea) : "memory");
        result[4] = word;
    }
#endif
    record_window(window);
#if PAGE
    (void)syscall1_s32(SYS_MEMORY_FREE, addr);
#endif

    result[0] = DONE;
    result[1] = 0;
    write_frame();
    return 0;
}
