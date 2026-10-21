/* SPU program for spu_put_source_progress: does the MFC read a large
 * put's local-store source all at once, or front to back while the put
 * runs?
 *
 * Argument: arg1 (argp) = EA of a 64 KB block the PPU zeroed.
 *
 * Four cases, one after another. Case i fills a 16 KB buffer with 0xC0 + i,
 * puts it to block + 0x4000 * i, spins SPINS[i] times, stores 0x70 + i
 * over the buffer's first, middle and last 16 bytes, then waits for the
 * put's tag.
 *
 * At each of the three places, 0xC0 + i at the destination means the MFC
 * read the source there before the store; 0x70 + i means after.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1
#define CASES 4
#define BIG 16384

/* Keeps the compiler from moving the buffer stores across the put. */
#define COMPILER_BARRIER() __asm__ volatile ("" ::: "memory")

static const unsigned int SPINS[CASES] = { 32, 128, 512, 2048 };

static unsigned char big[BIG] __attribute__((aligned(128)));
static volatile unsigned int spin;

static void fill(unsigned char *p, unsigned int n, unsigned char v)
{
    unsigned int i;
    for (i = 0; i < n; i++)
        p[i] = v;
}

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
    unsigned char after;

    (void)spe_id;
    (void)envp;

    for (i = 0; i < CASES; i++) {
        fill(big, BIG, (unsigned char)(0xC0 + i));
        COMPILER_BARRIER();
        mfc_put(big, argp + 0x4000 * i, BIG, TAG, 0, 0);
        COMPILER_BARRIER();
        for (k = 0; k < SPINS[i]; k++)
            spin++;
        after = (unsigned char)(0x70 + i);
        fill(big, 16, after);
        fill(big + BIG / 2, 16, after);
        fill(big + BIG - 16, 16, after);
        COMPILER_BARRIER();
        wait_tag();
    }

    spu_thread_exit(0);
    return 0;
}
