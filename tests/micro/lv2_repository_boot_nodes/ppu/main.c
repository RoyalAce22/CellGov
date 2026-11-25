/* PPU program: the repository values the running kernel read from lv1 at
 * boot, read back from the kernel's memory through the CFW's lv2 peek
 * (syscall 6).
 *
 * The stock 4.93 kernel reads its boot repository nodes once, in one
 * function, and stores each value into one static record in its zero-fill
 * data. The probe reads the words that locate the record and the words of
 * the record itself. It reads no kernel code, calls nothing in the kernel
 * and writes no kernel memory.
 *
 * Where each value sits, from the stock 4.93 kernel's code:
 *   TOC_WORD        the doubleword the kernel loads into r2 at start
 *   TOC_SLOT        r2 - 31808: the TOC slot that holds the record's address
 *   RECORD_POINTER  the pointer the kernel sets to the record's address
 *   RECORD + 0x08   the partition id (hypercall 74)
 *   RECORD + 0x10   lpar 2 bi.pun
 *   RECORD + 0x18   lpar 2 bi.pu (index 0)
 *   RECORD + 0x30   lpar 2 bi.pu.<id>.rm_addr
 *   RECORD + 0x38   lpar 2 bi.pu.<id>.rm_size
 *   RECORD + 0x68   lpar 2 bi.rgntotal
 *   RECORD + 0x70   the region state the kernel derives from the two sizes
 *   RECORD + 0x80   rm_size again
 *   RECORD + 0x90   bi.rgntotal - rm_size - (bi.rgntotal & 0xfffff)
 *   RECORD + 0xa0   bi.rgntotal & 0xfffff, when it is not 0
 *   RECORD + 0xa8   lpar 1 plat.id
 *   RECORD + 0xb0   lpar 1 be.0.clock
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

#define TOC_WORD       0x8000000000003000ULL
#define TOC_SLOT       0x8000000000347f70ULL
#define RECORD_POINTER 0x800000000037a590ULL
#define RECORD         0x800000000037a5a0ULL
#define RECORD_WORDS   24

/* The CFW's lv2 peek: r3 = address in, the 64-bit word there out. */
#define CFW_LV2_PEEK 6

#define READS (3 + RECORD_WORDS)

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
    read_at(2, RECORD_POINTER);
    for (word = 0; word < RECORD_WORDS; word++)
        read_at(3 + word, RECORD + 8ULL * word);

    write_frame(result, sizeof(result));
    return 0;
}
