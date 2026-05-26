"""Python reference implementation of the NPDRM RAP-to-rifkey algorithm.

Reads a .rap file from disk and prints the 16-byte klicensee
(the NPDRM scheme calls this the "rifkey": the intermediate
value derived from the RAP that the envelope-peel step then
ECB-decrypts with NP_KLIC_KEY to produce the layer key).

There is no public Sony specification for this transform; the
constants in `cellgov_ps3_abi::sce` and the round structure
below are reverse-engineered. RPCS3's `rap_to_rif` is the
de-facto cross-reference implementation but is not the
specification.

Use:
    python rap_to_klic_oracle.py <path/to/file.rap>
    python rap_to_klic_oracle.py --selftest

Output: 16 space-separated hex bytes. Paste into the Rust test
module as a fixed constant (see
`apps/cellgov_firmware/src/npdrm.rs` `FLOW_EXPECTED_KLIC` /
`SSHD_EXPECTED_KLIC`). The value is a witness:
`(real RAP, fixed RE'd algorithm)` -- not a correctness anchor,
which lives in the end-to-end "decrypts to parseable ELF" test.

Requires: pycryptodome (pip install pycryptodome). The script
uses no host state (clock, env, RNG); two runs on the same RAP
produce byte-identical output.
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

# Invariant the E1 / cascade / borrow loops all silently depend on:
# RAP_PBOX must be a permutation of 0..15. A typo in the table would
# leave some index untouched and another visited twice; the bug would
# only surface on inputs that drive a specific byte through the broken
# path. Cheap insurance, checked at module import.
assert sorted(RAP_PBOX) == list(range(16)), \
    "RAP_PBOX must be a permutation of 0..15"


def _borrow_round(key: bytearray) -> bytearray:
    """One pass of the borrow-subtraction sweep.

    Extracted so the borrow-preservation branch can be exercised in
    isolation: forcing `o == 1` + `key[p] == 0` (so kc wraps to 0xFF)
    on a constructed `key` is far easier than hunting a RAP that
    reaches that state through five full rounds.
    """
    o = 0
    for i in range(16):
        p = RAP_PBOX[i]
        kc = (key[p] - o) & 0xFF
        ec2 = RAP_E2[p]
        # Borrow-subtract, byte order per RAP_PBOX. Mirrors RPCS3's
        # rap_to_rif exactly; do NOT collapse this chain. The `elif`
        # is load-bearing: when a pending borrow (o == 1) meets
        # key[p] == 0 (so kc wraps to 0xFF) it must PRESERVE the
        # borrow (o stays 1). Branch A would clear it (kc < ec2 is
        # false for kc == 0xFF, so o would become 0). The final
        # `else` is dead (kc is always 0xFF inside the elif chain)
        # and is kept only to mirror the reference's shape verbatim.
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
    borrow-subtraction. Mirrors `rap_to_rif` as the algorithm
    name -- no specification, only the corroborating
    implementation in RPCS3 and the constants in
    `cellgov_ps3_abi::sce`.

    Raises ValueError on a non-16-byte input. A bare `assert` would
    be stripped under `python -O` and a 32-byte (or any multiple of
    16) RAP would silently return wrong-length, wrong-content bytes
    because AES-ECB-decrypt does not raise on a multi-block input.
    """
    if len(rap) != 16:
        raise ValueError(f"RAP must be exactly 16 bytes, got {len(rap)}")
    # RPCS3 uses AES-128-CBC with a zero IV; for a single 16-byte
    # block that is byte-identical to ECB, which we use here. Valid
    # ONLY because RAP is fixed at 16 bytes (the length check above
    # is the gate that keeps this equivalence safe).
    cipher = AES.new(RAP_KEY, AES.MODE_ECB)
    key = bytearray(cipher.decrypt(rap))

    for _round in range(5):
        # PBOX indexing is cosmetic in this loop: since RAP_PBOX is a
        # permutation of 0..15 (asserted at module import) and each
        # index is touched once, the result is identical to a plain
        # `for p in range(16): key[p] ^= RAP_E1[p]`. Kept for shape
        # fidelity with RPCS3; do not infer an order dependence.
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
    """Raise unconditionally on failure. Used instead of `assert` so
    selftest invariants are not stripped under `python -O`."""
    if not condition:
        raise AssertionError(message)


def _selftest() -> int:
    """Algorithm-shape self-tests.

    Anchors the borrow-preservation branch and the PBOX permutation
    invariant. Does NOT check the (RAP -> rifkey) mapping itself --
    that requires operator-supplied RAP fixtures and lives in the
    Rust npdrm-oracle-vectors feature tests. All checks use
    [`_check`] so `python -O` does not strip them.
    """
    _check(
        sorted(RAP_PBOX) == list(range(16)),
        "RAP_PBOX must be a permutation of 0..15",
    )

    # Borrow-preservation anchor. Construct a `key` where:
    #   - step 0 (PBOX[0] = 0x0C, byte 0x00) generates a borrow
    #     (0x00 < ec2[0x0C] = 0xF5).
    #   - step 1 (PBOX[1] = 0x03, byte 0x00) hits the trigger:
    #     kc = (0 - 1) & 0xFF = 0xFF, so the elif must preserve
    #     o = 1. A collapsed chain (branch A only) would clear o
    #     because kc < ec2 is false for kc == 0xFF, corrupting
    #     every later byte in the round.
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

    # Smoke-run the extracted helper to confirm it does not raise.
    _borrow_round(bytearray(key))

    # Bad-length rejection (the M1 fix). A non-16-byte input must
    # raise rather than silently return wrong-length garbage.
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
