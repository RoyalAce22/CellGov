/* SPU program: MFC queueing, list transfers, local-store arbitration and
 * EIB contention, read from the decrementer.
 *
 * Every SPU thread of a group runs this program. arg0 holds the
 * thread's index in its group in its high word and the group's number
 * of streaming threads, 0 to 5, in its low word; thread 0 measures and
 * the others stream.
 *
 * A streamer puts 1 to its start slot, then gets 16 KB from its own
 * source and the control line in turn until thread 0 puts 1 to the stop
 * slot, or 4 s of decrementer pass.
 *
 * Thread 0 waits up to 1 s for every streamer's start slot, then times
 * a single 16 KB get, sixteen 4 KB gets in one queue and a single 1 KB
 * get, and stores them with the wait in its contention block. In the
 * group with no streamer it first times the rest (layout in the PPU
 * program's header comment).
 *
 * arg1 = EA of the 384-byte result buffer.
 * arg2 = EA of the data region: 64 KB of transfer data, then a 16 KB
 *        source for each of streamers 1 to 5, then the 128-byte control
 *        line (stop slot at +0, streamer i's start slot at +16 * i).
 */

#include <spu_intrinsics.h>
#include <spu_mfcio.h>
#include <sys/spu_thread.h>

#define TAG 1
#define REPS 32
#define REPS_Q 8
#define LS_ITERS 128
#define DATA_BYTES 0x10000
#define SOURCE_BYTES 0x4000
#define CTRL_AT (DATA_BYTES + 6 * SOURCE_BYTES)
#define START_LIMIT 79800000u
#define STREAM_LIMIT (4u * 79800000u)

static unsigned char ls_big[DATA_BYTES] __attribute__((aligned(128)));
static unsigned char ls_buf[SOURCE_BYTES] __attribute__((aligned(128)));
static volatile vec_uint4 ls_scratch[64] __attribute__((aligned(128)));
static volatile unsigned int ls_ctrl[32] __attribute__((aligned(128)));
static volatile unsigned int ls_flag[4] __attribute__((aligned(16)));
static volatile mfc_list_element_t list[16] __attribute__((aligned(16)));

/* Words 16 to 71 of the result: thread 0 of the group with no streamer. */
static unsigned int quiet[56] __attribute__((aligned(16)));
/* One contention block: words 72 + 4k to 75 + 4k of the result. */
static unsigned int block[4] __attribute__((aligned(16)));

static const unsigned int COUNTS[5] = { 1, 2, 4, 8, 16 };

static inline unsigned int dec(void)
{
    return spu_readch(SPU_RdDec);
}

static inline void wait_tag(void)
{
    mfc_write_tag_mask(1 << TAG);
    mfc_read_tag_status_all();
}

/* `n` transfers of `size` bytes in one queue, then the tag wait; the sum
 * over `reps` runs. A 16 KB transfer uses ls_buf and the first 16 KB of
 * data each time; a smaller one uses its own slice of each. */
static unsigned int time_queue(int put, unsigned int data, unsigned int n,
                               unsigned int size, unsigned int reps)
{
    unsigned int sum = 0, t0, t1, i, r;
    for (r = 0; r < reps; r++) {
        t0 = dec();
        for (i = 0; i < n; i++) {
            unsigned int off = size >= SOURCE_BYTES ? 0 : i * size;
            void *ls = size >= SOURCE_BYTES ? (void *)ls_buf : (void *)(ls_big + off);
            if (put)
                mfc_put(ls, data + off, size, TAG, 0, 0);
            else
                mfc_get(ls, data + off, size, TAG, 0, 0);
        }
        wait_tag();
        t1 = dec();
        sum += t0 - t1;
    }
    return sum;
}

/* A list of `n` elements of `size` bytes each over the data region. */
static unsigned int time_list(int put, unsigned int data, unsigned int n,
                              unsigned int size)
{
    unsigned int sum = 0, t0, t1, i, r;
    for (i = 0; i < n; i++) {
        list[i].notify = 0;
        list[i].size = size;
        list[i].eal = data + i * size;
    }
    for (r = 0; r < REPS_Q; r++) {
        t0 = dec();
        if (put)
            mfc_putl(ls_big, data, (void *)list, n * 8, TAG, 0, 0);
        else
            mfc_getl(ls_big, data, (void *)list, n * 8, TAG, 0, 0);
        wait_tag();
        t1 = dec();
        sum += t0 - t1;
    }
    return sum;
}

#define R4(x) x x x x
#define R16(x) R4(x) R4(x) R4(x) R4(x)

/* LS_ITERS rounds of 64 quadword loads, or stores, from ls_scratch. */
static unsigned int __attribute__((noinline)) ls_loop(int store)
{
    unsigned int t0, t1, i;
    vec_uint4 v = spu_splats(0u);
    t0 = dec();
    for (i = 0; i < LS_ITERS; i++) {
        if (store) {
            R16(ls_scratch[0] = v; ls_scratch[1] = v; ls_scratch[2] = v; ls_scratch[3] = v;)
        } else {
            R16((void)ls_scratch[0]; (void)ls_scratch[1];
                (void)ls_scratch[2]; (void)ls_scratch[3];)
        }
    }
    t1 = dec();
    return t0 - t1;
}

/* The LS loop beside `n` queued transfers of `size` bytes: the loop's
 * sum in *loop and the sum to the end of the tag wait in *wait, both
 * from the loop's start. */
static void ls_beside(int store, int put, unsigned int data, unsigned int n,
                      unsigned int size, unsigned int *loop, unsigned int *wait)
{
    unsigned int r, i, t0, t1;
    *loop = 0;
    *wait = 0;
    for (r = 0; r < REPS_Q; r++) {
        for (i = 0; i < n; i++) {
            if (put)
                mfc_put(ls_big + i * size, data + i * size, size, TAG, 0, 0);
            else
                mfc_get(ls_big + i * size, data + i * size, size, TAG, 0, 0);
        }
        t0 = dec();
        *loop += ls_loop(store);
        wait_tag();
        t1 = dec();
        *wait += t0 - t1;
    }
}

static unsigned int ls_alone(int store)
{
    unsigned int r, sum = 0;
    for (r = 0; r < REPS_Q; r++)
        sum += ls_loop(store);
    return sum;
}

static unsigned int time_single(unsigned int data, unsigned int size)
{
    return time_queue(0, data, 1, size, REPS);
}

static void put_flag(unsigned int ea, unsigned int value)
{
    ls_flag[0] = value;
    mfc_put((void *)ls_flag, ea, 16, TAG, 0, 0);
    wait_tag();
}

static void stream(unsigned int data, unsigned int index)
{
    unsigned int ctrl = data + CTRL_AT;
    unsigned int src = data + DATA_BYTES + index * SOURCE_BYTES;
    unsigned int start = dec();
    put_flag(ctrl + 16 * index, 1);
    for (;;) {
        mfc_get(ls_buf, src, SOURCE_BYTES, TAG, 0, 0);
        mfc_get((void *)ls_ctrl, ctrl, 128, TAG, 0, 0);
        wait_tag();
        if (ls_ctrl[0] != 0 || start - dec() > STREAM_LIMIT)
            break;
    }
}

/* Wait for streamers 1 to `streamers`; the ticks it took, or all ones. */
static unsigned int await_streamers(unsigned int ctrl, unsigned int streamers)
{
    unsigned int start = dec(), i, all;
    for (;;) {
        mfc_get((void *)ls_ctrl, ctrl, 128, TAG, 0, 0);
        wait_tag();
        all = 1;
        for (i = 1; i <= streamers; i++)
            if (ls_ctrl[4 * i] == 0)
                all = 0;
        if (all)
            return start - dec();
        if (start - dec() > START_LIMIT)
            return 0xFFFFFFFFu;
    }
}

static void measure_quiet(unsigned int data)
{
    unsigned int i, r, t0, t1, sum = 0;
    quiet[0] = REPS;
    quiet[1] = REPS_Q;
    for (r = 0; r < REPS; r++) {
        t0 = dec();
        wait_tag();
        t1 = dec();
        sum += t0 - t1;
    }
    quiet[2] = sum;
    quiet[3] = 0;
    for (i = 0; i < 5; i++) {
        quiet[4 + i] = time_queue(0, data, COUNTS[i], 1024, REPS_Q);
        quiet[9 + i] = time_queue(0, data, COUNTS[i], SOURCE_BYTES, REPS_Q);
        quiet[14 + i] = time_queue(1, data, COUNTS[i], 1024, REPS_Q);
        quiet[19 + i] = time_queue(1, data, COUNTS[i], SOURCE_BYTES, REPS_Q);
        quiet[24 + i] = time_list(0, data, COUNTS[i], 128);
        quiet[29 + i] = time_list(0, data, COUNTS[i], 1024);
        quiet[34 + i] = time_list(0, data, COUNTS[i], 4096);
        quiet[39 + i] = time_list(1, data, COUNTS[i], 1024);
    }
    quiet[44] = ls_alone(0);
    ls_beside(0, 0, data, 16, 4096, &quiet[45], &quiet[46]);
    quiet[47] = ls_alone(1);
    ls_beside(1, 0, data, 16, 4096, &quiet[48], &quiet[49]);
    ls_beside(0, 1, data, 16, 4096, &quiet[50], &quiet[51]);
    ls_beside(0, 0, data, 16, 1024, &quiet[52], &quiet[53]);
    quiet[54] = 0;
    quiet[55] = 0;
}

int main(unsigned long long arg0, unsigned long long argp,
         unsigned long long envp, unsigned long long arg3)
{
    unsigned int result = (unsigned int)argp;
    unsigned int data = (unsigned int)envp;
    unsigned int index = (unsigned int)(arg0 >> 32);
    unsigned int streamers = (unsigned int)arg0;
    unsigned int ctrl = data + CTRL_AT;

    (void)arg3;
    spu_writech(SPU_WrDec, 0x7FFFFFFF);

    if (index != 0) {
        stream(data, index);
        spu_thread_exit(0);
        return 0;
    }

    if (streamers == 0) {
        measure_quiet(data);
        mfc_put(quiet, result + 16 * 4, sizeof(quiet), TAG, 0, 0);
        wait_tag();
    }

    block[3] = streamers ? await_streamers(ctrl, streamers) : 0;
    if (block[3] != 0xFFFFFFFFu) {
        block[0] = time_single(data, SOURCE_BYTES);
        block[1] = time_queue(0, data, 16, 4096, REPS_Q);
        block[2] = time_single(data, 1024);
    }
    put_flag(ctrl, 1);
    mfc_put(block, result + (72 + 4 * streamers) * 4, sizeof(block), TAG, 0, 0);
    wait_tag();

    spu_thread_exit(0);
    return 0;
}
