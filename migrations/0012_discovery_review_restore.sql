BEGIN IMMEDIATE;
INSERT INTO schema_migrations(version,name) VALUES(12,'discovery_review_restore');

CREATE TABLE discovery_run_disposition_events (
    request_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES discovery_runs(run_id) ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK(disposition IN ('discarded','restored')),
    previous_request_id TEXT REFERENCES discovery_run_disposition_events(request_id) ON DELETE RESTRICT,
    result_fingerprint TEXT NOT NULL,
    actor TEXT NOT NULL,
    reason TEXT NOT NULL,
    decided_at TEXT NOT NULL
) STRICT;
CREATE INDEX discovery_disposition_events_by_run ON discovery_run_disposition_events(run_id,decided_at,request_id);
-- Preserve every existing discard as the first event of its run.
INSERT INTO discovery_run_disposition_events
    (request_id,run_id,disposition,previous_request_id,result_fingerprint,actor,reason,decided_at)
SELECT x.request_id,x.run_id,x.disposition,NULL,d.result_fingerprint,x.actor,x.reason,x.decided_at
FROM discovery_run_dispositions x JOIN discovery_runs d ON d.run_id=x.run_id;

ALTER TABLE discovery_run_dispositions RENAME TO discovery_run_dispositions_v1;
CREATE TABLE discovery_run_dispositions (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES discovery_runs(run_id) ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK(disposition IN ('discarded','restored')),
    reason TEXT NOT NULL,
    actor TEXT NOT NULL,
    request_id TEXT NOT NULL UNIQUE REFERENCES discovery_run_disposition_events(request_id) ON DELETE RESTRICT,
    decided_at TEXT NOT NULL
) STRICT;
INSERT INTO discovery_run_dispositions SELECT * FROM discovery_run_dispositions_v1;
DROP TABLE discovery_run_dispositions_v1;
CREATE INDEX discovery_run_dispositions_by_decision ON discovery_run_dispositions(request_id);
PRAGMA user_version=12;
COMMIT;
