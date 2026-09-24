BEGIN IMMEDIATE;
INSERT INTO schema_migrations(version,name) VALUES(8,'markdown_patches');

CREATE TABLE markdown_patch_sets (
 patch_set_id TEXT PRIMARY KEY NOT NULL,
 run_id TEXT NOT NULL REFERENCES discovery_runs(run_id),
 base_artifact_id TEXT NOT NULL REFERENCES artifacts(artifact_id),
 base_input_fingerprint TEXT NOT NULL,
 patch_fingerprint TEXT NOT NULL UNIQUE,
 status TEXT NOT NULL CHECK(status IN ('proposed','accepted','rejected','stale','applied','failed')),
 base_snapshot_json TEXT NOT NULL CHECK(json_valid(base_snapshot_json)),
 patch_json TEXT NOT NULL CHECK(json_valid(patch_json)),
 patched_artifact_id TEXT REFERENCES artifacts(artifact_id),
 conversion_id TEXT,
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE markdown_patch_operations (
 patch_set_id TEXT NOT NULL REFERENCES markdown_patch_sets(patch_set_id),
 patch_ref TEXT NOT NULL,
 issue_ref TEXT NOT NULL,
 ordinal INTEGER NOT NULL,
 old_start INTEGER NOT NULL,
 old_end INTEGER NOT NULL,
 old_text_hash TEXT NOT NULL,
 model_replacement TEXT NOT NULL,
 draft_replacement TEXT NOT NULL,
 draft_revision INTEGER NOT NULL DEFAULT 0,
 draft_actor TEXT,
 draft_updated_at TEXT,
 status TEXT NOT NULL CHECK(status IN ('proposed','accepted','rejected','stale','applied','failed')),
 operation_json TEXT NOT NULL CHECK(json_valid(operation_json)),
 PRIMARY KEY(patch_set_id, patch_ref),
 UNIQUE(patch_set_id, ordinal)
) STRICT;

CREATE TABLE markdown_patch_decisions (
 decision_id TEXT PRIMARY KEY NOT NULL,
 patch_set_id TEXT NOT NULL REFERENCES markdown_patch_sets(patch_set_id),
 run_id TEXT NOT NULL REFERENCES discovery_runs(run_id),
 actor TEXT NOT NULL,
 decision_kind TEXT NOT NULL CHECK(decision_kind IN ('accept','reject')),
 patch_set_fingerprint TEXT NOT NULL,
 accepted_patch_refs_json TEXT NOT NULL CHECK(json_valid(accepted_patch_refs_json)),
 created_at TEXT NOT NULL
) STRICT;

CREATE TABLE markdown_patch_applications (
 application_id TEXT PRIMARY KEY NOT NULL,
 patch_set_id TEXT NOT NULL REFERENCES markdown_patch_sets(patch_set_id),
 base_artifact_id TEXT NOT NULL REFERENCES artifacts(artifact_id),
 patched_artifact_id TEXT REFERENCES artifacts(artifact_id),
 conversion_id TEXT,
 old_input_fingerprint TEXT NOT NULL,
 new_input_fingerprint TEXT,
 safety_json TEXT NOT NULL CHECK(json_valid(safety_json)),
 status TEXT NOT NULL CHECK(status IN ('proposed','accepted','rejected','stale','applied','failed')),
 error TEXT,
 created_at TEXT NOT NULL
) STRICT;

CREATE INDEX markdown_patch_sets_by_run ON markdown_patch_sets(run_id, updated_at);
CREATE INDEX markdown_patch_operations_by_issue ON markdown_patch_operations(patch_set_id, issue_ref);
CREATE INDEX markdown_patch_decisions_by_run ON markdown_patch_decisions(run_id, created_at);
CREATE INDEX markdown_patch_applications_by_set ON markdown_patch_applications(patch_set_id, created_at);

PRAGMA user_version=8;
COMMIT;
