/* SPU program: DMA timing read from the decrementer.
 *
 * Each case runs REPS times. One run reads the decrementer, issues the
 * command, waits for it (tag group 1, or the atomic status), and reads
 * the decrementer again; the case's word is the sum of the REPS
 * differences. The decrementer counts down at the time-base frequency.
 *
 * arg1 = EA of the 256-byte result buffer; the program writes 224 bytes
 *        at +16 (layout in the PPU program's header comment).
 * arg2 = EA of a 128-byte-aligned data buffer of 16 KB + 256 bytes.
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1
#define REPS 32
#define SIZES 11
#define UNALIGNED 16
#define LINE_AT (16384 + 128)

static unsigned char ls_buf[16384 + 128] __attribute__((aligned(128)));
static unsigned char ls_line[128] __attribute__((aligned(128)));
static volatile mfc_list_element_t list[16] __attribute__((aligned(16)));

static struct {
    unsigned int reps;
    unsigned int overhead;
    unsigned int putllc_failures;
    unsigned int zero;
    unsigned int get_aligned[SIZES];
    unsigned int get_unaligned[SIZES];
    unsigned int put_aligned[SIZES];
    unsigned int put_unaligned[SIZES];
    unsigned int getl_16x1k;
    unsigned int putl_16x1k;
    unsigned int getl_4x4k;
    unsigned int getllar;
    unsigned int putllc;
    unsigned int putlluc;
    unsigned int pad[2];
} out __attribute__((aligned(128)));

static inline unsigned int dec(void)
{
    return spu_readch(SPU_RdDec);
}

static inline void wait_tag(void)
{
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();
}

static unsigned int time_transfer(int put, unsigned int lsa_off,
                                  unsigned int ea, unsigned int size)
{
    unsigned int sum = 0, t0, t1;
    int r;
    for (r = 0; r < REPS; r++) {
        t0 = dec();
        if (put)
            mfc_put(ls_buf + lsa_off, ea, size, TAG, 0, 0);
        else
            mfc_get(ls_buf + lsa_off, ea, size, TAG, 0, 0);
        wait_tag();
        t1 = dec();
        sum += t0 - t1;
    }
    return sum;
}

/* A list of `n` elements of `size` bytes each over the data buffer. */
static unsigned int time_list(int put, unsigned int ea, unsigned int n,
                              unsigned int size)
{
    unsigned int sum = 0, t0, t1, i;
    int r;
    for (i = 0; i < n; i++) {
        list[i].notify = 0;
        list[i].size = size;
        list[i].eal = ea + i * size;
    }
    for (r = 0; r < REPS; r++) {
        t0 = dec();
        if (put)
            mfc_putl(ls_buf, ea, (void *)list, n * 8, TAG, 0, 0);
        else
            mfc_getl(ls_buf, ea, (void *)list, n * 8, TAG, 0, 0);
        wait_tag();
        t1 = dec();
        sum += t0 - t1;
    }
    return sum;
}

int main(unsigned long long spe_id,
         unsigned long long argp,
         unsigned long long envp)
{
    unsigned int result = (unsigned int)argp;
    unsigned int data = (unsigned int)envp;
    unsigned int line = data + LINE_AT;
    unsigned int t0, t1, i;
    int r;

    spu_writech(SPU_WrDec, 0x7FFFFFFF);
    out.reps = REPS;

    for (r = 0; r < REPS; r++) {
        t0 = dec();
        wait_tag();
        t1 = dec();
        out.overhead += t0 - t1;
    }

    for (i = 0; i < SIZES; i++) {
        unsigned int size = 16u << i;
        out.get_aligned[i] = time_transfer(0, 0, data, size);
        out.get_unaligned[i] = time_transfer(0, UNALIGNED, data + UNALIGNED, size);
        out.put_aligned[i] = time_transfer(1, 0, data, size);
        out.put_unaligned[i] = time_transfer(1, UNALIGNED, data + UNALIGNED, size);
    }

    out.getl_16x1k = time_list(0, data, 16, 1024);
    out.putl_16x1k = time_list(1, data, 16, 1024);
    out.getl_4x4k = time_list(0, data, 4, 4096);

    for (r = 0; r < REPS; r++) {
        t0 = dec();
        mfc_getllar(ls_line, line, 0, 0);
        mfc_read_atomic_status();
        t1 = dec();
        out.getllar += t0 - t1;
    }
    for (r = 0; r < REPS; r++) {
        mfc_getllar(ls_line, line, 0, 0);
        mfc_read_atomic_status();
        t0 = dec();
        mfc_putllc(ls_line, line, 0, 0);
        if (mfc_read_atomic_status() & MFC_PUTLLC_STATUS)
            out.putllc_failures++;
        t1 = dec();
        out.putllc += t0 - t1;
    }
    for (r = 0; r < REPS; r++) {
        t0 = dec();
        mfc_putlluc(ls_line, line, 0, 0);
        mfc_read_atomic_status();
        t1 = dec();
        out.putlluc += t0 - t1;
    }

    mfc_put(&out, result + 16, sizeof(out), TAG, 0, 0);
    wait_tag();

    spu_thread_exit(0);
    return 0;
}
