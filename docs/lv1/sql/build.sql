-- Rendered from cellgov_lv1_archive by
--   CELLGOV_REGENERATE=1 cargo test -p cellgov_lv1_archive --test lv1_archive committed_archive_matches_generator
-- Do not edit by hand: committed_archive_matches_generator fails on drift.
-- Run in docs/lv1/: sqlite3 lv1.db < sql/build.sql

.bail on
PRAGMA foreign_keys = ON;
.read sql/schema.sql

CREATE TEMP TABLE staging_firmware ("fw" TEXT, "order" TEXT, "release_date" TEXT, "priority" TEXT, "role" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 ../lv2/tables/firmware.tsv staging_firmware
INSERT INTO firmware ("fw", "order", "release_date", "priority", "role")
SELECT "fw", CAST("order" AS INTEGER), NULLIF("release_date", 'none'), CAST("priority" AS INTEGER), NULLIF("role", 'none')
FROM staging_firmware;
DROP TABLE staging_firmware;

CREATE TEMP TABLE staging_pup ("pup_sha256" TEXT, "fw" TEXT, "size_bytes" TEXT, "image_version" TEXT, "source_note" TEXT, "acquired" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 ../lv2/tables/pup.tsv staging_pup
INSERT INTO pup ("pup_sha256", "fw", "size_bytes", "image_version", "source_note", "acquired")
SELECT "pup_sha256", "fw", CAST("size_bytes" AS INTEGER), "image_version", "source_note", NULLIF("acquired", 'none')
FROM staging_pup;
DROP TABLE staging_pup;

CREATE TEMP TABLE staging_hypercall_site ("pup_sha256" TEXT, "number" TEXT, "sites" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 tables/hypercall_site.tsv staging_hypercall_site
INSERT INTO hypercall_site ("pup_sha256", "number", "sites")
SELECT "pup_sha256", CAST("number" AS INTEGER), "sites"
FROM staging_hypercall_site;
DROP TABLE staging_hypercall_site;

CREATE TEMP TABLE staging_hypercall_unresolved ("pup_sha256" TEXT, "sites" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 tables/hypercall_unresolved.tsv staging_hypercall_unresolved
INSERT INTO hypercall_unresolved ("pup_sha256", "sites")
SELECT "pup_sha256", NULLIF("sites", 'none')
FROM staging_hypercall_unresolved;
DROP TABLE staging_hypercall_unresolved;

CREATE TEMP TABLE staging_hypercall ("number" TEXT, "name" TEXT, "name_source" TEXT, "inputs" TEXT, "outputs" TEXT, "family" TEXT, "leaf" TEXT, "fw" TEXT, "sites" TEXT, "lv1_493" TEXT, "state" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 tables/hypercall.tsv staging_hypercall
INSERT INTO hypercall ("number", "name", "name_source", "inputs", "outputs", "family", "leaf", "fw", "sites", "lv1_493", "state")
SELECT CAST("number" AS INTEGER), NULLIF("name", 'none'), NULLIF("name_source", 'none'), CAST(NULLIF("inputs", 'none') AS INTEGER), CAST(NULLIF("outputs", 'none') AS INTEGER), "family", "leaf", NULLIF("fw", 'none'), CAST("sites" AS INTEGER), "lv1_493", "state"
FROM staging_hypercall;
DROP TABLE staging_hypercall;

CREATE TEMP TABLE staging_repository_read ("pup_sha256" TEXT, "lpar" TEXT, "n1" TEXT, "n2" TEXT, "n3" TEXT, "n4" TEXT, "node" TEXT, "first" TEXT, "state" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 tables/repository_read.tsv staging_repository_read
INSERT INTO repository_read ("pup_sha256", "lpar", "n1", "n2", "n3", "n4", "node", "first", "state")
SELECT "pup_sha256", CAST("lpar" AS INTEGER), "n1", "n2", "n3", "n4", "node", CAST("first" AS INTEGER), "state"
FROM staging_repository_read;
DROP TABLE staging_repository_read;

CREATE TEMP TABLE staging_repository ("lpar" TEXT, "n1" TEXT, "n2" TEXT, "n3" TEXT, "n4" TEXT, "node" TEXT, "v1" TEXT, "v2" TEXT, "fw" TEXT, "source" TEXT, "citation" TEXT, "section" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 tables/repository.tsv staging_repository
INSERT INTO repository ("lpar", "n1", "n2", "n3", "n4", "node", "v1", "v2", "fw", "source", "citation", "section")
SELECT CAST("lpar" AS INTEGER), "n1", "n2", "n3", "n4", "node", "v1", "v2", "fw", "source", "citation", NULLIF("section", 'none')
FROM staging_repository;
DROP TABLE staging_repository;
