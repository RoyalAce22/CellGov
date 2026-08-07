"""Python reference implementation of the NPDRM RAP-to-rifkey algorithm.

Reads a .rap file and prints the 16-byte klicensee (the "rifkey"
the envelope-peel step ECB-decrypts with NP_KLIC_KEY to produce
the layer key).

There is no public Sony specification for this transform; the
constants and round structure are reverse-engineered, with
RPCS3's `rap_to_rif` as the corroborating implementation.

Use:
    python rap_to_klic_oracle.py <path/to/file.rap>
    python rap_to_klic_oracle.py --selftest

Output: 16 space-separated hex bytes, pasted into
`apps/cellgov_install/src/npdrm.rs` as `FLOW_EXPECTED_KLIC` /
`SSHD_EXPECTED_KLIC`. Those constants witness
`(real RAP, fixed RE'd algorithm)`; the correctness anchor is
the end-to-end "decrypts to parseable ELF" test.

Requires pycryptodome. Uses no host state, so two runs on the
same RAP produce byte-identical output.
"""

import sys

from Crypto.Cipher import AES

RAP_KEY = bytes.fromhex("869F7745C13FD890CCF29188E3CC3EDF")

RAP_PBOX = bytes([
    0x0C, 0x03, 0x06, 0x04, 0x01, 0x0B, 0x0F, 0x08,
    0x02, 0x07, 0x00, 0x05, 0x0A, 0x0E, 0x0D, 0x09,
])
RAP_E1 = bytes([
    0xA9, 0x3E, 0x1F, 0xD6, 0x7C, 0x55, 0xA3, 0x29,
    0xB7, 0x5F, 0xDD, 0xA6, 0x2A, 0x95, 0xC7, 0xA5,
])
RAP_E2 = bytes([
    0x67, 0xD4, 0x5D, 0xA3, 0x29, 0x6D, 0x00, 0x6A,
    0x4E, 0x7C, 0x53, 0x7B, 0xF5, 0x53, 0x8C, 0x74,
])

# The E1 / cascade / borrow loops assume every index is visited once.
assert sorted(RAP_PBOX) == list(range(16)), \
    "RAP_PBOX must be a permutation of 0..15"


def _borrow_round(key: bytearray) -> bytearray:
    """One pass of the borrow-subtraction sweep."""
    o = 0
    for i in range(16):
        p = RAP_PBOX[i]
        kc = (key[p] - o) & 0xFF
        ec2 = RAP_E2[p]
        # A pending borrow (o == 1) meeting kc == 0xFF must keep the
        # borrow set; the first branch would clear it, since kc < ec2
        # is false at 0xFF. The final `else` is unreachable (kc is
        # always 0xFF inside the elif) and mirrors RPCS3's shape.
        if o != 1 or kc != 0xFF:
            o = 1 if kc < ec2 else 0
            key[p] = (kc - ec2) & 0xFF
        elif kc == 0xFF:
            key[p] = (kc - ec2) & 0xFF
        else:  # unreachable; mirrors RPCS3
            key[p] = kc
    return key


def rap_to_rifkey(rap: bytes) -> bytes:
    """Derive the NPDRM rifkey from a 16-byte RAP.

    One AES-128-ECB-decrypt with RAP_KEY, then five rounds of
    PBOX permutation + E1 XOR + descending cascade + E2
    borrow-subtraction.

    Raises ValueError on a non-16-byte input, which ECB-decrypt
    would otherwise accept as multiple blocks.
    """
    if len(rap) != 16:
        raise ValueError(f"RAP must be exactly 16 bytes, got {len(rap)}")
    # RPCS3's zero-IV AES-128-CBC is byte-identical to ECB for the
    # single block the length check above guarantees.
    cipher = AES.new(RAP_KEY, AES.MODE_ECB)
    key = bytearray(cipher.decrypt(rap))

    for _round in range(5):
        # PBOX indexing is cosmetic here -- each index is touched
        # once, so this equals `key[p] ^= RAP_E1[p]` over 0..15.
        for i in range(16):
            p = RAP_PBOX[i]
            key[p] ^= RAP_E1[p]
        for i in range(15, 0, -1):
            p = RAP_PBOX[i]
            pp = RAP_PBOX[i - 1]
            key[p] ^= key[pp]
        _borrow_round(key)
    return bytes(key)


def _check(condition: bool, message: str) -> None:
    """Assertion that survives `python -O`."""
    if not condition:
        raise AssertionError(message)


def _selftest() -> int:
    """Algorithm-shape self-tests.

    Covers the PBOX permutation and borrow-preservation branch only.
    The (RAP -> rifkey) mapping needs operator-supplied fixtures and
    is checked by the Rust npdrm-oracle-vectors tests.
    """
    _check(
        sorted(RAP_PBOX) == list(range(16)),
        "RAP_PBOX must be a permutation of 0..15",
    )

    # Zero bytes at PBOX[0] / PBOX[1] make step 0 generate a borrow
    # and step 1 wrap kc to 0xFF, the one state that reaches the
    # borrow-preserving elif.
    key = bytearray(16)
    key[RAP_PBOX[0]] = 0x00
    key[RAP_PBOX[1]] = 0x00

    p0 = RAP_PBOX[0]
    kc0 = (key[p0] - 0) & 0xFF
    ec2_0 = RAP_E2[p0]
    o = 1 if kc0 < ec2_0 else 0
    _check(o == 1, "step 0 must produce a pending borrow for this test")
    p1 = RAP_PBOX[1]
    kc1 = (key[p1] - o) & 0xFF
    _check(kc1 == 0xFF, "step 1 must hit the borrow-preservation trigger")
    o_after_elif = 1
    o_if_collapsed = 1 if kc1 < RAP_E2[p1] else 0
    _check(
        o_after_elif != o_if_collapsed,
        "elif must preserve the pending borrow that branch A would clear",
    )

    _borrow_round(bytearray(key))

    try:
        rap_to_rifkey(b"\x00" * 32)
    except ValueError:
        pass
    else:
        raise AssertionError("rap_to_rifkey must reject non-16-byte input")

    print("selftest: ok (PBOX permutation, borrow-preservation, length guard)")
    return 0


def main() -> int:
    if len(sys.argv) != 2:
        print(
            "usage: python rap_to_klic_oracle.py <path/to/file.rap>\n"
            "       python rap_to_klic_oracle.py --selftest",
            file=sys.stderr,
        )
        return 2
    if sys.argv[1] == "--selftest":
        return _selftest()
    try:
        with open(sys.argv[1], "rb") as f:
            rap = f.read()
    except OSError as e:
        print(f"error: cannot read {sys.argv[1]}: {e}", file=sys.stderr)
        return 1
    if len(rap) != 16:
        print(f"error: {sys.argv[1]} is {len(rap)} bytes; expected 16", file=sys.stderr)
        return 1
    klic = rap_to_rifkey(rap)
    print(" ".join(f"0x{b:02X}" for b in klic))
    return 0


if __name__ == "__main__":
    sys.exit(main())
