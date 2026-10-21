-- Applied inside one Rust transaction by `ink_snapshot_label::migrate_v20`;
-- the file therefore has no BEGIN/COMMIT.
INSERT INTO schema_migrations(version,name) VALUES(20,'ink_snapshot_labels_n3d');

-- N3d: an optional, renamable display name for an immutable snapshot. It is
-- deliberately NOT part of manifest_json or the snapshot Artifact bytes, so the
-- content hash, snapshot_id and integrity checks never change on rename.
-- NULL means "no name". Names are trimmed, 1..=40 Unicode scalars (SQLite
-- length() counts characters for TEXT), and need not be unique.
ALTER TABLE ink_snapshots ADD COLUMN label TEXT
    CHECK(label IS NULL OR (length(label) BETWEEN 1 AND 40 AND label = trim(label)));
-- Optimistic-concurrency counter for renames (expected_label_revision).
ALTER TABLE ink_snapshots ADD COLUMN label_revision INTEGER NOT NULL DEFAULT 0
    CHECK(label_revision >= 0);

-- Idempotent rename requests: a retried request_id replays its receipt; the
-- same id with a different payload is an idempotency conflict.
CREATE TABLE ink_snapshot_label_requests (
    request_id          TEXT PRIMARY KEY NOT NULL,
    snapshot_id         TEXT NOT NULL REFERENCES ink_snapshots(snapshot_id) ON DELETE RESTRICT,
    request_fingerprint TEXT NOT NULL,
    response_json       TEXT NOT NULL,
    created_at          INTEGER NOT NULL
) STRICT;

PRAGMA user_version=20;
