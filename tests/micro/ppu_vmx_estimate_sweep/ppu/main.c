/* PPU program: compare a model of vrefp, vrsqrtefp, vexptefp and
 * vlogefp, read off ppu_vmx_estimate_readout and ppu_vmx_estimate_edges,
 * with the instructions themselves, and report only the mismatches.
 *
 * Sweeps, in frame order, eight words each:
 *   0-3  vrefp, vrsqrtefp, vexptefp, vlogefp under VSCR[NJ] = 0, every
 *        one of the 2^32 inputs
 *   4-7  the same four under NJ = 1, every input whose biased exponent is
 *        0, or from 250 to 255
 *   8    control: vrefp under NJ = 0 over biased exponent 127 of both
 *        signs, with table entry 5's base one higher; it must count
 *        exactly the 2 x 2^18 inputs of that segment
 *
 * Each sweep writes: mismatches (u64), the first mismatching input, the
 * instruction's and the model's result there, inputs swept (u64), 0.
 */

#include <altivec.h>
#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SWEEPS 9
#define WORDS_PER_SWEEP 8
#define BLOCK 1024

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_frame(const void *payload, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, payload, len, &written);
    CGOV_OUT_FILE_WRITE(payload, len);
}

static unsigned int RECIP_B[32] = {
    0x7ff800, 0x783800, 0x70ea00, 0x6a0800, 0x638800, 0x5d6200, 0x579000, 0x520800,
    0x4cc800, 0x47ca00, 0x430800, 0x3e8000, 0x3a2c00, 0x360800, 0x321400, 0x2e4a00,
    0x2aa800, 0x272c00, 0x23d600, 0x209e00, 0x1d8800, 0x1a9000, 0x17ae00, 0x14f800,
    0x124400, 0x0fbe00, 0x0d3800, 0x0ade00, 0x088400, 0x065000, 0x041c00, 0x020c00,
};
static const unsigned int RECIP_S[32] = {
    3972, 3740, 3524, 3328, 3148, 2984, 2832, 2688, 2556, 2436, 2324, 2216, 2120, 2028, 1940, 1860,
    1784, 1712, 1644, 1580, 1520, 1464, 1388, 1388, 1292, 1292, 1204, 1204, 1128, 1128, 1056, 1048,
};
static const unsigned int RSQRT_ODD_B[16] = {
    0x7ff400, 0x785200, 0x715400, 0x6ae400, 0x64f200, 0x5f6e00, 0x5a4c00, 0x558000,
    0x510200, 0x4cca00, 0x48d000, 0x450e00, 0x418200, 0x3e2400, 0x3af200, 0x37e800,
};
static const unsigned int RSQRT_ODD_S[16] = {
    1956, 1792, 1648, 1522, 1412, 1316, 1228, 1150, 1082, 1018, 962, 910, 862, 818, 778, 742,
};
static const unsigned int RSQRT_EVEN_B[16] = {
    0x34fd00, 0x2f9700, 0x2aa500, 0x261800, 0x21e400, 0x1dfe00, 0x1a5c00, 0x16f800,
    0x13ca00, 0x10ce00, 0x0dfe00, 0x0b5700, 0x08d400, 0x067300, 0x043100, 0x020b00,
};
static const unsigned int RSQRT_EVEN_S[16] = {
    1384, 1267, 1165, 1077, 999, 930, 869, 814, 764, 720, 680, 643, 609, 579, 550, 523,
};
/* base, shift, slope, offset per 128 log-table entries */
static const unsigned int LOG_SEG[16][4] = {
    {0, 1, 3, 0}, {192, 2, 5, 0}, {352, 2, 5, 0}, {512, 3, 9, 0},
    {656, 3, 9, 0}, {800, 3, 9, 0}, {944, 0, 1, 0}, {1072, 0, 1, 0},
    {1200, 0, 1, 0}, {1327, 3, 7, 7}, {1439, 3, 7, 7}, {1551, 3, 7, 7},
    {1663, 2, 3, 3}, {1759, 2, 3, 3}, {1855, 2, 3, 3}, {1951, 2, 3, 3},
};

static int is_nan(unsigned int x)
{
    return (x & 0x7F800000u) == 0x7F800000u && (x & 0x7FFFFFu) != 0;
}

/* The zero a NJ = 1 read gives: a true zero, or a denormal under NJ. */
static int reads_zero(unsigned int x, int nj)
{
    return (x & 0x7FFFFFFFu) == 0 || (nj && (x & 0x7F800000u) == 0);
}

/* Unbiased exponent and 23-bit fraction of a nonzero finite x. */
static void normalise(unsigned int x, int *e, unsigned int *m)
{
    unsigned int be = x >> 23 & 0xFF;
    unsigned int f = x & 0x7FFFFF;
    if (be) {
        *e = (int)be - 127;
        *m = f;
    } else {
        int lead = 31 - __builtin_clz(f);
        *e = lead - 149;
        *m = (f << (23 - lead)) & 0x7FFFFF;
    }
}

/* (1.frac) x 2^exp with sign; an exponent below -126 denormalises with
 * round to nearest even, or gives a zero under NJ = 1. */
static unsigned int pack(unsigned int sign, int exp, unsigned int frac, int nj)
{
    int b = exp + 127;
    unsigned int full, kept, rem, half;
    int shift;
    if (b >= 255)
        return sign << 31 | 0x7F800000u;
    if (b >= 1)
        return sign << 31 | (unsigned int)b << 23 | frac;
    if (nj)
        return sign << 31;
    shift = 1 - b;
    if (shift > 24)
        return sign << 31;
    full = frac | 0x800000u;
    kept = full >> shift;
    rem = full & ((1u << shift) - 1);
    half = 1u << (shift - 1);
    if (rem > half || (rem == half && (kept & 1)))
        kept++;
    return sign << 31 | kept;
}

static unsigned int m_vrefp(unsigned int x, int nj)
{
    unsigned int s = x >> 31, m;
    int e;
    if (is_nan(x))
        return x | 0x400000u;
    if (reads_zero(x, nj))
        return s << 31 | 0x7F800000u;
    if ((x & 0x7FFFFFFFu) == 0x7F800000u)
        return s << 31;
    normalise(x, &e, &m);
    if (e == 126 && m == 0)
        return s << 31 | 0x00800000u;
    return pack(s, -e - 1, RECIP_B[m >> 18] - RECIP_S[m >> 18] * ((m & 0x3FFFF) >> 9) / 4, nj);
}

static unsigned int rsqrt_fraction(unsigned int m, int odd)
{
    unsigned int b = odd ? RSQRT_ODD_B[m >> 19] : RSQRT_EVEN_B[m >> 19];
    unsigned int s = odd ? RSQRT_ODD_S[m >> 19] : RSQRT_EVEN_S[m >> 19];
    unsigned int v4 = 4 * b - s * ((m & 0x7FFFF) >> 9);
    unsigned int q = v4 >> 2, r = v4 & 3;
    if (r > 2 || (r == 2 && (q & 1)))
        q++;
    return q;
}

static unsigned int m_vrsqrtefp(unsigned int x, int nj)
{
    unsigned int m;
    int e;
    if (is_nan(x))
        return x | 0x400000u;
    if (reads_zero(x, nj))
        return (x & 0x80000000u) | 0x7F800000u;
    if (x >> 31)
        return 0x7FC00000u;
    if (x == 0x7F800000u)
        return 0;
    normalise(x, &e, &m);
    if ((e & 1) == 0)
        return pack(0, -(e / 2) - 1, rsqrt_fraction(m, 1), nj);
    return pack(0, -((e - 1) >> 1) - 1, rsqrt_fraction(m, 0), nj);
}

static unsigned int exp_table(unsigned int f)
{
    if (f < 8192)
        return (f >> 4) - (f >> 6);
    if (f < 14336)
        return (f >> 4) - (f >> 7) - 64;
    if (f < 20480)
        return (f >> 4) - 176;
    if (f < 26624)
        return (f >> 4) + (f >> 7) - 335;
    if (f < 30720)
        return (f >> 4) + (f >> 6) - 543;
    return (f >> 4) + (f >> 5) - 1023;
}

static unsigned int m_vexptefp(unsigned int x, int nj)
{
    unsigned int m, mag;
    int e, shift, n;
    long long q;
    if (is_nan(x))
        return x | 0x400000u;
    if (x == 0xFF800000u)
        return 0;
    if (x == 0x7F800000u)
        return x;
    if (reads_zero(x, nj))
        return 0x3F800000u;
    /* Every x below -126 gives +0, even one that truncates to -126. */
    if ((x >> 31) && (x & 0x7FFFFFFFu) > 0x42FC0000u)
        return 0;
    normalise(x, &e, &m);
    mag = m | 0x800000u;
    shift = e + 15 - 23;
    if (shift >= 8)
        q = 1LL << 30;
    else if (shift >= 0)
        q = (long long)mag << shift;
    else if (shift > -32)
        q = mag >> -shift;
    else
        q = 0;
    if (q >= (1LL << 30))
        return (x >> 31) ? 0 : 0x7F800000u;
    if (x >> 31)
        q = -q;
    n = (int)(q >> 15);
    if (n < -126)
        return 0;
    return pack(0, n, exp_table((unsigned int)(q & 0x7FFF)) << 12, nj);
}

static unsigned int m_vlogefp(unsigned int x, int nj)
{
    unsigned int m, i, a, frac;
    const unsigned int *seg;
    int e, v, lead;
    if (is_nan(x))
        return x | 0x400000u;
    if (reads_zero(x, nj))
        return 0xFF800000u;
    if (x >> 31)
        return 0x7FC00000u;
    if (x == 0x7F800000u)
        return x;
    normalise(x, &e, &m);
    i = m >> 12;
    seg = LOG_SEG[i >> 7];
    v = e * 2048 + (int)(seg[0] + ((seg[2] * (i & 127) + seg[3]) >> seg[1]));
    if (v == 0)
        return 0;
    a = v < 0 ? (unsigned int)-v : (unsigned int)v;
    lead = 31 - __builtin_clz(a);
    frac = lead <= 23 ? (a << (23 - lead)) & 0x7FFFFF : (a >> (lead - 23)) & 0x7FFFFF;
    return (v < 0 ? 0x80000000u : 0) | (unsigned int)(lead - 11 + 127) << 23 | frac;
}

static unsigned int model(int op, unsigned int x, int nj)
{
    switch (op) {
    case 0: return m_vrefp(x, nj);
    case 1: return m_vrsqrtefp(x, nj);
    case 2: return m_vexptefp(x, nj);
    default: return m_vlogefp(x, nj);
    }
}

static unsigned int in_block[BLOCK] __attribute__((aligned(16)));
static unsigned int out_block[BLOCK] __attribute__((aligned(16)));
static unsigned int results[SWEEPS * WORDS_PER_SWEEP] __attribute__((aligned(16)));

static void set_vscr(unsigned int value)
{
    vector unsigned int v = { value, value, value, value };
    vec_mtvscr(v);
}

static void hardware(int op)
{
    unsigned int i;
    for (i = 0; i < BLOCK; i += 4) {
        vector float b = (vector float)vec_ld(0, &in_block[i]);
        vector float d;
        switch (op) {
        case 0: d = vec_re(b); break;
        case 1: d = vec_rsqrte(b); break;
        case 2: d = vec_expte(b); break;
        default: d = vec_loge(b); break;
        }
        vec_st((vector unsigned int)d, 0, &out_block[i]);
    }
}

/* Sweep op over [first, first + count) in blocks, accumulating into slot. */
static void sweep_range(int op, int nj, unsigned long long first, unsigned long long count,
                        unsigned int *slot)
{
    unsigned long long done = 0;
    unsigned long long bad = ((unsigned long long)slot[0] << 32) | slot[1];
    unsigned long long swept = ((unsigned long long)slot[5] << 32) | slot[6];
    while (done < count) {
        unsigned int i;
        for (i = 0; i < BLOCK; i++)
            in_block[i] = (unsigned int)(first + done + i);
        hardware(op);
        for (i = 0; i < BLOCK; i++) {
            unsigned int p = model(op, in_block[i], nj);
            if (p != out_block[i]) {
                if (bad == 0) {
                    slot[2] = in_block[i];
                    slot[3] = out_block[i];
                    slot[4] = p;
                }
                bad++;
            }
        }
        done += BLOCK;
    }
    swept += count;
    slot[0] = (unsigned int)(bad >> 32);
    slot[1] = (unsigned int)bad;
    slot[5] = (unsigned int)(swept >> 32);
    slot[6] = (unsigned int)swept;
}

int main(void)
{
    int op;
    unsigned int sign, be;

    memset(results, 0, sizeof(results));

    set_vscr(0);
    for (op = 0; op < 4; op++)
        sweep_range(op, 0, 0, 1ULL << 32, &results[op * WORDS_PER_SWEEP]);

    set_vscr(0x10000);
    for (op = 0; op < 4; op++)
        for (sign = 0; sign < 2; sign++)
            for (be = 0; be < 256; be++)
                if (be == 0 || be >= 250)
                    sweep_range(op, 1, (unsigned long long)(sign << 31 | be << 23), 1ULL << 23,
                                &results[(4 + op) * WORDS_PER_SWEEP]);

    set_vscr(0);
    RECIP_B[5] += 1;
    for (sign = 0; sign < 2; sign++)
        sweep_range(0, 0, (unsigned long long)(sign << 31 | 127u << 23), 1ULL << 23,
                    &results[8 * WORDS_PER_SWEEP]);
    RECIP_B[5] -= 1;
    set_vscr(0x10000);

    write_frame(results, sizeof(results));
    return 0;
}
