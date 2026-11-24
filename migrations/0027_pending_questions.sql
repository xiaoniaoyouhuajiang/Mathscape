BEGIN IMMEDIATE;

INSERT INTO schema_migrations(version, name)
VALUES (27, 'pending_questions_v1');

-- Phase2 `user.ask` pending user clarification questions (docs/phase2-write-contract.md §4.1).
-- Scoped to `investigation_id` (e.g. `sandbox:<session_id>`), non-blocking (`end_turn`),
-- answered or cancelled exclusively by Desktop user actions.
CREATE TABLE pending_questions (
    question_id      TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT NOT NULL,
    round_id         TEXT NOT NULL,
    origin           TEXT NOT NULL CHECK (origin IN ('mcp_user_ask', 'harness_native')),
    harness_ref_json TEXT,
    question         TEXT NOT NULL,
    options_json     TEXT,
    ink_requested    INTEGER NOT NULL DEFAULT 0 CHECK (ink_requested IN (0, 1)),
    state            TEXT NOT NULL CHECK (state IN ('pending', 'answered', 'cancelled')),
    answer_json      TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);

CREATE INDEX idx_pending_questions_investigation_state
    ON pending_questions(investigation_id, state, created_at);

CREATE INDEX idx_pending_questions_round
    ON pending_questions(round_id);

PRAGMA user_version = 27;

COMMIT;
