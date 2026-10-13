BEGIN IMMEDIATE;

INSERT INTO schema_migrations (version, name)
VALUES (14, 'ink_snapshots_p2');

CREATE TABLE ink_snapshots (
    snapshot_id       TEXT PRIMARY KEY NOT NULL,
    request_id        TEXT NOT NULL UNIQUE,
    document_id       TEXT NOT NULL REFERENCES ink_documents(document_id) ON DELETE RESTRICT,
    through_seq       INTEGER NOT NULL CHECK (through_seq >= 0),
    artifact_id       TEXT NOT NULL REFERENCES artifacts(artifact_id) ON DELETE RESTRICT,
    content_hash      TEXT NOT NULL,
    byte_size         INTEGER NOT NULL CHECK (byte_size >= 0),
    manifest_json     TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    UNIQUE (document_id, through_seq, artifact_id)
) STRICT;

CREATE INDEX ink_snapshots_by_document
    ON ink_snapshots(document_id, through_seq, created_at, snapshot_id);

PRAGMA user_version = 14;

COMMIT;

