/* PPU program: writes an NV406E_SEMAPHORE_OFFSET + RELEASE pair
 * to a FIFO buffer, advances the RSX put pointer, polls a label
 * for the release value, and exits.
 *
 * Exercises the label-write path:
 *
 *   1. `cellgov_cli` runs this ELF with `[rsx] mirror = true`, so
 *      writes to 0xC000_0040 / 0xC000_0044 land in the
 *      Runtime's RSX cursor via the writeback mirror.
 *   2. The FIFO advance pass drains the buffer between
 *      `cursor.get` and `cursor.put`, decodes the two NV406E
 *      methods, and queues an RsxLabelWrite effect.
 *   3. The commit pipeline applies the queued effect in the
 *      NEXT batch, landing a 4-byte big-endian write at the
 *      label address.
 *   4. The spin loop here sees the written value, the program
 *      reports success via sysTtyWrite, and exits cleanly via
 *      sys_process_exit.
 *
 * Output layout (TTY payload = CGOV magic + 16 bytes):
 *   status         (u32)  0 = pass, non-zero bitfield on any
 *                         per-check failure
 *   label_value    (u32)  whatever the label actually held at
 *                         check time (expected = magic)
 *   expected       (u32)  the magic we emitted via RELEASE
 *   spin_iters     (u32)  how many poll iterations it took
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

/* Direct-syscall helpers. The RSX microtests avoid cellGcm HLE
 * bindings (PRX resolution on top of the HLE scaffolding is
 * still in progress) and drive the RSX cursor directly via the
 * writeback mirror, which is the path the
 * [rsx] mirror = true title manifest flag enables. */
static inline s32 syscall4_s32(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile(
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r6), "r"(r11)
        : "memory");
    return (s32)r3;
}

static inline void syscall1_noreturn(u64 num, u64 a)
{
    register u64 r3 __asm__("3") = a;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile(
        "sc\n"
        :
        : "r"(r3), "r"(r11)
        : "memory");
}

#define SYS_PPU_THREAD_EXIT 41
#define SYS_TTY_WRITE 403

/* RSX FIFO header shape: bits 2..=15 = method address (14 bits,
 * bottom two zero because methods are 4-byte aligned); bits
 * 18..=28 = argument count (11 bits). Low-bit flags distinguish
 * control-flow commands; a normal incrementing method has none
 * set. */
#define NV406E_SEMAPHORE_OFFSET 0x0064
#define NV406E_SEMAPHORE_RELEASE 0x006C
#define NV_COUNT_SHIFT 18

/* RSX control register MMIO addresses. Writing to these through
 * a volatile u32 store triggers the writeback mirror on
 * the oracle side. Real PS3 guests observe the same addresses
 * via cellGcmGetControlRegister; we hardcode them because the
 * layout is fixed and we are skipping cellGcmInit. */
#define RSX_PUT_ADDR 0xC0000040u
#define RSX_GET_ADDR 0xC0000044u

struct TestResult
{
    unsigned int status;
    unsigned int label_value;
    unsigned int expected;
    unsigned int spin_iters;
};

static const char CGOV_MAGIC[4] = {'C', 'G', 'O', 'V'};

/* FIFO buffer. Static, 128-byte aligned so it sits on its own
 * cache line and has no false-sharing artifacts. The oracle's
 * FIFO drain reads this memory as little-endian u32, so the
 * stores below pre-byte-swap. */
static volatile unsigned int fifo_buffer[4] __attribute__((aligned(128)));
/* Label target. 16-byte aligned (Sony libgcm label layout is
 * 16-byte stride). The oracle's commit pipeline writes a 4-byte
 * big-endian value here when the FIFO drain emits an
 * RsxLabelWrite. */
static volatile unsigned int label __attribute__((aligned(16)));
static struct TestResult result __attribute__((aligned(128)));

static void write_tty_result(const struct TestResult *r)
{
    unsigned int len = sizeof(*r);
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >> 8) & 0xFF;
    len_be[3] = (len) & 0xFF;
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)CGOV_MAGIC, 4,
                 (unsigned long)&written);
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)len_be, 4,
                 (unsigned long)&written);
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)r, len,
                 (unsigned long)&written);
}

/* Endian: PPU is big-endian, so a native u32 store writes BE
 * bytes. RSX FIFO memory is little-endian u32. To place a
 * specific u32 value in LE on disk, we store its byte-swap
 * (which the BE-store then un-reverses). */
static inline void fifo_store_le(volatile unsigned int *slot, unsigned int value)
{
    *slot = __builtin_bswap32(value);
}

int main(void)
{
    unsigned int fifo_base = (unsigned int)(unsigned long)&fifo_buffer[0];
    unsigned int fifo_end = fifo_base + 16;
    unsigned int label_addr = (unsigned int)(unsigned long)&label;
    const unsigned int magic = 0xCAFEBABEu;

    label = 0;
    result.status = 0;
    result.label_value = 0;
    result.expected = magic;
    result.spin_iters = 0;

    /* OFFSET header: method 0x64, count 1, no flags. */
    unsigned int header_offset = ((unsigned int)1 << NV_COUNT_SHIFT) | NV406E_SEMAPHORE_OFFSET;
    /* RELEASE header: method 0x6C, count 1, no flags. */
    unsigned int header_release = ((unsigned int)1 << NV_COUNT_SHIFT) | NV406E_SEMAPHORE_RELEASE;

    /* FIFO words (little-endian on disk):
     *   [0] OFFSET header
     *   [1] label address (absolute, because we skipped
     *       cellGcmInit so label_base = 0 on the oracle side)
     *   [2] RELEASE header
     *   [3] magic value to store at label_addr */
    fifo_store_le(&fifo_buffer[0], header_offset);
    fifo_store_le(&fifo_buffer[1], label_addr);
    fifo_store_le(&fifo_buffer[2], header_release);
    fifo_store_le(&fifo_buffer[3], magic);

    /* Memory barrier so every FIFO byte is visible before the
     * put-pointer advance. On the oracle this is redundant --
     * stores within one step commit atomically -- but matching
     * the real-PS3 pattern keeps the test portable to other
     * runners. */
    __asm__ volatile("sync" ::: "memory");

    /* Initialize cursor.get to the FIFO base. Under the
     * rsx_mirror path this store lands in memory AND is
     * mirrored into the runtime cursor. A guest running
     * through Sony's libgcm does not do this directly -- the
     * library's init seeds the FIFO state via cellGcmInit --
     * but we skip PRX / HLE bindings and poke the cursor
     * directly. */
    *((volatile unsigned int *)(unsigned long)RSX_GET_ADDR) = fifo_base;

    /* Now advance put. The next commit boundary's FIFO drain
     * parses the OFFSET + RELEASE pair and queues a
     * RsxLabelWrite. The batch after that applies it. */
    *((volatile unsigned int *)(unsigned long)RSX_PUT_ADDR) = fifo_end;

    /* Poll the label. The spin loop creates the commit
     * batches the pending RsxLabelWrite needs. Bounded so a
     * regression does not hang the test runner. */
    unsigned int iters;
    for (iters = 0; iters < 100000u; iters++)
    {
        if (label == magic)
        {
            break;
        }
    }
    result.spin_iters = iters;
    result.label_value = label;
    if (label != magic)
    {
        result.status = 1;
    }

    /* Emit a human-readable PASS / FAIL line before the binary
     * struct so the TTY capture surfaces the outcome even when
     * the struct bytes render as unprintable. */
    const char *verdict;
    if (result.status == 0)
    {
        verdict = "RSX_LABEL_WRITE_POLL: PASS\n";
    }
    else
    {
        verdict = "RSX_LABEL_WRITE_POLL: FAIL\n";
    }
    unsigned int verdict_len = 0;
    while (verdict[verdict_len] != '\0')
    {
        verdict_len++;
    }
    unsigned int tty_written;
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)verdict, verdict_len,
                 (unsigned long)&tty_written);

    write_tty_result(&result);
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0xCAFEF00Du);
    for (;;)
    {
    }
}
