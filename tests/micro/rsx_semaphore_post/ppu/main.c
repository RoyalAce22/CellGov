/* PPU program: writes an NV4097_GET_REPORT method to the FIFO,
 * advances put, polls the report-target label for a non-zero
 * timestamp, and exits.
 *
 * Exercises the SetReport path. Real PS3 libgcm's
 * cellGcmSetReport emits this method; the GPU writes a
 * timestamped report payload to the guest-specified offset.
 * CellGov's nv4097_get_report handler emits an RsxLabelWrite
 * carrying the current GuestTicks clock as the report value.
 *
 * Test name / TTY verdict string: this microtest lives in
 * `rsx_semaphore_post/` and reports `RSX_SEMAPHORE_POST: ...`
 * because Sony's libgcm groups SetReport alongside the
 * semaphore-release family under the "back-end semaphore post"
 * label (gcm_implementation_sub.h). The method exercised is
 * GET_REPORT / 0x1800 specifically; the directory name uses
 * the Sony-family name to match the public RSX surface.
 *
 * FIFO:
 *   [0] NV4097_GET_REPORT header (method 0x1800, count 1)
 *   [1] report argument (low 24 bits = absolute label address;
 *       upper 8 bits = report-type tag, unused by CellGov)
 *
 * Output (TTY payload = CGOV magic + 16 bytes):
 *   status         (u32)  0 = pass, 1 = report_value stayed 0
 *   report_value   (u32)  low 32 bits of the guest-ticks clock
 *                         CellGov wrote
 *   spin_iters     (u32)  iterations until the report became
 *                         non-zero
 *   _padding       (u32)  zero
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

static inline s32 syscall4_s32(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r6), "r"(r11)
        : "memory"
    );
    return (s32)r3;
}

static inline void syscall1_noreturn(u64 num, u64 a)
{
    register u64 r3 __asm__("3") = a;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        :
        : "r"(r3), "r"(r11)
        : "memory"
    );
}

static inline s32 syscall5_s32(u64 num, u64 a, u64 b, u64 c, u64 d, u64 e)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r7 __asm__("7") = e;
    register u64 r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3)
        : "r"(r4), "r"(r5), "r"(r6), "r"(r7), "r"(r11)
        : "memory"
    );
    return (s32)r3;
}

#define SYS_PPU_THREAD_EXIT 41
#define SYS_TTY_WRITE       403

/* Identity IO window over the first MiB of main memory, where these
 * statics live. cursor.get / cursor.put are RSX IO offsets, not
 * effective addresses; a guest reaching the FIFO through libgcm gets
 * its window from cellGcmInit. This test skips cellGcmInit, so
 * without this call the map stays empty and the FIFO drain refuses
 * every header. io / ea / size must be 1 MiB aligned --
 * sys_rsx_context_iomap returns CELL_EINVAL otherwise, and io + size
 * must stay inside the backed region, which rules out mapping the
 * image's own address as the io base. The window starts at io 0 and
 * covers the MiB holding this program's statics, so an EA converts
 * to an IO offset by subtracting RSX_IOMAP_EA. */
#define SYS_RSX_CONTEXT_IOMAP 672
#define RSX_CONTEXT_ID        0x55555555u
#define RSX_IOMAP_EA          0x10000000u
#define RSX_IOMAP_SIZE        0x00100000u

/* result.status bits for the two setup failures, so a bad window is
 * reported rather than showing up as an unexplained poll timeout. */
#define STATUS_IOMAP_FAILED        0x80u
#define STATUS_FIFO_OUTSIDE_WINDOW 0x40u

#define NV4097_GET_REPORT 0x1800
#define NV_COUNT_SHIFT    18

#define RSX_PUT_ADDR 0xC0000040u
#define RSX_GET_ADDR 0xC0000044u

struct TestResult {
    unsigned int status;
    unsigned int report_value;
    unsigned int spin_iters;
    unsigned int _padding;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int fifo_buffer[2] __attribute__((aligned(128)));
/* The report target. Aligned 16 to match the Sony libgcm label
 * stride so the report address is a conventional label slot. */
static volatile unsigned int report_slot __attribute__((aligned(16)));
static struct TestResult result __attribute__((aligned(128)));

static void write_tty_result(const struct TestResult *r)
{
    unsigned int len = sizeof(*r);
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)CGOV_MAGIC, 4,
                 (unsigned long)&written);
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)len_be, 4,
                 (unsigned long)&written);
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)r, len,
                 (unsigned long)&written);
}

/* Endian: the FIFO consumer reads command words big-endian --
 * RPCS3's FIFO_control does so via vm::read32, and CellGov's
 * rsx_advance via u32::from_be_bytes. The PPU is big-endian, so a
 * plain store already lays the bytes down the way both read them. */
static inline void fifo_store(volatile unsigned int *slot, unsigned int value)
{
    *slot = value;
}

int main(void)
{
    unsigned int fifo_base = (unsigned int)(unsigned long)&fifo_buffer[0];
    unsigned int fifo_end = fifo_base + 8;
    unsigned int report_addr = (unsigned int)(unsigned long)&report_slot;

    report_slot = 0;
    result.status = 0;
    result.report_value = 0;
    result.spin_iters = 0;
    result._padding = 0;

    if (syscall5_s32(SYS_RSX_CONTEXT_IOMAP, RSX_CONTEXT_ID, 0, RSX_IOMAP_EA,
                     RSX_IOMAP_SIZE, 0) != 0)
    {
        result.status |= STATUS_IOMAP_FAILED;
    }
    /* The window is fixed at compile time; if the linker ever moves
     * these statics outside it, say so rather than pushing an IO
     * offset that resolves somewhere else. */
    if (fifo_base < RSX_IOMAP_EA || fifo_base - RSX_IOMAP_EA >= RSX_IOMAP_SIZE)
    {
        result.status |= STATUS_FIFO_OUTSIDE_WINDOW;
    }

    /* Report descriptor: real PS3 libgcm packs offset (low 24
     * bits) and report-type tag (upper 8 bits). CellGov
     * treats the full 32 bits as an offset so microtests can
     * target statics at absolute addresses when label_base is
     * zero (no cellGcmInit). Pass the full report address
     * unmasked. */
    unsigned int report_arg = report_addr;

    unsigned int header_report = ((unsigned int)1 << NV_COUNT_SHIFT) | NV4097_GET_REPORT;
    fifo_store(&fifo_buffer[0], header_report);
    fifo_store(&fifo_buffer[1], report_arg);

    __asm__ volatile ("sync" ::: "memory");

    *((volatile unsigned int *)(unsigned long)RSX_GET_ADDR) = fifo_base - RSX_IOMAP_EA;
    *((volatile unsigned int *)(unsigned long)RSX_PUT_ADDR) = fifo_end - RSX_IOMAP_EA;

    unsigned int iters;
    for (iters = 0; iters < 100000u; iters++) {
        if (report_slot != 0) {
            break;
        }
    }
    result.spin_iters = iters;
    result.report_value = report_slot;
    if (report_slot == 0) {
        result.status |= 1;
    }

    const char *verdict;
    if (result.status == 0) {
        verdict = "RSX_SEMAPHORE_POST: PASS\n";
    } else {
        verdict = "RSX_SEMAPHORE_POST: FAIL\n";
    }
    unsigned int vlen = 0;
    while (verdict[vlen] != '\0') {
        vlen++;
    }
    unsigned int written;
    syscall4_s32(SYS_TTY_WRITE, 0, (unsigned long)verdict, vlen,
                 (unsigned long)&written);

    write_tty_result(&result);
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0xCAFEF00Du);
    for (;;) { }
}
