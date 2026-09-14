-- This reconciliation is executed inside the store's migration transaction.
-- The result_fingerprint column is added conditionally by Rust because SQLite
-- does not support ADD COLUMN IF NOT EXISTS. Every statement below is safe to
-- repeat for development databases that ran an earlier draft of migration 6.
CREATE TABLE IF NOT EXISTS discovery_acceptance_decisions (
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

CREATE INDEX IF NOT EXISTS discovery_acceptance_by_run
    ON discovery_acceptance_decisions(run_id, decided_at, decision_id);
