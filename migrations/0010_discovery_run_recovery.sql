-- Persisted user disposition for a completed discovery result. The workflow
-- status remains in discovery_runs; this table records an explicit decision.
INSERT INTO schema_migrations(version, name) VALUES (10, 'discovery_run_recovery');

CREATE TABLE discovery_run_dispositions (
    run_id       TEXT PRIMARY KEY NOT NULL REFERENCES discovery_runs(run_id) ON DELETE RESTRICT,
    disposition  TEXT NOT NULL CHECK (disposition IN ('discarded')),
    reason       TEXT NOT NULL,
    actor        TEXT NOT NULL,
    request_id   TEXT NOT NULL UNIQUE,
    decided_at   TEXT NOT NULL
) STRICT;

CREATE INDEX discovery_run_dispositions_by_decision
    ON discovery_run_dispositions(request_id);

PRAGMA user_version = 10;
