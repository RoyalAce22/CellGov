/* PPU probe: does LV2 mask a 32-bit argument whose register carries a high
 * word, or refuse it? Asked of a u32 pointer (sc 48's priop) and an int
 * (sc 47's prio) on the calling thread. Every priority sent is 3000 in its
 * low word, a low priority, and each set is undone with the original.
 *
 * Payload: big-endian u64 words, one per slot.
 *   0  sys_ppu_thread_get_id rc          1  the thread id
 *   2  sc 48(tid, &p) rc                 3  p, the original priority
 *   4  sc 48(tid, &p | 1 << 32) rc       5  p (pre-filled 0xEEEEEEEE)
 *   6  sc 47(tid, 0x1_0000_0BB8) rc      7  priority read back
 *   8  sc 47(tid, original) rc
 *   9  sc 47(tid, 0xFFFF_FFFF_0000_0BB8) rc  10  priority read back
 *  11  sc 47(tid, original) rc
 *  12  sc 47(tid, 0xBBB) rc (control)    13  priority read back
 *  14  sc 47(tid, original) rc           15  priority read back
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_SET_PRIORITY 47
#define SYS_PPU_THREAD_GET_PRIORITY 48

#define WORDS   16
#define PAYLOAD (WORDS * 8)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static volatile int prio_word __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b)
{
    return (u64)cgov_sc(num, a, b, 0, 0, 0, 0, 0, 0);
}

static void put(unsigned slot, u64 v)
{
    unsigned i;
    for (i = 0; i < 8; i++)
        out[slot * 8 + i] = (unsigned char)(v >> (56 - 8 * i));
}

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = PAYLOAD;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)out, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)out, len);
}

static u64 read_prio(u64 tid)
{
    prio_word = (int)0xEEEEEEEE;
    sc(SYS_PPU_THREAD_GET_PRIORITY, tid, EA(&prio_word));
    return (u64)(unsigned int)prio_word;
}

int main(void)
{
    unsigned i;
    sys_ppu_thread_t tid = 0;
    u64 original;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;

    put(0, (u64)(s64)sysThreadGetId(&tid));
    put(1, tid);

    prio_word = (int)0xEEEEEEEE;
    put(2, sc(SYS_PPU_THREAD_GET_PRIORITY, tid, EA(&prio_word)));
    original = (u64)(unsigned int)prio_word;
    put(3, original);

    prio_word = (int)0xEEEEEEEE;
    put(4, sc(SYS_PPU_THREAD_GET_PRIORITY, tid, EA(&prio_word) | (1ull << 32)));
    put(5, (u64)(unsigned int)prio_word);

    put(6, sc(SYS_PPU_THREAD_SET_PRIORITY, tid, 0x100000BB8ull));
    put(7, read_prio(tid));
    put(8, sc(SYS_PPU_THREAD_SET_PRIORITY, tid, original));

    put(9, sc(SYS_PPU_THREAD_SET_PRIORITY, tid, 0xFFFFFFFF00000BB8ull));
    put(10, read_prio(tid));
    put(11, sc(SYS_PPU_THREAD_SET_PRIORITY, tid, original));

    put(12, sc(SYS_PPU_THREAD_SET_PRIORITY, tid, 0xBBB));
    put(13, read_prio(tid));
    put(14, sc(SYS_PPU_THREAD_SET_PRIORITY, tid, original));
    put(15, read_prio(tid));

    write_frame();
    return 0;
}
