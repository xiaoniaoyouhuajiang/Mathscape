BEGIN IMMEDIATE;

INSERT INTO schema_migrations (version, name)
VALUES (4, 'discovery_runs_v1');

CREATE TABLE discovery_runs (
    run_id                 TEXT PRIMARY KEY NOT NULL,
    artifact_id            TEXT NOT NULL REFERENCES artifacts(artifact_id) ON DELETE RESTRICT,
    conversion_id          TEXT,
    pipeline_version       TEXT NOT NULL,
    input_fingerprint      TEXT NOT NULL,
    selection_fingerprint  TEXT NOT NULL,
    unit_set_fingerprint   TEXT,
    result_fingerprint     TEXT,
    status                 TEXT NOT NULL CHECK (
        status IN ('running', 'completed', 'partial', 'needs_attention', 'failed', 'cancelled')
    ),
    admission              TEXT NOT NULL CHECK (
        admission IN ('review_before_publish', 'automatic_candidate')
    ),
    profile_id             TEXT NOT NULL,
    snapshot_json          TEXT NOT NULL CHECK (json_valid(snapshot_json)),
    created_at             TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at             TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX discovery_runs_by_source
    ON discovery_runs(artifact_id, input_fingerprint, updated_at);

CREATE TABLE discovery_attempts (
    run_id             TEXT NOT NULL REFERENCES discovery_runs(run_id) ON DELETE CASCADE,
    ordinal            INTEGER NOT NULL CHECK (ordinal > 0),
    stage              TEXT NOT NULL,
    batch_fingerprint  TEXT,
    repair             INTEGER NOT NULL CHECK (repair IN (0, 1)),
    status             TEXT NOT NULL,
    request_json       TEXT NOT NULL CHECK (json_valid(request_json)),
    raw_text           TEXT,
    run_json           TEXT NOT NULL CHECK (json_valid(run_json)),
    error              TEXT,
    PRIMARY KEY (run_id, ordinal)
) STRICT;

CREATE TABLE discovery_validation_reports (
    run_id          TEXT NOT NULL REFERENCES discovery_runs(run_id) ON DELETE CASCADE,
    report_ordinal  INTEGER NOT NULL CHECK (report_ordinal > 0),
    report_json     TEXT NOT NULL CHECK (json_valid(report_json)),
    PRIMARY KEY (run_id, report_ordinal)
) STRICT;

CREATE TABLE discovery_diagnostics (
    run_id              TEXT NOT NULL,
    report_ordinal      INTEGER NOT NULL,
    diagnostic_ordinal  INTEGER NOT NULL CHECK (diagnostic_ordinal > 0),
    code                TEXT NOT NULL,
    path                TEXT NOT NULL,
    message             TEXT NOT NULL,
    checked             INTEGER NOT NULL CHECK (checked IN (0, 1)),
    PRIMARY KEY (run_id, report_ordinal, diagnostic_ordinal),
    FOREIGN KEY (run_id, report_ordinal)
        REFERENCES discovery_validation_reports(run_id, report_ordinal) ON DELETE CASCADE
) STRICT;

CREATE TABLE discovery_units (
    run_id                 TEXT NOT NULL REFERENCES discovery_runs(run_id) ON DELETE CASCADE,
    unit_ref               TEXT NOT NULL,
    parent_ref             TEXT,
    unit_kind              TEXT NOT NULL,
    completeness           TEXT NOT NULL,
    unit_set_fingerprint   TEXT NOT NULL,
    payload_json           TEXT NOT NULL CHECK (json_valid(payload_json)),
    PRIMARY KEY (run_id, unit_ref)
) STRICT;

CREATE TABLE discovery_fact_tasks (
    run_id                  TEXT NOT NULL REFERENCES discovery_runs(run_id) ON DELETE CASCADE,
    task_ref                TEXT NOT NULL,
    owner_unit_ref          TEXT NOT NULL,
    processed               INTEGER NOT NULL CHECK (processed IN (0, 1)),
    requires_unit_revision  INTEGER NOT NULL CHECK (requires_unit_revision IN (0, 1)),
    payload_json            TEXT NOT NULL CHECK (json_valid(payload_json)),
    PRIMARY KEY (run_id, task_ref),
    FOREIGN KEY (run_id, owner_unit_ref)
        REFERENCES discovery_units(run_id, unit_ref) ON DELETE CASCADE
) STRICT;

CREATE TABLE discovery_pending_tasks (
    run_id          TEXT NOT NULL REFERENCES discovery_runs(run_id) ON DELETE CASCADE,
    task_ref        TEXT NOT NULL,
    owner_unit_ref  TEXT,
    PRIMARY KEY (run_id, task_ref),
    FOREIGN KEY (run_id, owner_unit_ref)
        REFERENCES discovery_units(run_id, unit_ref) ON DELETE CASCADE
) STRICT;

CREATE TABLE discovery_acceptance_decisions (
    decision_id                  TEXT PRIMARY KEY NOT NULL,
    run_id                       TEXT NOT NULL REFERENCES discovery_runs(run_id) ON DELETE RESTRICT,
    actor                        TEXT NOT NULL,
    decided_at                   TEXT NOT NULL,
    authority                    TEXT NOT NULL CHECK (authority IN ('human_review', 'automatic_policy')),
    expected_result_fingerprint  TEXT NOT NULL,
    selection_json               TEXT NOT NULL CHECK (json_valid(selection_json)),
    plan_fingerprint             TEXT NOT NULL UNIQUE,
    plan_json                    TEXT NOT NULL CHECK (json_valid(plan_json)),
    created_at                   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX discovery_acceptance_by_run
    ON discovery_acceptance_decisions(run_id, decided_at, decision_id);

PRAGMA user_version = 4;

COMMIT;
