BEGIN IMMEDIATE;
INSERT INTO schema_migrations(version,name) VALUES(11,'registration_reversal');

-- Broaden the membership outcome without rewriting historical registrations.
ALTER TABLE map_commit_objects RENAME TO map_commit_objects_v1;
CREATE TABLE map_commit_objects (
    commit_id TEXT NOT NULL REFERENCES map_commits(commit_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK(ordinal > 0),
    object_id TEXT NOT NULL REFERENCES map_objects(object_id) ON DELETE RESTRICT,
    revision_id TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK(outcome IN ('created','reused','reactivated','revised')),
    PRIMARY KEY(commit_id,ordinal), UNIQUE(commit_id,object_id),
    FOREIGN KEY(object_id,revision_id) REFERENCES map_object_revisions(object_id,revision_id) ON DELETE RESTRICT
) STRICT;
INSERT INTO map_commit_objects SELECT * FROM map_commit_objects_v1;
DROP TABLE map_commit_objects_v1;
CREATE INDEX map_commit_objects_by_object ON map_commit_objects(object_id,commit_id);

CREATE TABLE discovery_registration_reversals (
    request_id TEXT PRIMARY KEY NOT NULL,
    decision_id TEXT NOT NULL UNIQUE REFERENCES source_registration_commits(decision_id) ON DELETE RESTRICT,
    registration_commit_id TEXT NOT NULL REFERENCES map_commits(commit_id) ON DELETE RESTRICT,
    base_commit TEXT REFERENCES map_commits(commit_id) ON DELETE RESTRICT,
    plan_fingerprint TEXT NOT NULL,
    request_json TEXT NOT NULL CHECK(json_valid(request_json)),
    map_commit_id TEXT REFERENCES map_commits(commit_id) ON DELETE RESTRICT,
    receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json)),
    decided_at TEXT NOT NULL
) STRICT;
CREATE TABLE source_registration_requests (
    request_id TEXT PRIMARY KEY NOT NULL,
    decision_id TEXT NOT NULL REFERENCES source_registration_commits(decision_id) ON DELETE RESTRICT,
    request_json TEXT NOT NULL CHECK(json_valid(request_json)),
    plan_fingerprint TEXT NOT NULL,
    receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json))
) STRICT;
PRAGMA user_version=11;
COMMIT;
