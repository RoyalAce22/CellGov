-- Rendered from cellgov_lv2::archive by
--   cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate
-- Do not edit by hand: committed_archive_matches_generator fails on drift.
-- Run in docs/lv2/: sqlite3 lv2.db < build.sql

.bail on
PRAGMA foreign_keys = ON;
.read schema.sql

CREATE TEMP TABLE staging_arm (arm TEXT, fidelity TEXT, ordinals TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 arm.tsv staging_arm
INSERT INTO arm (arm, fidelity, ordinals)
SELECT arm, fidelity, NULLIF(ordinals, 'none')
FROM staging_arm;
DROP TABLE staging_arm;

CREATE TEMP TABLE staging_route (ordinal TEXT, route TEXT, arm TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 route.tsv staging_route
INSERT INTO route (ordinal, route, arm)
SELECT CAST(ordinal AS INTEGER), route, NULLIF(arm, 'none')
FROM staging_route;
DROP TABLE staging_route;

CREATE TEMP TABLE staging_behavior (ordinal TEXT, packet TEXT, same_as TEXT, selector_slot TEXT, provenance_kind TEXT, provenance_ref TEXT, witness TEXT, exception TEXT, arm_source TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 behavior.tsv staging_behavior
INSERT INTO behavior (ordinal, packet, same_as, selector_slot, provenance_kind, provenance_ref, witness, exception, arm_source)
SELECT CAST(ordinal AS INTEGER), NULLIF(packet, 'none'), CAST(NULLIF(same_as, 'none') AS INTEGER), NULLIF(selector_slot, 'none'), provenance_kind, NULLIF(provenance_ref, 'none'), NULLIF(witness, 'none'), NULLIF(exception, 'none'), arm_source
FROM staging_behavior;
DROP TABLE staging_behavior;
