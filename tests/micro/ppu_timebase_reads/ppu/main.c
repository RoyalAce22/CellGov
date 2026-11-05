/* PPU probe: what the time-base reads return.
 *
 * Part A reads mftb 64 times, as eight groups of eight back-to-back
 * reads into registers; the stores sit between the groups. It runs
 * REPS_A times. Part B runs mftbu / mftb / mftbu triples, first away
 * from a carry, then across a carry out of the low word: the probe
 * sleeps until the low word is 0.1 s from carrying and runs triples
 * until 64 have run after it. Part C has a second thread: one thread
 * reads mftb and publishes the value, the other waits for it and reads
 * mftb, ROUNDS times each way.
 *
 * Payload: big-endian u64 words.
 *    0-62  part A: the 63 deltas of the last run
 *   63     part A: deltas of zero across every run (of REPS_A * 63)
 *   64     part A: the smallest delta across every run
 *   65     part A: the largest delta across every run
 *   66     REPS_A
 *   67     part B away from a carry: triples whose mftbu left a high word set (of 64)
 *   68     away: triples with equal mftbu reads whose mftb high word differs
 *   69     away: triples whose two mftbu reads differ
 *   70     sc 141 usleep rc before the carry
 *   71     across: triples run
 *   72     across: triples whose second mftbu is the first plus one
 *   73     across: inconsistent triples -- equal mftbu reads with a different
 *          mftb high word, mftbu reads more than one apart, or an mftb below
 *          the triple before
 *   74     across: 1 when the low word carried
 *   75     across: the last mftb before the carry
 *   76     across: the first mftb after it
 *   77     sc 52 thread create rc
 *   78     1 when the second thread ran
 *   79     part C, main publishes: rounds the second thread completed
 *   80     main publishes: rounds whose read was below the published value
 *   81     main publishes: the smallest read minus published value (s64)
 *   82     part C, second thread publishes: rounds main completed
 *   83     second publishes: rounds whose read was below the published value
 *   84     second publishes: the smallest read minus published value (s64)
 *   85     sc 44 join rc
 *   86     the second thread's exit value (expected 0x5442)
 *   87     mftb at main's entry
 *   88     the time-base frequency sc 147 returned
 *   89     0x50505442 ("PPTB"): the probe wrote every word
 */

#include <sys/process.h>
#include <sys/thread.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_PPU_THREAD_EXIT             41
#define SYS_PPU_THREAD_JOIN             44
#define SYS_TIMER_USLEEP                141
#define SYS_TIME_GET_TIMEBASE_FREQUENCY 147

#define WORDS   90
#define PAYLOAD (WORDS * 8)
#define REPS_A  16
#define TRIPLES 64
#define ROUNDS  1000

/* The most spins any wait runs before it gives up. */
#define SPIN_LIMIT (1u << 26)

static volatile unsigned char out[PAYLOAD] __attribute__((aligned(128)));
static u64 reads[64];

static volatile unsigned int sibling_started;
static volatile u64 pub1, pub2;
static volatile unsigned int seq1, ack1, seq2, ack2;
static volatile u64 rounds1, below1;
static volatile s64 margin1;

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

struct elfv1_opd {
    unsigned long long code;
    unsigned long long toc;
    unsigned long long env;
};

struct cg_thread_param {
    unsigned int entry_opd_ptr;
    unsigned int tls;
    unsigned int opd_code;
    unsigned int opd_toc;
};

static struct cg_thread_param sibling_param __attribute__((aligned(8)));

static unsigned long make_thread_param(struct cg_thread_param *p, const void *fn)
{
    const struct elfv1_opd *desc = (const struct elfv1_opd *)fn;
    unsigned long tls_reg;
    __asm__ volatile ("mr %0, 13" : "=r"(tls_reg));
    p->opd_code = (unsigned int)desc->code;
    p->opd_toc = (unsigned int)desc->toc;
    p->entry_opd_ptr = (unsigned int)(unsigned long)&p->opd_code;
    p->tls = (unsigned int)tls_reg;
    return (unsigned long)p;
}

static inline u64 timebase(void)
{
    u64 tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
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

/* Spin until *p == v. Returns 0 when SPIN_LIMIT spins pass first. */
static int wait_for(volatile unsigned int *p, unsigned int v)
{
    unsigned n;
    for (n = 0; n < SPIN_LIMIT; n++) {
        if (*p == v)
            return 1;
    }
    return 0;
}

/* Eight groups of eight back-to-back mftb reads into `reads`. */
static void __attribute__((noinline)) read64(void)
{
    unsigned g;
    for (g = 0; g < 8; g++) {
        u64 a, b, c, d, e, f, h, i;
        __asm__ volatile (
            "mftb %0\n" "mftb %1\n" "mftb %2\n" "mftb %3\n"
            "mftb %4\n" "mftb %5\n" "mftb %6\n" "mftb %7\n"
            : "=&r"(a), "=&r"(b), "=&r"(c), "=&r"(d),
              "=&r"(e), "=&r"(f), "=&r"(h), "=&r"(i));
        reads[8 * g + 0] = a;
        reads[8 * g + 1] = b;
        reads[8 * g + 2] = c;
        reads[8 * g + 3] = d;
        reads[8 * g + 4] = e;
        reads[8 * g + 5] = f;
        reads[8 * g + 6] = h;
        reads[8 * g + 7] = i;
    }
}

static inline void triple(u64 *u1, u64 *t, u64 *u2)
{
    __asm__ volatile ("mftbu %0\n" "mftb %1\n" "mftbu %2\n"
                      : "=&r"(*u1), "=&r"(*t), "=&r"(*u2));
}

/* A triple is consistent when its mftbu reads are equal and name the
 * mftb's high word, or differ by one. */
static int consistent(u64 u1, u64 t, u64 u2)
{
    if (u1 == u2)
        return (t >> 32) == u1;
    return u2 == u1 + 1;
}

/* The second thread: read what main publishes, then publish for main. */
static void sibling_entry(void *arg)
{
    unsigned i;
    u64 done = 0, below = 0;
    s64 least = 0x7FFFFFFFFFFFFFFFll;
    (void)arg;
    sibling_started = 1;
    for (i = 0; i < ROUNDS; i++) {
        u64 p, t;
        s64 m;
        if (!wait_for(&seq1, 2 * i + 1))
            break;
        __asm__ volatile ("lwsync" : : : "memory");
        p = pub1;
        t = timebase();
        m = (s64)(t - p);
        if (t < p)
            below++;
        if (m < least)
            least = m;
        done++;
        ack1 = 2 * i + 1;
    }
    rounds1 = done;
    below1 = below;
    margin1 = least;
    for (i = 0; i < ROUNDS; i++) {
        pub2 = timebase();
        __asm__ volatile ("lwsync" : : : "memory");
        seq2 = 2 * i + 1;
        if (!wait_for(&ack2, 2 * i + 1))
            break;
    }
    syscall1_noreturn(SYS_PPU_THREAD_EXIT, 0x5442);
    for (;;) { }
}

static void part_a(void)
{
    unsigned r, k;
    u64 zeros = 0, least = ~0ull, most = 0;
    for (r = 0; r < REPS_A; r++) {
        read64();
        for (k = 1; k < 64; k++) {
            u64 d = reads[k] - reads[k - 1];
            if (d == 0)
                zeros++;
            if (d < least)
                least = d;
            if (d > most)
                most = d;
            if (r == REPS_A - 1)
                put(k - 1, d);
        }
    }
    put(63, zeros);
    put(64, least);
    put(65, most);
    put(66, REPS_A);
}

static void part_b(u64 freq)
{
    unsigned k;
    u64 high = 0, mismatch = 0, differ = 0;
    u64 tb, rem, lead, end, prev = 0, before = 0, after = 0;
    u64 run = 0, plus_one = 0, bad = 0, carried = 0, past = 0;
    unsigned n;

    for (k = 0; k < TRIPLES; k++) {
        u64 u1, t, u2;
        triple(&u1, &t, &u2);
        if ((u1 >> 32) != 0 || (u2 >> 32) != 0)
            high++;
        if (u1 == u2 && (t >> 32) != u1)
            mismatch++;
        if (u1 != u2)
            differ++;
    }
    put(67, high);
    put(68, mismatch);
    put(69, differ);

    tb = timebase();
    rem = 0x100000000ull - (tb & 0xFFFFFFFFull);
    lead = freq / 10;
    if (rem > lead)
        put(70, (u64)(s64)syscall1_s32(SYS_TIMER_USLEEP, (rem - lead) * 1000000ull / freq));
    end = tb + rem + freq;
    for (n = 0; n < SPIN_LIMIT; n++) {
        u64 u1, t, u2;
        triple(&u1, &t, &u2);
        run++;
        if (u2 == u1 + 1)
            plus_one++;
        if (!consistent(u1, t, u2) || (prev != 0 && t < prev))
            bad++;
        if (!carried && prev != 0 && (t & 0xFFFFFFFFull) < (prev & 0xFFFFFFFFull)) {
            carried = 1;
            before = prev;
            after = t;
        }
        prev = t;
        if (carried && ++past >= TRIPLES)
            break;
        if (t > end)
            break;
    }
    put(71, run);
    put(72, plus_one);
    put(73, bad);
    put(74, carried);
    put(75, before);
    put(76, after);
}

static void part_c(void)
{
    unsigned i;
    u64 done = 0, below = 0;
    s64 least = 0x7FFFFFFFFFFFFFFFll;
    for (i = 0; i < ROUNDS; i++) {
        pub1 = timebase();
        __asm__ volatile ("lwsync" : : : "memory");
        seq1 = 2 * i + 1;
        if (!wait_for(&ack1, 2 * i + 1))
            break;
    }
    for (i = 0; i < ROUNDS; i++) {
        u64 p, t;
        s64 m;
        if (!wait_for(&seq2, 2 * i + 1))
            break;
        __asm__ volatile ("lwsync" : : : "memory");
        p = pub2;
        t = timebase();
        m = (s64)(t - p);
        if (t < p)
            below++;
        if (m < least)
            least = m;
        done++;
        ack2 = 2 * i + 1;
    }
    put(82, done);
    put(83, below);
    put(84, (u64)least);
}

int main(void)
{
    unsigned i;
    u64 entry = timebase();
    u64 freq;
    unsigned long long tid = 0, retval = 0;
    s32 rc;

    for (i = 0; i < PAYLOAD; i++)
        out[i] = 0;
    put(87, entry);
    freq = (u64)syscall0_s32(SYS_TIME_GET_TIMEBASE_FREQUENCY);
    put(88, freq);

    part_a();
    part_b(freq);

    rc = cgov_ppu_thread_create(
        &tid,
        make_thread_param(&sibling_param, (const void *)&sibling_entry),
        0, 0, 2000, 0x4000, THREAD_JOINABLE, 0);
    put(77, (u64)(s64)rc);
    if (rc == 0) {
        if (wait_for(&sibling_started, 1)) {
            put(78, 1);
            part_c();
        }
        put(85, (u64)(s64)syscall2_s32(SYS_PPU_THREAD_JOIN, tid, (unsigned long)&retval));
        put(86, retval);
        put(79, rounds1);
        put(80, below1);
        put(81, (u64)margin1);
    }
    put(89, 0x50505442ull);

    write_frame();
    return 0;
}
