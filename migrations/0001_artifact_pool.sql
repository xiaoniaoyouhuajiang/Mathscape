BEGIN IMMEDIATE;

CREATE TABLE IF NOT EXISTS schema_migrations (
    version       INTEGER PRIMARY KEY NOT NULL,
    name          TEXT UNIQUE NOT NULL
) STRICT;

INSERT INTO schema_migrations (version, name)
VALUES (1, 'artifact_pool_v1');

CREATE TABLE artifacts (
    artifact_id        TEXT PRIMARY KEY NOT NULL,
    content_hash       TEXT UNIQUE NOT NULL,
    byte_size          INTEGER NOT NULL CHECK (byte_size >= 0),
    current_revision   TEXT NOT NULL
) STRICT;

CREATE TABLE artifact_revisions (
    artifact_id        TEXT NOT NULL REFERENCES artifacts(artifact_id) ON DELETE RESTRICT,
    revision_id        TEXT NOT NULL,
    edition_id         TEXT,
    media_type         TEXT NOT NULL,
    original_filename  TEXT,
    page_count         INTEGER CHECK (page_count IS NULL OR page_count >= 0),
    rights_status      TEXT NOT NULL,
    permitted_uses     TEXT NOT NULL,
    rights_note        TEXT,
    schema_version     TEXT NOT NULL,
    lifecycle          TEXT NOT NULL,
    created_at         TEXT NOT NULL,
    created_by         TEXT NOT NULL,
    generation_run     TEXT,
    PRIMARY KEY (artifact_id, revision_id)
) STRICT;

CREATE TABLE artifact_locations (
    location_id        TEXT PRIMARY KEY NOT NULL,
    artifact_id        TEXT NOT NULL REFERENCES artifacts(artifact_id) ON DELETE RESTRICT,
    storage_kind       TEXT NOT NULL,
    uri                TEXT NOT NULL,
    priority           INTEGER NOT NULL,
    role               TEXT NOT NULL,
    last_verified_at   TEXT,
    current_status     TEXT NOT NULL,
    UNIQUE (artifact_id, uri)
) STRICT;

CREATE INDEX artifact_locations_by_artifact
    ON artifact_locations(artifact_id, priority, location_id);

CREATE TABLE artifact_checks (
    check_id           TEXT PRIMARY KEY NOT NULL,
    location_id        TEXT NOT NULL REFERENCES artifact_locations(location_id) ON DELETE RESTRICT,
    checked_at         TEXT NOT NULL,
    observed_hash      TEXT,
    observed_size      INTEGER CHECK (observed_size IS NULL OR observed_size >= 0),
    result             TEXT NOT NULL,
    checker_version    TEXT NOT NULL
) STRICT;

CREATE INDEX artifact_checks_by_location
    ON artifact_checks(location_id, checked_at, check_id);

CREATE TABLE conversion_runs (
    run_id                    TEXT NOT NULL,
    revision_id               TEXT NOT NULL,
    schema_version            TEXT NOT NULL,
    lifecycle                 TEXT NOT NULL,
    created_at                TEXT NOT NULL,
    created_by                TEXT NOT NULL,
    generation_run            TEXT,
    tool_name                 TEXT NOT NULL,
    tool_version              TEXT,
    command_template          TEXT,
    command_record_status     TEXT NOT NULL,
    page_start                INTEGER,
    page_end                  INTEGER,
    page_index_base           TEXT,
    output_format             TEXT NOT NULL,
    redo_inline_math          INTEGER,
    additional_options        TEXT NOT NULL,
    environment               TEXT,
    started_at                TEXT,
    finished_at               TEXT,
    exit_status               TEXT NOT NULL,
    warnings                  TEXT NOT NULL,
    cost                      TEXT,
    PRIMARY KEY (run_id, revision_id)
) STRICT;

CREATE TABLE conversion_endpoints (
    run_id             TEXT NOT NULL,
    revision_id        TEXT NOT NULL,
    role                TEXT NOT NULL,
    ordinal             INTEGER NOT NULL,
    artifact_id         TEXT NOT NULL REFERENCES artifacts(artifact_id) ON DELETE RESTRICT,
    PRIMARY KEY (run_id, revision_id, role, ordinal),
    FOREIGN KEY (run_id, revision_id)
        REFERENCES conversion_runs(run_id, revision_id) ON DELETE CASCADE
) STRICT;

PRAGMA user_version = 1;

COMMIT;
