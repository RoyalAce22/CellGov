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
