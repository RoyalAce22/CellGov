/* PPU program: the port ids that lv1 construct_event_receive_port
 * (hypercall 18) gave the running 4.93 kernel, read back from the
 * kernel's memory through the CFW's lv2 peek (syscall 6).
 *
 * The kernel calls hypercall 18 four times on each hardware thread:
 * - once in the function that sets up the thread's 640-byte record; it
 *   stores r4 at +0x40 of the record;
 * - three times in the per-thread start, with n the thread index that
 *   it loads from +0x00 of the record through SPRG0:
 *   - call A stores r4 at TABLE_A + 48 * n + 8;
 *   - call B stores r4 at TABLE_B + 56 * n;
 *   - call C stores r4 at TABLE_C + 40 * n.
 * TABLE_A, TABLE_B and TABLE_C are the doublewords in the TOC slots at
 * r2 + 4416, r2 + 4464 and r2 + 4504.
 *
 * The probe reads the TOC word and the four TOC slots first. It reads
 * the records and the table entries only when each locator holds its
 * stock value, so it never peeks an address that it did not read from
 * the kernel. It reads no kernel code, calls nothing in the kernel and
 * writes no kernel memory.
 *
 * Result layout (16 * READS bytes, big-endian): per read, u64 the address,
 * then u64 the doubleword the peek returned. The reads are, in order:
 *   0  TOC_WORD
 *   1  RECORDS_SLOT (r2 - 32376)
 *   2  SLOT_A (r2 + 4416)
 *   3  SLOT_B (r2 + 4464)
 *   4  SLOT_C (r2 + 4504)
 *   5  record 0 +0x00, 6 record 0 +0x40
 *   7  record 1 +0x00, 8 record 1 +0x40
 *   9..14   words 0 to 5 of the TABLE_A entry of thread 0
 *   15..20  words 0 to 5 of the TABLE_A entry of thread 1
 *   21..27  words 0 to 6 of the TABLE_B entry of thread 0
 *   28..34  words 0 to 6 of the TABLE_B entry of thread 1
 *   35..39  words 0 to 4 of the TABLE_C entry of thread 0
 *   40..44  words 0 to 4 of the TABLE_C entry of thread 1
 * The port id of call A is word 1 of its entry; of calls B and C, word 0.
 * A read the probe skipped holds address 0 and doubleword 0.
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
#define RECORD_SIZE   640ULL
#define SLOT_A        0x8000000000350cf0ULL
#define TABLE_A       0x800000000037a668ULL
#define SLOT_B        0x8000000000350d20ULL
#define TABLE_B       0x80000000003dded0ULL
#define SLOT_C        0x8000000000350d48ULL
#define TABLE_C       0x80000000004067d0ULL

#define THREADS       2
#define A_WORDS       6
#define B_WORDS       7
#define C_WORDS       5

/* The CFW's lv2 peek: r3 = address in, the 64-bit word there out. */
#define CFW_LV2_PEEK 6

#define LOCATORS 5
#define READS (LOCATORS + 2 * THREADS + THREADS * (A_WORDS + B_WORDS + C_WORDS))

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

static unsigned int read_entries(unsigned int index, u64 table, u64 stride,
                                 unsigned int words)
{
    unsigned int thread;
    unsigned int word;
    for (thread = 0; thread < THREADS; thread++)
        for (word = 0; word < words; word++)
            read_at(index++, table + stride * thread + 8ULL * word);
    return index;
}

int main(void)
{
    unsigned int thread;
    unsigned int index = LOCATORS;
    int stock = 1;

    memset((void *)result, 0, sizeof(result));

    stock &= read_at(0, TOC_WORD) == TOC;
    stock &= read_at(1, RECORDS_SLOT) == RECORDS;
    stock &= read_at(2, SLOT_A) == TABLE_A;
    stock &= read_at(3, SLOT_B) == TABLE_B;
    stock &= read_at(4, SLOT_C) == TABLE_C;

    if (stock) {
        for (thread = 0; thread < THREADS; thread++) {
            read_at(index++, RECORDS + RECORD_SIZE * thread);
            read_at(index++, RECORDS + RECORD_SIZE * thread + 0x40ULL);
        }
        index = read_entries(index, TABLE_A, 48ULL, A_WORDS);
        index = read_entries(index, TABLE_B, 56ULL, B_WORDS);
        index = read_entries(index, TABLE_C, 40ULL, C_WORDS);
    }

    write_frame(result, sizeof(result));
    return 0;
}
