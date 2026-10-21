/* SPU program for spu_put_source_timing: when does the MFC read a put's
 * local-store source, if the SPU stores over it after the put issues and
 * before it waits for the put's tag?
 *
 * Argument: arg1 (argp) = EA of a 32 KB block the PPU zeroed.
 *
 * Five small cases, one after another. Case i fills a 16-byte buffer with
 * 0xA0 + i, puts it to block + 0x10 * i, spins SPINS[i] times, fills the
 * buffer with 0x50 + i, then waits for the put's tag.
 *
 * One large case: a 16 KB buffer filled with 0xC7 is put to
 * block + 0x4000; at once the SPU fills its first and its last 16 bytes
 * with 0x7C, then waits.
 *
 * 0xA0 + i (0xC7) at the destination means the MFC read the source before
 * the store over it; 0x50 + i (0x7C) means after.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1
#define CASES 5
#define BIG 16384

static const unsigned int SPINS[CASES] = { 0, 1, 8, 64, 512 };

static unsigned char small[16] __attribute__((aligned(16)));
static unsigned char big[BIG] __attribute__((aligned(128)));
static volatile unsigned int spin;

static void fill(unsigned char *p, unsigned int n, unsigned char v)
{
    unsigned int i;
    for (i = 0; i < n; i++)
        p[i] = v;
}

/* Keeps the compiler from moving the buffer stores across the put. */
#define COMPILER_BARRIER() __asm__ volatile ("" ::: "memory")

static void wait_tag(void)
{
    mfc_write_tag_mask(1u << TAG);
    mfc_read_tag_status_all();
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int i, k;

    (void)spe_id;
    (void)envp;

    for (i = 0; i < CASES; i++) {
        fill(small, 16, (unsigned char)(0xA0 + i));
        COMPILER_BARRIER();
        mfc_put(small, argp + 0x10 * i, 16, TAG, 0, 0);
        COMPILER_BARRIER();
        for (k = 0; k < SPINS[i]; k++)
            spin++;
        fill(small, 16, (unsigned char)(0x50 + i));
        COMPILER_BARRIER();
        wait_tag();
    }

    fill(big, BIG, 0xC7);
    COMPILER_BARRIER();
    mfc_put(big, argp + 0x4000, BIG, TAG, 0, 0);
    COMPILER_BARRIER();
    fill(big, 16, 0x7C);
    fill(big + BIG - 16, 16, 0x7C);
    COMPILER_BARRIER();
    wait_tag();

    spu_thread_exit(0);
    return 0;
}
