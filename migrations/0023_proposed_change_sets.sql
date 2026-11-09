BEGIN IMMEDIATE;

INSERT INTO schema_migrations(version, name)
VALUES (23, 'proposed_change_sets_v1');

CREATE TABLE proposed_change_sets (
    change_set_id    TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT NOT NULL,
    active_version   INTEGER NOT NULL CHECK (active_version > 0),
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
) STRICT;

CREATE TABLE proposed_change_set_versions (
    change_set_id        TEXT NOT NULL REFERENCES proposed_change_sets(change_set_id) ON DELETE CASCADE,
    version              INTEGER NOT NULL CHECK (version > 0),
    investigation_id     TEXT NOT NULL,
    round_id             TEXT NOT NULL,
    generation           TEXT NOT NULL,
    request_id           TEXT NOT NULL,
    request_fingerprint  TEXT NOT NULL,
    proposal_json        TEXT NOT NULL CHECK (json_valid(proposal_json)),
    result_json          TEXT NOT NULL CHECK (json_valid(result_json)),
    state                TEXT NOT NULL CHECK (state IN ('staged', 'blocked', 'accepted', 'discarded', 'superseded', 'withdrawn')),
    base_commit          TEXT,
    created_at           TEXT NOT NULL,
    PRIMARY KEY (change_set_id, version),
    UNIQUE (investigation_id, request_id),
    FOREIGN KEY (change_set_id) REFERENCES proposed_change_sets(change_set_id) ON DELETE CASCADE
) STRICT;

CREATE INDEX proposed_change_sets_by_investigation
    ON proposed_change_sets(investigation_id, updated_at, change_set_id);

CREATE INDEX proposed_change_set_versions_by_investigation
    ON proposed_change_set_versions(investigation_id, change_set_id, version);

PRAGMA user_version = 23;

COMMIT;
