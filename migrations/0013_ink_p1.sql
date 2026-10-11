BEGIN IMMEDIATE;

INSERT INTO schema_migrations (version, name)
VALUES (13, 'ink_p1');

CREATE TABLE ink_documents (
    document_id         TEXT PRIMARY KEY NOT NULL,
    version             TEXT NOT NULL,
    title               TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    exported_at         TEXT NOT NULL,
    pages_json          TEXT NOT NULL,
    capture_segments_json TEXT NOT NULL,
    head_seq            INTEGER NOT NULL CHECK (head_seq >= 0),
    updated_at          TEXT NOT NULL
) STRICT;

CREATE TABLE ink_sessions (
    session_id          TEXT PRIMARY KEY NOT NULL,
    document_id         TEXT NOT NULL REFERENCES ink_documents(document_id) ON DELETE CASCADE,
    writer_id            TEXT NOT NULL,
    writer_epoch         INTEGER NOT NULL CHECK (writer_epoch > 0),
    status               TEXT NOT NULL CHECK (status IN ('active', 'closed', 'recovery_required')),
    device_json          TEXT NOT NULL,
    started_at           TEXT NOT NULL,
    last_seen_at         TEXT NOT NULL
) STRICT;

CREATE INDEX ink_sessions_by_document
    ON ink_sessions(document_id, status, last_seen_at, session_id);

CREATE TABLE ink_operations (
    document_id         TEXT NOT NULL REFERENCES ink_documents(document_id) ON DELETE CASCADE,
    seq                 INTEGER NOT NULL CHECK (seq > 0),
    command_id          TEXT NOT NULL,
    operation_json      TEXT NOT NULL,
    operation_fingerprint TEXT NOT NULL,
    recorded_at         TEXT NOT NULL,
    PRIMARY KEY (document_id, seq),
    UNIQUE (document_id, command_id)
) STRICT;

CREATE INDEX ink_operations_by_command
    ON ink_operations(document_id, command_id);

CREATE TABLE ink_writer_state (
    document_id         TEXT PRIMARY KEY NOT NULL REFERENCES ink_documents(document_id) ON DELETE CASCADE,
    writer_epoch        INTEGER NOT NULL CHECK (writer_epoch > 0),
    active_session_id   TEXT REFERENCES ink_sessions(session_id) ON DELETE RESTRICT,
    writer_id            TEXT,
    head_seq            INTEGER NOT NULL CHECK (head_seq >= 0),
    updated_at          TEXT NOT NULL
) STRICT;

CREATE TABLE ink_requests (
    request_id          TEXT PRIMARY KEY NOT NULL,
    document_id         TEXT NOT NULL REFERENCES ink_documents(document_id) ON DELETE CASCADE,
    request_fingerprint TEXT NOT NULL,
    document_seq        INTEGER NOT NULL CHECK (document_seq > 0),
    response_json       TEXT NOT NULL,
    created_at          TEXT NOT NULL
) STRICT;

CREATE INDEX ink_requests_by_document
    ON ink_requests(document_id, created_at, request_id);

PRAGMA user_version = 13;

COMMIT;

