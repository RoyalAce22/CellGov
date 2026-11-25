/* PPU program: the interrupt state bitmaps that the running kernel
 * registered with lv1 at boot, read back from the kernel's per-thread
 * records through the CFW's lv2 peek (syscall 6).
 *
 * The stock 4.93 kernel sets up one 640-byte record per hardware thread
 * in the function that holds the sc at 0x8000000000007d78. It zeroes the
 * 64 bytes at +0x1C0 of the record (0x8000000000007c98 to
 * 0x8000000000007cc4), passes record + 0x1C0 to the function at
 * 0x8000000000077760 (0x8000000000007d3c, 0x8000000000007d48), and loads
 * the address that function returns into r5 of lv1
 * configure_irq_state_bitmap (hypercall 11, 0x8000000000007d74). r3 is
 * the record's +0x08 and r4 its +0x10.
 *
 * The probe reads the TOC word and the two TOC slots that locate the
 * records and the count, as lv2_logical_ppe_record does. It reads the
 * count, the thread word at +0x10 and the bitmap's eight doublewords
 * only when each locator holds its stock value, so it never peeks an
 * address that it did not read from the kernel. It reads no kernel
 * code, calls nothing in the kernel and writes no kernel memory.
 *
 * Result layout (16 * READS bytes, big-endian): per read, u64 the address,
 * then u64 the doubleword the peek returned. The reads are, in order:
 * TOC_WORD, RECORDS_SLOT, COUNT_SLOT, COUNT, then for record 0 and then
 * record 1: the word at +0x10 and the eight doublewords from +0x1C0. A
 * read the probe skipped holds address 0 and doubleword 0.
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
#define THREAD_WORD   0x10ULL
#define BITMAP        0x1c0ULL
#define BITMAP_WORDS  8

/* The CFW's lv2 peek: r3 = address in, the 64-bit word there out. */
#define CFW_LV2_PEEK 6

#define LOCATORS 3
#define READS (LOCATORS + 1 + RECORD_COUNT * (1 + BITMAP_WORDS))

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
    u64 base;
    int stock = 1;

    memset((void *)result, 0, sizeof(result));

    stock &= read_at(0, TOC_WORD) == TOC;
    stock &= read_at(1, RECORDS_SLOT) == RECORDS;
    stock &= read_at(2, COUNT_SLOT) == COUNT;

    if (stock) {
        read_at(LOCATORS, COUNT);
        for (record = 0; record < RECORD_COUNT; record++) {
            base = RECORDS + RECORD_SIZE * record;
            read_at(index++, base + THREAD_WORD);
            for (word = 0; word < BITMAP_WORDS; word++)
                read_at(index++, base + BITMAP + 8ULL * word);
        }
    }

    write_frame(result, sizeof(result));
    return 0;
}
