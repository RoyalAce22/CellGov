-- Rendered from cellgov_lv2::archive by
--   cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate
-- Do not edit by hand: committed_archive_matches_generator fails on drift.

PRAGMA user_version = 6;

CREATE TABLE firmware (
    "fw" TEXT NOT NULL,
    "order" INTEGER NOT NULL,
    "release_date" TEXT,
    "priority" INTEGER NOT NULL,
    "role" TEXT CHECK ("role" IN ('baseline', 'census_reference', 'final')),
    PRIMARY KEY ("fw")
) STRICT;

CREATE TABLE pup (
    "pup_sha256" TEXT NOT NULL,
    "fw" TEXT NOT NULL REFERENCES firmware ("fw"),
    "size_bytes" INTEGER NOT NULL,
    "image_version" TEXT NOT NULL,
    "source_note" TEXT NOT NULL,
    "acquired" TEXT,
    PRIMARY KEY ("pup_sha256")
) STRICT;

CREATE TABLE arm (
    "arm" TEXT NOT NULL,
    "fidelity" TEXT NOT NULL CHECK ("fidelity" IN ('modeled', 'partial-state', 'abi-only', 'null-backend')),
    "ordinals" TEXT,
    PRIMARY KEY ("arm")
) STRICT;

CREATE TABLE route (
    "ordinal" INTEGER NOT NULL,
    "route" TEXT NOT NULL CHECK ("route" IN ('typed', 'routed', 'null_backend', 'runtime_fast_path')),
    "arm" TEXT REFERENCES arm ("arm"),
    PRIMARY KEY ("ordinal")
) STRICT;

CREATE TABLE kernel (
    "pup_sha256" TEXT NOT NULL REFERENCES pup ("pup_sha256"),
    "kernel_elf_sha256" TEXT NOT NULL,
    "table_base" TEXT NOT NULL,
    "entry_width" INTEGER NOT NULL,
    "entry_format" TEXT NOT NULL CHECK ("entry_format" IN ('ppc64_descriptor_pointer')),
    "entry_count" INTEGER NOT NULL,
    "discovery_method" TEXT NOT NULL CHECK ("discovery_method" IN ('sc_vector_descriptor_array')),
    "confidence" TEXT NOT NULL CHECK ("confidence" IN ('high')),
    "census_sha256" TEXT NOT NULL,
    "subentry_sha256" TEXT NOT NULL,
    "gate_sha256" TEXT NOT NULL,
    PRIMARY KEY ("pup_sha256")
) STRICT;

CREATE TABLE stub (
    "pup_sha256" TEXT NOT NULL REFERENCES kernel ("pup_sha256"),
    "descriptor" TEXT NOT NULL,
    "target" TEXT NOT NULL,
    "errno" TEXT NOT NULL,
    "errno_symbol" TEXT NOT NULL,
    "references" INTEGER NOT NULL,
    "primary" TEXT NOT NULL CHECK ("primary" IN ('yes', 'no')),
    PRIMARY KEY ("pup_sha256", "descriptor")
) STRICT;

CREATE TABLE subentry (
    "pup_sha256" TEXT NOT NULL REFERENCES kernel ("pup_sha256"),
    "ordinal" INTEGER NOT NULL REFERENCES route ("ordinal"),
    "selector_slot" TEXT NOT NULL CHECK ("selector_slot" IN ('r3', 'r4', 'r5', 'r6', 'r7', 'r8', 'r9', 'r10')),
    "packet" INTEGER NOT NULL,
    "class" TEXT NOT NULL CHECK ("class" IN ('implemented', 'stub', 'absent')),
    "target" TEXT NOT NULL,
    PRIMARY KEY ("pup_sha256", "ordinal", "packet")
) STRICT;

CREATE TABLE gate (
    "pup_sha256" TEXT NOT NULL REFERENCES kernel ("pup_sha256"),
    "ordinal" INTEGER NOT NULL REFERENCES route ("ordinal"),
    "state" TEXT NOT NULL CHECK ("state" IN ('gated', 'ungated', 'not_analysed')),
    "reads" TEXT,
    "fail_errno" TEXT,
    PRIMARY KEY ("pup_sha256", "ordinal")
) STRICT;

CREATE TABLE presence (
    "ordinal" INTEGER NOT NULL,
    "implemented_versions" TEXT,
    "stub_versions" TEXT,
    "absent_versions" TEXT,
    PRIMARY KEY ("ordinal")
) STRICT;

CREATE TABLE transitions (
    "record" INTEGER NOT NULL,
    "from_fw" TEXT NOT NULL REFERENCES firmware ("fw"),
    "to_fw" TEXT NOT NULL REFERENCES firmware ("fw"),
    "comparison" TEXT NOT NULL CHECK ("comparison" IN ('compared', 'not_compared')),
    "kind" TEXT CHECK ("kind" IN ('added', 'removed', 'class_changed', 'retargeted', 'gate_added', 'gate_removed')),
    "ordinal" INTEGER REFERENCES route ("ordinal"),
    PRIMARY KEY ("record")
) STRICT;

CREATE TABLE subentry_attribution (
    "ordinal" INTEGER NOT NULL REFERENCES route ("ordinal"),
    "selector_slot" TEXT NOT NULL CHECK ("selector_slot" IN ('r3', 'r4', 'r5', 'r6', 'r7', 'r8', 'r9', 'r10')),
    "packet" INTEGER NOT NULL,
    "source" TEXT NOT NULL CHECK ("source" IN ('psdevwiki')),
    "ref" TEXT NOT NULL,
    PRIMARY KEY ("ordinal", "selector_slot", "packet", "source")
) STRICT;

CREATE TABLE caller (
    "pup_sha256" TEXT NOT NULL REFERENCES pup ("pup_sha256"),
    "module" TEXT NOT NULL,
    "ordinal" INTEGER NOT NULL REFERENCES route ("ordinal"),
    "sites" TEXT NOT NULL,
    PRIMARY KEY ("pup_sha256", "module", "ordinal")
) STRICT;

CREATE TABLE caller_unresolved (
    "pup_sha256" TEXT NOT NULL REFERENCES pup ("pup_sha256"),
    "module" TEXT NOT NULL,
    "sites" TEXT,
    PRIMARY KEY ("pup_sha256", "module")
) STRICT;

CREATE TABLE reach (
    "pup_sha256" TEXT NOT NULL REFERENCES pup ("pup_sha256"),
    "module" TEXT NOT NULL,
    "export_nid" INTEGER NOT NULL,
    "ordinal" INTEGER NOT NULL REFERENCES route ("ordinal"),
    PRIMARY KEY ("pup_sha256", "module", "export_nid", "ordinal")
) STRICT;

CREATE TABLE behavior (
    "ordinal" INTEGER NOT NULL REFERENCES route ("ordinal"),
    "packet" TEXT,
    "same_as" INTEGER REFERENCES route ("ordinal"),
    "selector_slot" TEXT CHECK ("selector_slot" IN ('r3', 'r4', 'r5', 'r6', 'r7', 'r8', 'r9', 'r10')),
    "provenance_kind" TEXT NOT NULL CHECK ("provenance_kind" IN ('citation', 'firmware_reading', 'console_capture', 'non_public', 'unestablished')),
    "provenance_ref" TEXT,
    "witness" TEXT,
    "exception" TEXT CHECK ("exception" IN ('fabricated_success')),
    "arm_source" TEXT NOT NULL,
    PRIMARY KEY ("ordinal")
) STRICT;

CREATE TABLE name (
    "ordinal" INTEGER NOT NULL REFERENCES route ("ordinal"),
    "packet" TEXT,
    "name" TEXT NOT NULL,
    "source" TEXT NOT NULL CHECK ("source" IN ('psdevwiki', 'psl1ght', 'cellgov', 'non_public')),
    "ref" TEXT,
    "fw_from" TEXT,
    "fw_to" TEXT,
    UNIQUE ("ordinal", "packet", "source", "name")
) STRICT;

CREATE TABLE conflicts (
    "ordinal" INTEGER NOT NULL REFERENCES route ("ordinal"),
    "packet" TEXT,
    "name" TEXT NOT NULL,
    "source" TEXT NOT NULL CHECK ("source" IN ('psdevwiki', 'psl1ght', 'cellgov', 'non_public')),
    "disagreement" TEXT NOT NULL CHECK ("disagreement" IN ('spelling', 'name')),
    UNIQUE ("ordinal", "packet", "source", "name")
) STRICT;

CREATE TABLE census (
    "fw" TEXT NOT NULL REFERENCES firmware ("fw"),
    "ordinal" INTEGER NOT NULL REFERENCES route ("ordinal"),
    "class" TEXT NOT NULL CHECK ("class" IN ('implemented', 'stub', 'absent')),
    "target" TEXT,
    "dispatch" TEXT NOT NULL CHECK ("dispatch" IN ('flat', 'subtable', 'chain_incomplete')),
    PRIMARY KEY ("fw", "ordinal")
) STRICT;

CREATE VIEW handling AS
SELECT route.ordinal, route.route, route.arm, arm.fidelity
FROM route
LEFT JOIN arm ON arm.arm = route.arm;

CREATE VIEW authority AS
SELECT behavior.ordinal, route.arm, arm.fidelity,
       behavior.provenance_kind, behavior.provenance_ref,
       behavior.witness, behavior.exception, behavior.arm_source
FROM behavior
JOIN route ON route.ordinal = behavior.ordinal
LEFT JOIN arm ON arm.arm = route.arm;
