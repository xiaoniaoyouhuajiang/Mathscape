BEGIN IMMEDIATE;
INSERT INTO schema_migrations(version,name) VALUES(7,'map_edits');
CREATE TABLE map_change_requests (
 request_id TEXT PRIMARY KEY NOT NULL,
 request_json TEXT NOT NULL CHECK(json_valid(request_json)),
 plan_fingerprint TEXT NOT NULL,
 commit_id TEXT NOT NULL UNIQUE REFERENCES map_commits(commit_id),
 receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json))
) STRICT;
CREATE TABLE map_edit_operations (
 commit_id TEXT NOT NULL REFERENCES map_commits(commit_id),
 object_id TEXT NOT NULL REFERENCES map_objects(object_id),
 previous_revision TEXT,
 current_revision TEXT NOT NULL,
 action TEXT NOT NULL CHECK(action IN ('create','revise','retire')),
 reason TEXT NOT NULL,
 PRIMARY KEY(commit_id,object_id),
 FOREIGN KEY(object_id,current_revision) REFERENCES map_object_revisions(object_id,revision_id)
) STRICT;
PRAGMA user_version=7;
COMMIT;
