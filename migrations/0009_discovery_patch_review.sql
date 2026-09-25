-- Executed by the store inside an IMMEDIATE transaction with foreign keys
-- disabled; SQLite can only widen a CHECK constraint by rebuilding the table.
INSERT INTO schema_migrations(version, name) VALUES (9, 'discovery_patch_review');

CREATE TABLE discovery_runs_v9 (
    run_id                 TEXT PRIMARY KEY NOT NULL,
    artifact_id            TEXT NOT NULL REFERENCES artifacts(artifact_id) ON DELETE RESTRICT,
    conversion_id          TEXT,
    pipeline_version       TEXT NOT NULL,
    input_fingerprint      TEXT NOT NULL,
    selection_fingerprint  TEXT NOT NULL,
    unit_set_fingerprint   TEXT,
    result_fingerprint     TEXT,
    status                 TEXT NOT NULL CHECK (
        status IN (
            'running', 'awaiting_patch_review', 'completed', 'partial',
            'needs_attention', 'failed', 'cancelled'
        )
    ),
    admission              TEXT NOT NULL CHECK (
        admission IN ('review_before_publish', 'automatic_candidate')
    ),
    profile_id             TEXT NOT NULL,
    snapshot_json          TEXT NOT NULL CHECK (json_valid(snapshot_json)),
    created_at             TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at             TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

INSERT INTO discovery_runs_v9 (
    run_id, artifact_id, conversion_id, pipeline_version, input_fingerprint,
    selection_fingerprint, unit_set_fingerprint, result_fingerprint, status,
    admission, profile_id, snapshot_json, created_at, updated_at
)
SELECT
    run_id, artifact_id, conversion_id, pipeline_version, input_fingerprint,
    selection_fingerprint, unit_set_fingerprint, result_fingerprint, status,
    admission, profile_id, snapshot_json, created_at, updated_at
FROM discovery_runs;

DROP TABLE discovery_runs;
ALTER TABLE discovery_runs_v9 RENAME TO discovery_runs;

CREATE INDEX discovery_runs_by_source
    ON discovery_runs(artifact_id, input_fingerprint, updated_at);

PRAGMA user_version = 9;
