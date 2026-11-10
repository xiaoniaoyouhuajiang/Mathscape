BEGIN IMMEDIATE;

INSERT INTO schema_migrations(version, name)
VALUES (24, 'phase2_pr_b_dependencies_v1');

ALTER TABLE proposed_change_set_versions
    ADD COLUMN proposed_by TEXT NOT NULL DEFAULT 'agent:custom';

CREATE TABLE change_set_dependencies (
    dependent_change_set_id TEXT NOT NULL,
    dependent_version       INTEGER NOT NULL CHECK (dependent_version > 0),
    depends_on_change_set_id TEXT NOT NULL,
    depends_on_version      INTEGER NOT NULL CHECK (depends_on_version > 0),
    investigation_id        TEXT NOT NULL,
    referenced_ids_json     TEXT NOT NULL CHECK (json_valid(referenced_ids_json)),
    PRIMARY KEY (dependent_change_set_id, dependent_version,
                 depends_on_change_set_id, depends_on_version),
    FOREIGN KEY (dependent_change_set_id, dependent_version)
        REFERENCES proposed_change_set_versions(change_set_id, version)
        ON DELETE CASCADE
);

CREATE INDEX change_set_dependencies_by_investigation
    ON change_set_dependencies(investigation_id, dependent_change_set_id, dependent_version);

CREATE TABLE change_set_decisions (
    decision_id       TEXT PRIMARY KEY NOT NULL,
    change_set_id     TEXT NOT NULL,
    version           INTEGER NOT NULL CHECK (version > 0),
    investigation_id  TEXT NOT NULL,
    kind              TEXT NOT NULL CHECK (kind IN ('accepted', 'discarded', 'withdrawn')),
    decided_by        TEXT NOT NULL,
    plan_fingerprint  TEXT,
    commit_id         TEXT,
    receipt_json      TEXT CHECK (receipt_json IS NULL OR json_valid(receipt_json)),
    batch_id          TEXT,
    reason            TEXT,
    decided_at        TEXT NOT NULL,
    FOREIGN KEY (change_set_id, version)
        REFERENCES proposed_change_set_versions(change_set_id, version)
        ON DELETE CASCADE
);

CREATE INDEX change_set_decisions_by_investigation
    ON change_set_decisions(investigation_id, decided_at, decision_id);

-- A round is one execution authorization; an investigation is the durable
-- proposal visibility scope that several rounds may share. The adapter and
-- the MCP server both bind (or verify) a round here, first writer wins.
CREATE TABLE investigation_rounds (
    round_id         TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT NOT NULL,
    bound_by         TEXT NOT NULL,
    bound_at         TEXT NOT NULL
);

CREATE INDEX investigation_rounds_by_investigation
    ON investigation_rounds(investigation_id, bound_at);

PRAGMA user_version = 24;

COMMIT;
