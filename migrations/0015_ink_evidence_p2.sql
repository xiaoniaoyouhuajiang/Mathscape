BEGIN IMMEDIATE;

INSERT INTO schema_migrations (version, name) VALUES (15, 'ink_evidence_p2');

-- Freeze the authoritative prefix before Artifact Pool import. An interrupted
-- request resumes this reservation even when editing has advanced the head.
CREATE TABLE ink_snapshot_requests (
    request_id TEXT PRIMARY KEY NOT NULL,
    document_id TEXT NOT NULL REFERENCES ink_documents(document_id) ON DELETE RESTRICT,
    through_seq INTEGER NOT NULL CHECK (through_seq >= 0),
    manifest_json TEXT CHECK (manifest_json IS NULL OR json_valid(manifest_json)),
    created_at TEXT NOT NULL,
    snapshot_id TEXT REFERENCES ink_snapshots(snapshot_id) ON DELETE RESTRICT,
    CHECK ((snapshot_id IS NULL AND manifest_json IS NOT NULL)
        OR (snapshot_id IS NOT NULL AND manifest_json IS NULL))
) STRICT;

INSERT INTO ink_snapshot_requests
    (request_id, document_id, through_seq, manifest_json, created_at, snapshot_id)
SELECT request_id, document_id, through_seq, NULL, created_at, snapshot_id
FROM ink_snapshots;

CREATE TABLE ink_evidence_requests (
    request_id TEXT PRIMARY KEY NOT NULL,
    request_fingerprint TEXT NOT NULL,
    snapshot_id TEXT NOT NULL REFERENCES ink_snapshots(snapshot_id) ON DELETE RESTRICT,
    span_id TEXT NOT NULL REFERENCES map_objects(object_id) ON DELETE RESTRICT,
    commit_id TEXT NOT NULL UNIQUE REFERENCES map_commits(commit_id) ON DELETE RESTRICT,
    receipt_json TEXT NOT NULL CHECK (json_valid(receipt_json))
) STRICT;

CREATE INDEX ink_evidence_by_snapshot ON ink_evidence_requests(snapshot_id, span_id);

PRAGMA user_version = 15;
COMMIT;
