"""Tests for patch_toc.py on ELF images built in memory.

No toolchain is needed: the objdump count is passed to patch() as a
function, and the objdump text parser is tested on sample lines.

Run from the repository root:
    python3 -B -m unittest discover -s tests/micro/common -p "test_*.py"
"""

from __future__ import annotations

import contextlib
import io
import os
import struct
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import patch_toc as pt  # noqa: E402

TEXT_ADDR = 0x10000
GOT_ADDR = 0x20000
OPD_ADDR = 0x30000
TOC = GOT_ADDR + 0x8000
NOP = 0x60000000
STT_FUNC = 2
STB_GLOBAL = 1


def rldicr(rs: int, ra: int) -> int:
    """`rldicr rA, rS, 16, 47` as the toolchain encodes it."""
    return (30 << 26) | (rs << 21) | (ra << 16) | (16 << 11) | 0x3E4


def slwi(rs: int, ra: int) -> int:
    """`rlwinm rA, rS, 16, 0, 15`."""
    return (21 << 26) | (rs << 21) | (ra << 16) | (16 << 11) | (15 << 1)


def strtab(names: list[str]) -> tuple[bytes, dict[str, int]]:
    blob = b"\0"
    offsets = {"": 0}
    for name in names:
        offsets[name] = len(blob)
        blob += name.encode("ascii") + b"\0"
    return blob, offsets


def build_elf(
    text_words: list[int] | None = None,
    *,
    tocval_index: int | None = None,
    tocval_value: int | None = None,
    tocval_shndx: int = 1,
    got_addr: int = GOT_ADDR,
    ident: bytes = b"\x7fELF\x02\x02\x01",
    machine: int = 21,
    with_got: bool = True,
    with_tocval: bool = True,
    opd_entry: int | None = None,
    shnum_override: int | None = None,
) -> bytes:
    """A minimal big-endian ELF64 with .text, .got, .symtab, .strtab and .shstrtab.

    .text holds `text_words` followed by the 0xDEADBEEF placeholder, and
    __cg_tocval names the placeholder unless `tocval_index` says otherwise.
    With `opd_entry`, an .opd section holds one descriptor for a function
    symbol `callee` whose code starts at `opd_entry`.
    """
    words = list(text_words if text_words is not None else [NOP, rldicr(9, 9), NOP])
    words.append(pt.TOC_PLACEHOLDER)

    # (name, sh_type, addr, contents); the index of each is its position + 1.
    sections = [
        (".text", 1, TEXT_ADDR, b"".join(struct.pack(">I", w) for w in words)),
        (".got" if with_got else ".data", 1, got_addr, b"\0" * 16),
    ]
    if opd_entry is not None:
        sections.append((".opd", 1, OPD_ADDR, struct.pack(">QQQ", opd_entry, TOC, 0)))
    symtab_index = len(sections) + 1

    sym_names, sym_off = strtab(["main", "__cg_tocval", "callee"])
    syms = [pt.SYM.pack(0, 0, 0, 0, 0, 0)]
    syms.append(pt.SYM.pack(sym_off["main"], (STB_GLOBAL << 4) | STT_FUNC, 0, 1, TEXT_ADDR, 4 * (len(words) - 1)))
    if with_tocval:
        index = len(words) - 1 if tocval_index is None else tocval_index
        value = TEXT_ADDR + 4 * index if tocval_value is None else tocval_value
        syms.append(pt.SYM.pack(sym_off["__cg_tocval"], STB_GLOBAL << 4, 0, tocval_shndx, value, 4))
    if opd_entry is not None:
        syms.append(pt.SYM.pack(sym_off["callee"], (STB_GLOBAL << 4) | STT_FUNC, 0, symtab_index - 1, OPD_ADDR, 8))
    sections.append((".symtab", 2, 0, b"".join(syms)))
    sections.append((".strtab", 3, 0, sym_names))
    shstr, sh_off = strtab([s[0] for s in sections] + [".shstrtab"])
    sections.append((".shstrtab", 3, 0, shstr))

    body = bytearray(b"\0" * pt.EHDR.size)
    headers = [pt.SHDR.pack(0, 0, 0, 0, 0, 0, 0, 0, 0, 0)]
    for name, sh_type, addr, blob in sections:
        while len(body) % 8:
            body.append(0)
        link = symtab_index + 1 if sh_type == 2 else 0
        entsize = pt.SYM.size if sh_type == 2 else 0
        headers.append(pt.SHDR.pack(sh_off[name], sh_type, 0, addr, len(body), len(blob), link, 0, 8, entsize))
        body += blob
    while len(body) % 8:
        body.append(0)
    shoff = len(body)
    body += b"".join(headers)

    shnum = len(headers) if shnum_override is None else shnum_override
    ident16 = ident.ljust(16, b"\0")
    body[: pt.EHDR.size] = pt.EHDR.pack(
        ident16, 2, machine, 1, TEXT_ADDR, 0, shoff, 0, pt.EHDR.size, 0, 0, pt.SHDR.size, shnum, len(headers) - 1
    )
    return bytes(body)


def text_word(image: bytes, index: int) -> int:
    elf = pt.Elf("image", image)
    text = elf.progbits(".text")
    return struct.unpack_from(">I", image, text.offset + 4 * index)[0]


class Counter:
    """Stands in for the objdump count: `before` on the input ELF, `after` on the rewritten copy."""

    def __init__(self, before: int, after: int = 0) -> None:
        self.before = before
        self.after = after
        self.calls: list[str] = []

    def __call__(self, objdump: str, path: str) -> int:
        self.calls.append(path)
        return self.before if len(self.calls) == 1 else self.after


class ElfCase(unittest.TestCase):
    """Writes ELF images into a temporary directory and reads them back."""

    def setUp(self) -> None:
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.readelf = os.path.join(self.dir.name, "powerpc64-ps3-elf-readelf")

    def write(self, image: bytes) -> str:
        path = os.path.join(self.dir.name, "test.elf")
        with open(path, "wb") as f:
            f.write(image)
        return path

    def read(self, path: str) -> bytes:
        with open(path, "rb") as f:
            return f.read()

    def patch(self, path: str, counter: Counter, toc_only: bool = False) -> str:
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            pt.patch(path, self.readelf, counter, toc_only=toc_only)
        return out.getvalue()

    def refuse(self, image: bytes, error: type, counter: Counter | None = None) -> pt.PatchTocError:
        """patch() raises `error`, and the ELF and its directory are unchanged."""
        path = self.write(image)
        with self.assertRaises(error) as caught:
            self.patch(path, counter or Counter(1))
        self.assertEqual(self.read(path), image)
        self.assertEqual(os.listdir(self.dir.name), ["test.elf"])
        return caught.exception


class PredicateTests(unittest.TestCase):
    def test_every_register_matches_and_maps_to_slwi(self) -> None:
        for r in range(32):
            with self.subTest(r=r):
                self.assertTrue(pt.matches_predicate(rldicr(r, r)))
                self.assertEqual(pt.replacement(rldicr(r, r)), slwi(r, r))

    def test_the_word_the_toolchain_emits(self) -> None:
        # The RSX microtests' libgcm code holds rldicr r22,r22,16,47.
        self.assertEqual(rldicr(22, 22), 0x7AD683E4)
        self.assertTrue(pt.matches_predicate(0x7AD683E4))
        self.assertEqual(pt.replacement(0x7AD683E4), 0x56D6801E)

    def test_other_shapes_do_not_match(self) -> None:
        cases = {
            "rS != rA": rldicr(3, 4),
            "Rc=1 (rldicr.)": rldicr(9, 9) | 1,
            "shift 17": rldicr(9, 9) ^ (16 << 11) | (17 << 11),
            "other mask": (rldicr(9, 9) & ~0xFFF) | 0x3C4,
            "rlwinm": slwi(9, 9),
            "nop": NOP,
        }
        for name, word in cases.items():
            with self.subTest(name):
                self.assertFalse(pt.matches_predicate(word))


class DisassemblyTests(unittest.TestCase):
    def test_counts_only_the_rewritten_shape(self) -> None:
        listing = "\n".join(
            [
                "   10000:\t79 29 83 e4 \trldicr  r9,r9,16,47",
                "   10004:\t79 29 83 e4 \trldicr\tr9,r9,16,47",
                "   10008:\t79 29 83 e5 \trldicr. r9,r9,16,47",
                "   1000c:\t78 64 83 e4 \trldicr  r4,r3,16,47",
                "   10010:\t79 29 8b e4 \trldicr  r9,r9,17,47",
                "   10014:\t60 00 00 00 \tnop",
                "0000000000010000 <main>:",
                "",
            ]
        )
        self.assertEqual(pt.count_rldicr_lines(listing), 2)


class ObjdumpPathTests(unittest.TestCase):
    def test_only_the_tool_name_changes(self) -> None:
        readelf = os.path.join("opt", "readelf-tools", "powerpc64-ps3-elf-readelf")
        self.assertEqual(
            pt.objdump_path(readelf), os.path.join("opt", "readelf-tools", "powerpc64-ps3-elf-objdump")
        )
        self.assertEqual(pt.objdump_path("powerpc64-ps3-elf-readelf.exe"), "powerpc64-ps3-elf-objdump.exe")

    def test_a_name_without_readelf_is_a_usage_error(self) -> None:
        with self.assertRaises(pt.UsageError):
            pt.objdump_path("/usr/bin/powerpc64-ps3-elf-nm")


class PatchTests(ElfCase):
    def test_both_patches_are_applied(self) -> None:
        path = self.write(build_elf())
        counter = Counter(1)
        log = self.patch(path, counter)
        image = self.read(path)
        self.assertEqual(text_word(image, 1), slwi(9, 9))
        self.assertEqual(text_word(image, 3), TOC)
        self.assertIn("rldicr at 0x10004 (main+0x4): 0x792983e4 -> 0x5529801e", log)
        self.assertEqual(counter.calls[0], path)
        self.assertNotEqual(counter.calls[1], path)
        self.assertEqual(os.listdir(self.dir.name), ["test.elf"])

    def test_a_site_is_named_by_its_opd_descriptor(self) -> None:
        path = self.write(build_elf([NOP, NOP, rldicr(3, 3)], opd_entry=TEXT_ADDR + 4))
        log = self.patch(path, Counter(1))
        self.assertIn("rldicr at 0x10008 (callee+0x4)", log)

    def test_a_second_run_changes_no_byte(self) -> None:
        path = self.write(build_elf())
        self.patch(path, Counter(1))
        once = self.read(path)
        counter = Counter(0)
        self.patch(path, counter)
        self.assertEqual(self.read(path), once)
        self.assertEqual(counter.calls, [path])

    def test_no_rldicr_patches_the_toc_alone(self) -> None:
        path = self.write(build_elf([NOP, NOP]))
        self.patch(path, Counter(0))
        image = self.read(path)
        self.assertEqual(text_word(image, 0), NOP)
        self.assertEqual(text_word(image, 2), TOC)

    def test_toc_only_leaves_every_rldicr_in_place(self) -> None:
        path = self.write(build_elf())
        counter = Counter(1)
        log = self.patch(path, counter, toc_only=True)
        image = self.read(path)
        self.assertEqual(text_word(image, 1), rldicr(9, 9))
        self.assertEqual(text_word(image, 3), TOC)
        self.assertIn("rldicr left in place: 1 site(s)", log)
        self.assertEqual(counter.calls, [path])
        self.assertEqual(os.listdir(self.dir.name), ["test.elf"])

    def test_the_file_mode_is_kept(self) -> None:
        path = self.write(build_elf())
        os.chmod(path, 0o644)
        before = os.stat(path).st_mode
        self.patch(path, Counter(1))
        self.assertEqual(os.stat(path).st_mode, before)


class RefusalTests(ElfCase):
    def test_not_an_elf(self) -> None:
        err = self.refuse(b"not an elf at all" * 8, pt.ElfFormatError)
        self.assertIn("no ELF magic", str(err))

    def test_elf32(self) -> None:
        err = self.refuse(build_elf(ident=b"\x7fELF\x01\x02\x01"), pt.ElfFormatError)
        self.assertIn("ELFCLASS64", str(err))

    def test_little_endian(self) -> None:
        err = self.refuse(build_elf(ident=b"\x7fELF\x02\x01\x01"), pt.ElfFormatError)
        self.assertIn("big-endian", str(err))

    def test_wrong_machine(self) -> None:
        err = self.refuse(build_elf(machine=20), pt.ElfFormatError)
        self.assertIn("EM_PPC64", str(err))

    def test_truncated_header(self) -> None:
        self.refuse(build_elf()[:40], pt.ElfFormatError)

    def test_section_table_past_the_end(self) -> None:
        err = self.refuse(build_elf(shnum_override=200), pt.ElfFormatError)
        self.assertIn("section header table", str(err))

    def test_no_got(self) -> None:
        err = self.refuse(build_elf(with_got=False), pt.SectionNotFound)
        self.assertEqual(err.name, ".got")

    def test_no_tocval_symbol(self) -> None:
        err = self.refuse(build_elf(with_tocval=False), pt.SymbolNotFound)
        self.assertEqual(err.name, "__cg_tocval")

    def test_tocval_outside_its_section(self) -> None:
        err = self.refuse(build_elf(tocval_value=TEXT_ADDR + 0x1000), pt.SymbolOutsideSection)
        self.assertEqual(err.section, ".text")
        self.assertIn("at 0x0000000000011000 has no 4-byte word", str(err))

    def test_tocval_undefined_absolute_or_past_the_section_table(self) -> None:
        for shndx in (0, 0xFFF1, 50):
            with self.subTest(shndx=shndx):
                self.refuse(build_elf(tocval_shndx=shndx), pt.SymbolOutsideSection)

    def test_tocval_on_a_word_that_is_not_the_placeholder(self) -> None:
        err = self.refuse(build_elf(tocval_index=0), pt.PlaceholderMismatch)
        self.assertEqual(err.found, NOP)

    def test_toc_wider_than_32_bits(self) -> None:
        err = self.refuse(build_elf(got_addr=0xFFFFF000), pt.TocOutOfRange)
        self.assertEqual(err.toc, 0xFFFFF000 + 0x8000)
        self.assertEqual(str(err), "TOC 0x0000000100007000 does not fit the 32-bit word crt0.S loads")

    def test_count_disagrees_before_the_rewrite(self) -> None:
        err = self.refuse(build_elf(), pt.RldicrCountMismatch, Counter(2))
        self.assertEqual((err.patched, err.expected), (1, 2))

    def test_sites_left_after_the_rewrite(self) -> None:
        err = self.refuse(build_elf(), pt.RldicrCountMismatch, Counter(1, after=1))
        self.assertEqual(err.stage, "after the rewrite")


class ToolTests(unittest.TestCase):
    def test_a_missing_tool(self) -> None:
        with self.assertRaises(pt.ToolError) as caught:
            pt.run_tool("objdump", [os.path.join(tempfile.gettempdir(), "no-such-objdump-tool")])
        self.assertIsNone(caught.exception.returncode)
        self.assertIsInstance(caught.exception.__cause__, OSError)

    def test_output_that_is_not_utf8_is_read(self) -> None:
        argv = [sys.executable, "-c", "import sys; sys.stdout.buffer.write(b'\\xff\\xfe rldicr\\n')"]
        self.assertIn("rldicr", pt.run_tool("objdump", argv))

    def test_a_tool_that_fails(self) -> None:
        argv = [sys.executable, "-c", "import sys; sys.stderr.write('bad input\\n'); sys.exit(3)"]
        with self.assertRaises(pt.ToolError) as caught:
            pt.run_tool("objdump", argv)
        self.assertEqual(caught.exception.returncode, 3)
        self.assertIn("bad input", str(caught.exception))


class MainTests(ElfCase):
    def main(self, argv: list[str]) -> tuple[int, str]:
        err = io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
            code = pt.main(argv)
        return code, err.getvalue()

    def test_every_refusal_has_its_own_exit_code(self) -> None:
        classes = [
            pt.UsageError,
            pt.ToolError,
            pt.ElfIoError,
            pt.ElfFormatError,
            pt.SectionNotFound,
            pt.SymbolNotFound,
            pt.SymbolOutsideSection,
            pt.PlaceholderMismatch,
            pt.TocOutOfRange,
            pt.RldicrCountMismatch,
        ]
        codes = [c.exit_code for c in classes]
        self.assertEqual(len(set(codes)), len(codes))
        self.assertNotIn(0, codes)
        self.assertNotIn(pt.EXIT_INTERNAL, codes)
        self.assertNotIn(pt.EXIT_INTERRUPTED, codes)

    def test_wrong_arguments(self) -> None:
        code, err = self.main(["only-one"])
        self.assertEqual(code, pt.UsageError.exit_code)
        self.assertIn("patch_toc: error: usage:", err)
        self.assertNotIn("Traceback", err)

    def test_the_toc_only_flag_reaches_patch(self) -> None:
        for argv, toc_only in ((["a.elf", "readelf", "nm"], False), (["a.elf", "readelf", "nm", "--toc-only"], True)):
            with self.subTest(toc_only=toc_only), mock.patch.object(pt, "patch") as patch:
                code, _ = self.main(argv)
                self.assertEqual(code, 0)
                self.assertEqual(patch.call_args.kwargs, {"toc_only": toc_only})

    def test_a_missing_elf(self) -> None:
        code, err = self.main([os.path.join(self.dir.name, "absent.elf"), self.readelf, "nm"])
        self.assertEqual(code, pt.ElfIoError.exit_code)
        self.assertIn("caused by: FileNotFoundError", err)
        self.assertNotIn("Traceback", err)

    def test_a_refusal_prints_one_error_line(self) -> None:
        path = self.write(build_elf(with_got=False))
        code, err = self.main([path, self.readelf, "nm"])
        self.assertEqual(code, pt.SectionNotFound.exit_code)
        self.assertEqual(err, "patch_toc: error: no PROGBITS section named .got\n")

    def test_an_unexpected_exception_is_an_internal_error(self) -> None:
        with mock.patch.object(pt, "patch", side_effect=ValueError("boom")), mock.patch.dict(os.environ):
            os.environ.pop(pt.TRACEBACK_ENV, None)
            code, err = self.main(["a", "readelf", "nm"])
        self.assertEqual(code, pt.EXIT_INTERNAL)
        self.assertEqual(err, "patch_toc: internal error: ValueError: boom\n")

    def test_the_traceback_is_printed_on_request(self) -> None:
        with mock.patch.object(pt, "patch", side_effect=ValueError("boom")), mock.patch.dict(
            os.environ, {pt.TRACEBACK_ENV: "1"}
        ):
            code, err = self.main(["a", "readelf", "nm"])
        self.assertEqual(code, pt.EXIT_INTERNAL)
        self.assertIn("Traceback", err)

    def test_an_interrupt(self) -> None:
        with mock.patch.object(pt, "patch", side_effect=KeyboardInterrupt):
            code, _ = self.main(["a", "readelf", "nm"])
        self.assertEqual(code, pt.EXIT_INTERRUPTED)


if __name__ == "__main__":
    unittest.main()
