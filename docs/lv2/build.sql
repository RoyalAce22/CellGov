-- Rendered from cellgov_lv2::archive by
--   cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate
-- Do not edit by hand: committed_archive_matches_generator fails on drift.
-- Run in docs/lv2/: sqlite3 lv2.db < build.sql

.bail on
PRAGMA foreign_keys = ON;
.read schema.sql

CREATE TEMP TABLE staging_firmware ("fw" TEXT, "order" TEXT, "release_date" TEXT, "priority" TEXT, "role" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 firmware.tsv staging_firmware
INSERT INTO firmware ("fw", "order", "release_date", "priority", "role")
SELECT "fw", CAST("order" AS INTEGER), NULLIF("release_date", 'none'), CAST("priority" AS INTEGER), NULLIF("role", 'none')
FROM staging_firmware;
DROP TABLE staging_firmware;

CREATE TEMP TABLE staging_pup ("pup_sha256" TEXT, "fw" TEXT, "size_bytes" TEXT, "image_version" TEXT, "source_note" TEXT, "acquired" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 pup.tsv staging_pup
INSERT INTO pup ("pup_sha256", "fw", "size_bytes", "image_version", "source_note", "acquired")
SELECT "pup_sha256", "fw", CAST("size_bytes" AS INTEGER), "image_version", "source_note", NULLIF("acquired", 'none')
FROM staging_pup;
DROP TABLE staging_pup;

CREATE TEMP TABLE staging_arm ("arm" TEXT, "fidelity" TEXT, "ordinals" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 arm.tsv staging_arm
INSERT INTO arm ("arm", "fidelity", "ordinals")
SELECT "arm", "fidelity", NULLIF("ordinals", 'none')
FROM staging_arm;
DROP TABLE staging_arm;

CREATE TEMP TABLE staging_route ("ordinal" TEXT, "route" TEXT, "arm" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 route.tsv staging_route
INSERT INTO route ("ordinal", "route", "arm")
SELECT CAST("ordinal" AS INTEGER), "route", NULLIF("arm", 'none')
FROM staging_route;
DROP TABLE staging_route;

CREATE TEMP TABLE staging_kernel ("pup_sha256" TEXT, "kernel_elf_sha256" TEXT, "table_base" TEXT, "entry_width" TEXT, "entry_format" TEXT, "entry_count" TEXT, "discovery_method" TEXT, "confidence" TEXT, "census_sha256" TEXT, "subentry_sha256" TEXT, "gate_sha256" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 kernel.tsv staging_kernel
INSERT INTO kernel ("pup_sha256", "kernel_elf_sha256", "table_base", "entry_width", "entry_format", "entry_count", "discovery_method", "confidence", "census_sha256", "subentry_sha256", "gate_sha256")
SELECT "pup_sha256", "kernel_elf_sha256", "table_base", CAST("entry_width" AS INTEGER), "entry_format", CAST("entry_count" AS INTEGER), "discovery_method", "confidence", "census_sha256", "subentry_sha256", "gate_sha256"
FROM staging_kernel;
DROP TABLE staging_kernel;

CREATE TEMP TABLE staging_stub ("pup_sha256" TEXT, "descriptor" TEXT, "target" TEXT, "errno" TEXT, "errno_symbol" TEXT, "references" TEXT, "primary" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 stub.tsv staging_stub
INSERT INTO stub ("pup_sha256", "descriptor", "target", "errno", "errno_symbol", "references", "primary")
SELECT "pup_sha256", "descriptor", "target", "errno", "errno_symbol", CAST("references" AS INTEGER), "primary"
FROM staging_stub;
DROP TABLE staging_stub;

CREATE TEMP TABLE staging_subentry ("pup_sha256" TEXT, "ordinal" TEXT, "selector_slot" TEXT, "packet" TEXT, "class" TEXT, "target" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 subentry.tsv staging_subentry
INSERT INTO subentry ("pup_sha256", "ordinal", "selector_slot", "packet", "class", "target")
SELECT "pup_sha256", CAST("ordinal" AS INTEGER), "selector_slot", CAST("packet" AS INTEGER), "class", "target"
FROM staging_subentry;
DROP TABLE staging_subentry;

CREATE TEMP TABLE staging_gate ("pup_sha256" TEXT, "ordinal" TEXT, "state" TEXT, "reads" TEXT, "fail_errno" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 gate.tsv staging_gate
INSERT INTO gate ("pup_sha256", "ordinal", "state", "reads", "fail_errno")
SELECT "pup_sha256", CAST("ordinal" AS INTEGER), "state", NULLIF("reads", 'none'), NULLIF("fail_errno", 'none')
FROM staging_gate;
DROP TABLE staging_gate;

CREATE TEMP TABLE staging_presence ("ordinal" TEXT, "implemented_versions" TEXT, "stub_versions" TEXT, "absent_versions" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 presence.tsv staging_presence
INSERT INTO presence ("ordinal", "implemented_versions", "stub_versions", "absent_versions")
SELECT CAST("ordinal" AS INTEGER), NULLIF("implemented_versions", 'none'), NULLIF("stub_versions", 'none'), NULLIF("absent_versions", 'none')
FROM staging_presence;
DROP TABLE staging_presence;

CREATE TEMP TABLE staging_subentry_attribution ("ordinal" TEXT, "selector_slot" TEXT, "packet" TEXT, "source" TEXT, "ref" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 subentry_attribution.tsv staging_subentry_attribution
INSERT INTO subentry_attribution ("ordinal", "selector_slot", "packet", "source", "ref")
SELECT CAST("ordinal" AS INTEGER), "selector_slot", CAST("packet" AS INTEGER), "source", "ref"
FROM staging_subentry_attribution;
DROP TABLE staging_subentry_attribution;

CREATE TEMP TABLE staging_caller ("pup_sha256" TEXT, "module" TEXT, "ordinal" TEXT, "sites" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 caller.tsv staging_caller
INSERT INTO caller ("pup_sha256", "module", "ordinal", "sites")
SELECT "pup_sha256", "module", CAST("ordinal" AS INTEGER), "sites"
FROM staging_caller;
DROP TABLE staging_caller;

CREATE TEMP TABLE staging_caller_unresolved ("pup_sha256" TEXT, "module" TEXT, "sites" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 caller_unresolved.tsv staging_caller_unresolved
INSERT INTO caller_unresolved ("pup_sha256", "module", "sites")
SELECT "pup_sha256", "module", NULLIF("sites", 'none')
FROM staging_caller_unresolved;
DROP TABLE staging_caller_unresolved;

CREATE TEMP TABLE staging_reach ("pup_sha256" TEXT, "module" TEXT, "export_nid" TEXT, "ordinal" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 reach.tsv staging_reach
INSERT INTO reach ("pup_sha256", "module", "export_nid", "ordinal")
SELECT "pup_sha256", "module", CAST("export_nid" AS INTEGER), CAST("ordinal" AS INTEGER)
FROM staging_reach;
DROP TABLE staging_reach;

CREATE TEMP TABLE staging_behavior ("ordinal" TEXT, "packet" TEXT, "same_as" TEXT, "selector_slot" TEXT, "provenance_kind" TEXT, "provenance_ref" TEXT, "witness" TEXT, "exception" TEXT, "arm_source" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 behavior.tsv staging_behavior
INSERT INTO behavior ("ordinal", "packet", "same_as", "selector_slot", "provenance_kind", "provenance_ref", "witness", "exception", "arm_source")
SELECT CAST("ordinal" AS INTEGER), NULLIF("packet", 'none'), CAST(NULLIF("same_as", 'none') AS INTEGER), NULLIF("selector_slot", 'none'), "provenance_kind", NULLIF("provenance_ref", 'none'), NULLIF("witness", 'none'), NULLIF("exception", 'none'), "arm_source"
FROM staging_behavior;
DROP TABLE staging_behavior;

CREATE TEMP TABLE staging_name ("ordinal" TEXT, "packet" TEXT, "name" TEXT, "source" TEXT, "ref" TEXT, "fw_from" TEXT, "fw_to" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 name.tsv staging_name
INSERT INTO name ("ordinal", "packet", "name", "source", "ref", "fw_from", "fw_to")
SELECT CAST("ordinal" AS INTEGER), NULLIF("packet", 'none'), "name", "source", NULLIF("ref", 'none'), NULLIF("fw_from", 'none'), NULLIF("fw_to", 'none')
FROM staging_name;
DROP TABLE staging_name;

CREATE TEMP TABLE staging_conflicts ("ordinal" TEXT, "packet" TEXT, "name" TEXT, "source" TEXT, "disagreement" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 conflicts.tsv staging_conflicts
INSERT INTO conflicts ("ordinal", "packet", "name", "source", "disagreement")
SELECT CAST("ordinal" AS INTEGER), NULLIF("packet", 'none'), "name", "source", "disagreement"
FROM staging_conflicts;
DROP TABLE staging_conflicts;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.02.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.10.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.11.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.30.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.31.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.32.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.50.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.51.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.54.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.60.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.70.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.80.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.81.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.82.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.90.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.92.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.93.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-1.94.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.00.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.01.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.10.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.17.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.20.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.30.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.35.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.36.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.40.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.41.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.42.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.43.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.50.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.52.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.53.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.60.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.70.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.76.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-2.80.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.00.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.01.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.10.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.15.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.16.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.21.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.30.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.40.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.41.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.42.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.50.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.55.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.56.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.60.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.61.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.65.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.66.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.70.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.71.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.72.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-3.73.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.00.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.01.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.10.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.11.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.20.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.21.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.25.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.30.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.31.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.40.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.41.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.45.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.46.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.50.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.53.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.55.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.60.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.65.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.66.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.70.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.75.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.76.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.78.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.80.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.81.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.82.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.83.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.84.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.85.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.86.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.87.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.88.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.89.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.90.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.91.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.92.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;

CREATE TEMP TABLE staging_census ("fw" TEXT, "ordinal" TEXT, "class" TEXT, "target" TEXT, "dispatch" TEXT);
.import --ascii --colsep "\t" --rowsep "\n" --skip 1 census/fw-4.93.tsv staging_census
INSERT INTO census ("fw", "ordinal", "class", "target", "dispatch")
SELECT "fw", CAST("ordinal" AS INTEGER), "class", NULLIF("target", 'none'), "dispatch"
FROM staging_census;
DROP TABLE staging_census;
