/* PPU program: the virtual address space the running kernel got from lv1
 * at boot, read back from the kernel's memory through the CFW's lv2 peek
 * (syscall 6).
 *
 * The stock 4.93 kernel makes lv1 construct_virtual_address_space
 * (hypercall 2) once, in the function at 0x800000000005dc98, and stores
 * its outputs into one static record before it selects the address space
 * (hypercall 7). The function zeroes the record words +0x30, +0x38, +0x48
 * and +0x50 before the call, so a nonzero word there came from lv1. The
 * probe reads the words that locate the record and the words of the
 * record itself. It reads no kernel code, calls nothing in the kernel and
 * writes no kernel memory.
 *
 * Where each value sits, from the stock 4.93 kernel's code:
 *   TOC_WORD        the doubleword the kernel loads into r2 at start
 *   TOC_SLOT        r2 - 25760: the TOC slot that holds the record's address
 *   RECORD + 0x30   r4 out of hypercall 2: the vas id
 *   RECORD + 0x40   0x800000000f000000, which the function stores
 *   RECORD + 0x48   r5 out of hypercall 2: the hash table size
 *   RECORD + 0x50   r5 >> 7
 *
 * Result layout (16 * READS bytes, big-endian): per read, u64 the address,
 * then u64 the doubleword the peek returned.
 */

#include <string.h>

#include <ppu-types.h>
#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define TOC_WORD     0x8000000000003000ULL
#define TOC_SLOT     0x8000000000349710ULL
#define RECORD       0x80000000003e3a80ULL
#define RECORD_WORDS 12

/* The CFW's lv2 peek: r3 = address in, the 64-bit word there out. */
#define CFW_LV2_PEEK 6

#define READS (2 + RECORD_WORDS)

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile u64 result[2 * READS] __attribute__((aligned(128)));

static void write_frame(const volatile void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)data, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)data, len);
}

static u64 peek(u64 addr)
{
    return (u64)cgov_sc(CFW_LV2_PEEK, addr, 0, 0, 0, 0, 0, 0, 0);
}

static void read_at(unsigned int index, u64 addr)
{
    result[2 * index] = addr;
    result[2 * index + 1] = peek(addr);
}

int main(void)
{
    unsigned int word;

    read_at(0, TOC_WORD);
    read_at(1, TOC_SLOT);
    for (word = 0; word < RECORD_WORDS; word++)
        read_at(2 + word, RECORD + 8ULL * word);

    write_frame(result, sizeof(result));
    return 0;
}
