/* PPU program: whether the PPE keeps MSR[LE] = 1 when rfid restores an
 * SRR1 that holds it.
 *
 * The stock 4.93 kernel takes a system call at 0xC00, moves the caller's
 * SRR0 and SRR1 into r3 and r4, and goes by rfid to its handler at
 * 0x8000000000275a90. The handler finds the current thread through
 * SPRG0, the per-hardware-thread record: the doubleword at record + 0x68
 * points to the current thread, the thread holds its kernel stack base at
 * +0x50 and the stack size at +0x58, and the handler puts a frame 288
 * bytes below the stack top. It stores the caller's SRR0 at frame + 0x10
 * and the caller's SRR1 at frame + 0x18. The fast exit at
 * 0x8000000000275cf8 loads frame + 0x18, inserts MSR bits 38 and 50 from
 * the thread, moves it to SRR1 and returns by rfid; the slow exit copies
 * frame + 0x18 to the thread for the later return. So a write to
 * frame + 0x18 during a system call reaches the rfid that ends it.
 *
 * The probe:
 *   1. reads the locators that the lv2_logical_ppe_record probe reads and
 *      goes on only when each holds its stock value;
 *   2. for each hardware-thread record, follows record + 0x68 to the
 *      current thread and its frame, and takes the frame whose saved SRR0
 *      is the address after the sc of the peek that read it: that is its
 *      own frame;
 *   3. reads frame + 0x18 (S0, the MSR the probe runs with);
 *   4. during the CFW's lv2 poke (syscall 7), writes S0 | LE | PMM to
 *      frame + 0x18, so the poke's rfid restores MSR[LE] = 1 and
 *      MSR[PMM] = 1;
 *   5. reads frame + 0x18 (S1: the MSR at the next sc);
 *   6. during a second poke, writes S1 without LE and PMM, and reads
 *      frame + 0x18 again (S2).
 *
 * rfid copies MSR bits 60:63 from SRR1. PMM (bit 61) is a bit the PPE
 * keeps, so S1 holding PMM shows that the written SRR1 reached the MSR;
 * S1's LE is then the console's answer. The PPE runs big-endian only, so
 * the write to LE does not change how the probe runs.
 *
 * Every address the probe reads or writes is a kernel record, thread or
 * stack slot that the kernel itself stores to: a record or thread inside
 * the kernel's first 256 MB and outside its page-table window, or a
 * stack slot in the 32 MB from 0x8000000010000000 that holds the
 * kernel's 4 KB thread stacks.
 *
 * Result layout (16 doublewords, big-endian):
 *   +0x00 the first step that failed, 0 when none
 *   +0x08 TOC_WORD, +0x10 RECORDS_SLOT, +0x18 COUNT_SLOT as read
 *   +0x20 the record count
 *   +0x28 the index of the record that led to the probe's own frame,
 *         all ones when none did
 *   +0x30 the address after the peek's sc
 *   +0x38 the frame address
 *   +0x40 the SRR0 saved in the frame
 *   +0x48 S0
 *   +0x50 the doubleword the first poke wrote
 *   +0x58 the first poke's return value
 *   +0x60 S1
 *   +0x68 the doubleword the second poke wrote
 *   +0x70 the second poke's return value
 *   +0x78 S2
 *   +0x80 per record, four doublewords: the doubleword at record + 0x68,
 *         the thread it points to, the thread's stack base and its stack
 *         size; a link the probe did not read holds 0
 *
 * Steps: 1 a locator is not stock, 2 the record count is not 1 or 2,
 * 3 no record led to the probe's own frame, 4 S0 is not a problem-state
 * 64-bit MSR outside hypervisor state with LE and PMM 0, 5 the first poke
 * returned neither 0 nor its address, 6 the second poke did.
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

#define RECORD_CURRENT     0x68ULL
#define THREAD_STACK_BASE  0x50ULL
#define THREAD_STACK_SIZE  0x58ULL
#define FRAME_BELOW_TOP    288ULL
#define FRAME_SRR0         0x10ULL
#define FRAME_SRR1         0x18ULL

#define KERNEL_FIRST  0x8000000000000000ULL
#define KERNEL_END    0x8000000010000000ULL
#define PT_FIRST      0x800000000F000000ULL
#define PT_LAST       0x800000000F03FFFFULL
#define STACK_MAX     0x100000ULL
#define STACKS_FIRST  0x8000000010000000ULL
#define STACKS_END    0x8000000012000000ULL

#define MSR_SF   (1ULL << 63)
#define MSR_HV   (1ULL << 60)
#define MSR_PR   (1ULL << 14)
#define MSR_PMM  (1ULL << 2)
#define MSR_LE   (1ULL << 0)

/* The CFW's lv2 peek: r3 = address in, the doubleword there out. */
#define CFW_LV2_PEEK 6
/* The CFW's lv2 poke: r3 = address, r4 = doubleword. */
#define CFW_LV2_POKE 7

#define WORDS 24

enum {
    W_STEP, W_TOC, W_RECORDS, W_COUNT_SLOT, W_COUNT, W_RECORD, W_AFTER_SC,
    W_FRAME, W_SRR0, W_S0, W_POKE1, W_POKE1_RC, W_S1, W_POKE2, W_POKE2_RC,
    W_S2, W_LINKS
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile u64 result[WORDS] __attribute__((aligned(128)));

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

/* The one sc every peek goes through. *after_sc gets the address after
 * it, which the kernel saves as the caller's SRR0. */
static u64 __attribute__((noinline, noclone)) peek_at(u64 addr, u64 *after_sc)
{
    register u64 r3 __asm__("3") = addr;
    register u64 r11 __asm__("11") = CFW_LV2_PEEK;
    u64 here;
    __asm__ volatile (
        "bl 1f\n"
        "1: mflr %0\n"
        "sc\n"
        : "=&r"(here), "+r"(r3), "+r"(r11)
        :
        : "r0", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r12", "lr",
          "ctr", "xer", "cr0", "cr1", "cr5", "cr6", "cr7", "memory"
    );
    *after_sc = here + 8;
    return r3;
}

static int in_kernel(u64 addr)
{
    if (addr & 7)
        return 0;
    if (addr < KERNEL_FIRST || addr >= KERNEL_END - 8)
        return 0;
    if (addr >= PT_FIRST && addr <= PT_LAST)
        return 0;
    return 1;
}

/* The window that holds the kernel's thread stacks: 4 KB stacks from
 * 0x8000000010001000 up, which every system call entry writes. */
static int in_stacks(u64 addr)
{
    return (addr & 7) == 0 && addr >= STACKS_FIRST && addr < STACKS_END - 8;
}

static u64 peek(u64 addr)
{
    u64 after;
    return peek_at(addr, &after);
}

static u64 poke(u64 addr, u64 value)
{
    return (u64)cgov_sc(CFW_LV2_POKE, addr, value, 0, 0, 0, 0, 0, 0);
}

/* The CFW's poke leaves r3 as it came in, the address, on the reference
 * console; a refusal would return an error code. */
static int poke_done(u64 rc, u64 addr)
{
    return rc == 0 || rc == addr;
}

/* The frame record `index` leads to, 0 when a link leaves the kernel. */
static u64 frame_of(u64 index)
{
    volatile u64 *links = &result[W_LINKS + 4 * index];
    u64 current = peek(RECORDS + RECORD_SIZE * index + RECORD_CURRENT);
    u64 thread;
    u64 base;
    u64 size;
    links[0] = current;
    if (!in_kernel(current))
        return 0;
    thread = peek(current);
    links[1] = thread;
    if (!in_kernel(thread) || !in_kernel(thread + THREAD_STACK_SIZE))
        return 0;
    base = peek(thread + THREAD_STACK_BASE);
    size = peek(thread + THREAD_STACK_SIZE);
    links[2] = base;
    links[3] = size;
    if (size <= FRAME_BELOW_TOP || size > STACK_MAX)
        return 0;
    if (!in_stacks(base) || !in_stacks(base + size - 8))
        return 0;
    return base + size - FRAME_BELOW_TOP;
}

static int run(void)
{
    u64 count;
    u64 index;
    u64 frame = 0;
    u64 after = 0;
    u64 s0;
    u64 s1;

    result[W_TOC] = peek(TOC_WORD);
    result[W_RECORDS] = peek(RECORDS_SLOT);
    result[W_COUNT_SLOT] = peek(COUNT_SLOT);
    if (result[W_TOC] != TOC || result[W_RECORDS] != RECORDS ||
        result[W_COUNT_SLOT] != COUNT)
        return 1;

    count = peek(COUNT);
    result[W_COUNT] = count;
    if (count < 1 || count > 2)
        return 2;

    result[W_RECORD] = ~0ULL;
    for (index = 0; index < count; index++) {
        u64 candidate = frame_of(index);
        u64 srr0;
        if (candidate == 0)
            continue;
        srr0 = peek_at(candidate + FRAME_SRR0, &after);
        result[W_AFTER_SC] = after;
        if (srr0 == after) {
            frame = candidate;
            result[W_RECORD] = index;
            result[W_FRAME] = frame;
            result[W_SRR0] = srr0;
            break;
        }
    }
    if (frame == 0)
        return 3;

    s0 = peek(frame + FRAME_SRR1);
    result[W_S0] = s0;
    if (!(s0 & MSR_SF) || !(s0 & MSR_PR) || (s0 & MSR_HV) ||
        (s0 & (MSR_LE | MSR_PMM)))
        return 4;

    result[W_POKE1] = s0 | MSR_LE | MSR_PMM;
    result[W_POKE1_RC] = poke(frame + FRAME_SRR1, result[W_POKE1]);

    s1 = peek(frame + FRAME_SRR1);
    result[W_S1] = s1;

    result[W_POKE2] = s1 & ~(MSR_LE | MSR_PMM);
    result[W_POKE2_RC] = poke(frame + FRAME_SRR1, result[W_POKE2]);

    result[W_S2] = peek(frame + FRAME_SRR1);

    if (!poke_done(result[W_POKE1_RC], frame + FRAME_SRR1))
        return 5;
    if (!poke_done(result[W_POKE2_RC], frame + FRAME_SRR1))
        return 6;
    return 0;
}

int main(void)
{
    memset((void *)result, 0, sizeof(result));
    result[W_STEP] = (u64)run();
    write_frame(result, sizeof(result));
    return 0;
}
