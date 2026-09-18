-- Rendered from cellgov_lv2::archive by
--   cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate
-- Do not edit by hand: committed_archive_matches_generator fails on drift.

CREATE TABLE arm (
    arm TEXT NOT NULL,
    fidelity TEXT NOT NULL CHECK (fidelity IN ('modeled', 'partial-state', 'abi-only', 'null-backend')),
    ordinals TEXT,
    PRIMARY KEY (arm)
) STRICT;

CREATE TABLE route (
    ordinal INTEGER NOT NULL,
    route TEXT NOT NULL CHECK (route IN ('typed', 'routed', 'null_backend', 'runtime_fast_path')),
    arm TEXT REFERENCES arm (arm),
    PRIMARY KEY (ordinal)
) STRICT;

CREATE TABLE behavior (
    ordinal INTEGER NOT NULL REFERENCES route (ordinal),
    packet TEXT,
    same_as INTEGER REFERENCES route (ordinal),
    selector_slot TEXT CHECK (selector_slot IN ('r3', 'r4', 'r5', 'r6', 'r7', 'r8', 'r9', 'r10')),
    provenance_kind TEXT NOT NULL CHECK (provenance_kind IN ('citation', 'firmware_reading', 'console_capture', 'non_public', 'unestablished')),
    provenance_ref TEXT,
    witness TEXT,
    exception TEXT CHECK (exception IN ('fabricated_success')),
    arm_source TEXT NOT NULL,
    PRIMARY KEY (ordinal)
) STRICT;

CREATE TABLE name (
    ordinal INTEGER NOT NULL REFERENCES route (ordinal),
    packet TEXT,
    name TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('psdevwiki', 'psl1ght', 'cellgov', 'non_public')),
    ref TEXT,
    fw_from TEXT,
    fw_to TEXT,
    UNIQUE (ordinal, packet, source, name)
) STRICT;

CREATE TABLE conflicts (
    ordinal INTEGER NOT NULL REFERENCES route (ordinal),
    packet TEXT,
    name TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('psdevwiki', 'psl1ght', 'cellgov', 'non_public')),
    disagreement TEXT NOT NULL CHECK (disagreement IN ('spelling', 'name')),
    UNIQUE (ordinal, packet, source, name)
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
