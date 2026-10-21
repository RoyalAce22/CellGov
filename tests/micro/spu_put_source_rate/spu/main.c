/* SPU program for spu_put_source_rate: how soon after a 16 KB put issues
 * has the MFC read each part of its local-store source?
 *
 * Argument: arg1 (argp) = EA of a 320 KB block the PPU zeroed.
 *
 * Twenty cases, one after another: for each of the offsets 0x80, 0x200,
 * 0x800 and 0x2000, and each of 0, 4, 16, 64 and 256 spins. Case
 * 5 * o + s fills a 16 KB buffer with 0xC5, puts it to
 * block + 0x4000 * case, spins SPINS[s] times, stores 0x5C over the 16
 * bytes at OFFSETS[o], then waits for the put's tag.
 *
 * 0xC5 at the destination's offset means the MFC read the source there
 * before the store; 0x5C means after.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1
#define OFFSETS_N 4
#define SPINS_N 5
#define BIG 16384

/* Keeps the compiler from moving the buffer stores across the put. */
#define COMPILER_BARRIER() __asm__ volatile ("" ::: "memory")

static const unsigned int OFFSETS[OFFSETS_N] = { 0x80, 0x200, 0x800, 0x2000 };
static const unsigned int SPINS[SPINS_N] = { 0, 4, 16, 64, 256 };

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
    unsigned int o, s, k, c = 0;
    vector unsigned char after = spu_splats((unsigned char)0x5C);

    (void)spe_id;
    (void)envp;

    for (o = 0; o < OFFSETS_N; o++) {
        for (s = 0; s < SPINS_N; s++, c++) {
            vector unsigned char *at = (vector unsigned char *)(big + OFFSETS[o]);
            fill(big, BIG, 0xC5);
            COMPILER_BARRIER();
            mfc_put(big, argp + 0x4000 * c, BIG, TAG, 0, 0);
            COMPILER_BARRIER();
            for (k = 0; k < SPINS[s]; k++)
                spin++;
            *at = after;
            COMPILER_BARRIER();
            wait_tag();
        }
    }

    spu_thread_exit(0);
    return 0;
}
