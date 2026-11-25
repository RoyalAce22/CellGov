-- Rendered from cellgov_lv1_archive by
--   CELLGOV_REGENERATE=1 cargo test -p cellgov_lv1_archive --test lv1_archive committed_archive_matches_generator
-- Do not edit by hand: committed_archive_matches_generator fails on drift.

PRAGMA user_version = 2;

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

CREATE TABLE hypercall_site (
    "pup_sha256" TEXT NOT NULL REFERENCES pup ("pup_sha256"),
    "number" INTEGER NOT NULL,
    "sites" TEXT NOT NULL,
    PRIMARY KEY ("pup_sha256", "number")
) STRICT;

CREATE TABLE hypercall_unresolved (
    "pup_sha256" TEXT NOT NULL REFERENCES pup ("pup_sha256"),
    "sites" TEXT,
    PRIMARY KEY ("pup_sha256")
) STRICT;

CREATE TABLE hypercall (
    "number" INTEGER NOT NULL,
    "name" TEXT,
    "name_source" TEXT CHECK ("name_source" IN ('header', 'psdevwiki', 'cellgov')),
    "inputs" INTEGER,
    "outputs" INTEGER,
    "family" TEXT NOT NULL CHECK ("family" IN ('console', 'device', 'interrupt', 'logical_spe', 'memory', 'other', 'partition', 'repository', 'undocumented', 'unnamed')),
    "leaf" TEXT NOT NULL CHECK ("leaf" IN ('boot', 'dispatch', 'interrupt_delivery', 'logical_spe', 'repository', 'translation')),
    "fw" TEXT,
    "sites" INTEGER NOT NULL,
    "lv1_493" TEXT NOT NULL CHECK ("lv1_493" IN ('handler', 'invalid', 'past_table')),
    "state" TEXT NOT NULL CHECK ("state" IN ('answered', 'null_backend')),
    PRIMARY KEY ("number")
) STRICT;

CREATE TABLE repository_read (
    "pup_sha256" TEXT NOT NULL REFERENCES pup ("pup_sha256"),
    "lpar" INTEGER NOT NULL,
    "n1" TEXT NOT NULL,
    "n2" TEXT NOT NULL,
    "n3" TEXT NOT NULL,
    "n4" TEXT NOT NULL,
    "node" TEXT NOT NULL,
    "first" INTEGER NOT NULL,
    "state" TEXT NOT NULL CHECK ("state" IN ('answered', 'refused')),
    PRIMARY KEY ("pup_sha256", "lpar", "n1", "n2", "n3", "n4")
) STRICT;

CREATE TABLE repository (
    "lpar" INTEGER NOT NULL,
    "n1" TEXT NOT NULL,
    "n2" TEXT NOT NULL,
    "n3" TEXT NOT NULL,
    "n4" TEXT NOT NULL,
    "node" TEXT NOT NULL,
    "v1" TEXT NOT NULL,
    "v2" TEXT NOT NULL,
    "fw" TEXT NOT NULL,
    "source" TEXT NOT NULL CHECK ("source" IN ('cellgov', 'console', 'linux', 'psdevwiki')),
    "citation" TEXT NOT NULL,
    "section" TEXT,
    PRIMARY KEY ("lpar", "n1", "n2", "n3", "n4", "fw")
) STRICT;
