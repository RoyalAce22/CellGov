/* PPU program: the logical PPE id the running kernel got from lv1 at
 * boot, read back from the kernel's per-thread records through the CFW's
 * lv2 peek (syscall 6).
 *
 * The stock 4.93 kernel sets up one 640-byte record per hardware thread
 * in the function that holds the sc at 0x8000000000007d2c. It takes the
 * record at RECORDS + 640 * n, where n is the count at COUNT, and it
 * increments the count; it stops at a count above 1. In each record it
 * stores n at +0x00, zeroes +0x48, +0x50 and +0x60, and calls lv1
 * get_logical_ppe_id (hypercall 69). On status 0 it stores r4 at +0x08,
 * and then cntlzw of CTRL at +0x10. Both words go to lv1
 * configure_irq_state_bitmap (hypercall 11) as r3 and r4.
 *
 * The probe reads the TOC word and the two TOC slots that locate the
 * records and the count. It reads the count and the records only when
 * each locator holds its stock value, so it never peeks an address that
 * it did not read from the kernel. It reads no kernel code, calls nothing
 * in the kernel and writes no kernel memory.
 *
 *   TOC_WORD      the doubleword the kernel loads into r2 at start
 *   RECORDS_SLOT  r2 - 32376: the TOC slot that holds RECORDS
 *   COUNT_SLOT    r2 - 32360: the TOC slot that holds COUNT
 *
 * Result layout (16 * READS bytes, big-endian): per read, u64 the address,
 * then u64 the doubleword the peek returned. The reads are, in order:
 * TOC_WORD, RECORDS_SLOT, COUNT_SLOT, COUNT, then words 0 to 12 of record
 * 0 and words 0 to 12 of record 1. A read the probe skipped holds address
 * 0 and doubleword 0.
 */

#include <string.h>

#include <ppu-types.h>
#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define TOC_WORD      0x8000000000003000ULL
#define TOC           0x800000000034fbb0ULL
#define RECORDS_SLOT  0x8000000000347d38ULL
#define RECORDS       0x8000000000368f80ULL
#define COUNT_SLOT    0x8000000000347d48ULL
#define COUNT         0x8000000000369488ULL
#define RECORD_SIZE   640ULL
#define RECORD_COUNT  2
#define RECORD_WORDS  13

/* The CFW's lv2 peek: r3 = address in, the 64-bit word there out. */
#define CFW_LV2_PEEK 6

#define LOCATORS 3
#define READS (LOCATORS + 1 + RECORD_COUNT * RECORD_WORDS)

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

static u64 read_at(unsigned int index, u64 addr)
{
    u64 value = peek(addr);
    result[2 * index] = addr;
    result[2 * index + 1] = value;
    return value;
}

int main(void)
{
    unsigned int record;
    unsigned int word;
    unsigned int index = LOCATORS + 1;
    int stock = 1;

    memset((void *)result, 0, sizeof(result));

    stock &= read_at(0, TOC_WORD) == TOC;
    stock &= read_at(1, RECORDS_SLOT) == RECORDS;
    stock &= read_at(2, COUNT_SLOT) == COUNT;

    if (stock) {
        read_at(LOCATORS, COUNT);
        for (record = 0; record < RECORD_COUNT; record++)
            for (word = 0; word < RECORD_WORDS; word++)
                read_at(index++,
                        RECORDS + RECORD_SIZE * record + 8ULL * word);
    }

    write_frame(result, sizeof(result));
    return 0;
}
