BEGIN IMMEDIATE;

INSERT INTO schema_migrations (version, name)
VALUES (5, 'source_map_v1');

CREATE TABLE map_objects (
    object_id         TEXT PRIMARY KEY NOT NULL,
    object_shape      TEXT NOT NULL CHECK (object_shape IN ('entity', 'relation')),
    entity_kind       TEXT NOT NULL,
    current_revision  TEXT NOT NULL
) STRICT;

CREATE INDEX map_objects_by_kind
    ON map_objects(entity_kind, object_id);

CREATE TABLE map_object_revisions (
    object_id       TEXT NOT NULL REFERENCES map_objects(object_id) ON DELETE RESTRICT,
    revision_id     TEXT NOT NULL,
    schema_version  TEXT NOT NULL,
    lifecycle       TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    created_by      TEXT NOT NULL,
    generation_run  TEXT,
    payload_json    TEXT NOT NULL CHECK (json_valid(payload_json)),
    PRIMARY KEY (object_id, revision_id)
) STRICT;

CREATE TABLE map_relation_endpoints (
    relation_id  TEXT NOT NULL,
    revision_id  TEXT NOT NULL,
    role         TEXT NOT NULL,
    ordinal_key  INTEGER NOT NULL,
    ordinal      INTEGER,
    entity_id    TEXT NOT NULL REFERENCES map_objects(object_id) ON DELETE RESTRICT,
    PRIMARY KEY (relation_id, revision_id, role, ordinal_key),
    FOREIGN KEY (relation_id, revision_id)
        REFERENCES map_object_revisions(object_id, revision_id) ON DELETE CASCADE
) STRICT;

CREATE INDEX map_relation_endpoints_by_entity
    ON map_relation_endpoints(entity_id, relation_id, role);

CREATE TABLE map_commits (
    commit_id       TEXT PRIMARY KEY NOT NULL,
    author          TEXT NOT NULL,
    generation_run  TEXT,
    message         TEXT,
    committed_at    TEXT NOT NULL
) STRICT;

CREATE TABLE map_commit_objects (
    commit_id    TEXT NOT NULL REFERENCES map_commits(commit_id) ON DELETE RESTRICT,
    ordinal      INTEGER NOT NULL CHECK (ordinal > 0),
    object_id    TEXT NOT NULL REFERENCES map_objects(object_id) ON DELETE RESTRICT,
    revision_id  TEXT NOT NULL,
    outcome      TEXT NOT NULL CHECK (outcome IN ('created', 'reused')),
    PRIMARY KEY (commit_id, ordinal),
    UNIQUE (commit_id, object_id),
    FOREIGN KEY (object_id, revision_id)
        REFERENCES map_object_revisions(object_id, revision_id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE source_registration_commits (
    decision_id       TEXT PRIMARY KEY NOT NULL
                      REFERENCES discovery_acceptance_decisions(decision_id) ON DELETE RESTRICT,
    plan_fingerprint  TEXT NOT NULL UNIQUE,
    commit_id         TEXT NOT NULL UNIQUE REFERENCES map_commits(commit_id) ON DELETE RESTRICT,
    receipt_json      TEXT NOT NULL CHECK (json_valid(receipt_json)),
    committed_at      TEXT NOT NULL
) STRICT;

PRAGMA user_version = 5;

COMMIT;
