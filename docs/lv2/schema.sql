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

CREATE VIEW handling AS
SELECT route.ordinal, route.route, route.arm, arm.fidelity
FROM route
LEFT JOIN arm ON arm.arm = route.arm;
