/* PPU program: the running kernel's syscall dispatch table, read through
 * the CFW's lv2 peek (syscall 6).
 *
 * The 4.93 kernel's table is TABLE_SLOTS 8-byte entries from TABLE_BASE;
 * each entry points at a function descriptor whose first word is the
 * handler's code address. For every slot the probe reads the entry and,
 * when the entry lies inside the kernel image, the descriptor's code
 * word. It calls nothing through the table and writes no kernel memory.
 *
 * Result layout (16 + 16 * TABLE_SLOTS bytes, big-endian):
 *   +0:  u64 the peek of TABLE_BASE - 8 (the word before the table),
 *        u64 the number of entries that lay outside the kernel image
 *   +16: per slot, u64 the entry, u64 its descriptor's code word (0 when
 *        the entry lies outside the kernel image)
 */

#include <string.h>

#include <ppu-types.h>
#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define TABLE_BASE  0x8000000000363be0ULL
#define TABLE_SLOTS 1024
#define KERNEL_LO   0x8000000000000000ULL
#define KERNEL_HI   0x8000000000800000ULL

/* The CFW's lv2 peek: r3 = address in, the 64-bit word there out. */
#define CFW_LV2_PEEK 6

#define RESULT_WORDS (2 + 2 * TABLE_SLOTS)

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile u64 result[RESULT_WORDS] __attribute__((aligned(128)));

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

int main(void)
{
    u64 outside = 0;
    unsigned int slot;

    result[0] = peek(TABLE_BASE - 8);
    for (slot = 0; slot < TABLE_SLOTS; slot++) {
        u64 entry = peek(TABLE_BASE + 8ULL * slot);
        u64 code = 0;
        if (entry >= KERNEL_LO && entry < KERNEL_HI)
            code = peek(entry);
        else
            outside++;
        result[2 + 2 * slot] = entry;
        result[3 + 2 * slot] = code;
    }
    result[1] = outside;

    write_frame(result, sizeof(result));
    return 0;
}
