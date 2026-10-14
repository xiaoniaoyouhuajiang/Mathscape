BEGIN IMMEDIATE;
INSERT INTO schema_migrations(version,name) VALUES(16,'ink_lan_p3');

CREATE TABLE ink_lan_settings (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
    port INTEGER NOT NULL CHECK(port BETWEEN 0 AND 65535)
) STRICT;

CREATE TABLE ink_pairings (
    token_hash TEXT PRIMARY KEY NOT NULL,
    document_id TEXT NOT NULL REFERENCES ink_documents(document_id) ON DELETE RESTRICT,
    expected_head INTEGER NOT NULL CHECK(expected_head>=0),
    expected_epoch INTEGER NOT NULL CHECK(expected_epoch>0),
    session_id TEXT NOT NULL UNIQUE,
    writer_id TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    credential_hash TEXT
) STRICT;

CREATE TABLE ink_lan_devices (
    credential_hash TEXT PRIMARY KEY NOT NULL,
    document_id TEXT NOT NULL REFERENCES ink_documents(document_id) ON DELETE RESTRICT,
    session_id TEXT NOT NULL REFERENCES ink_sessions(session_id) ON DELETE RESTRICT,
    writer_id TEXT NOT NULL,
    writer_epoch INTEGER NOT NULL CHECK(writer_epoch>0),
    device_json TEXT NOT NULL CHECK(json_valid(device_json)),
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER
) STRICT;
CREATE INDEX ink_lan_devices_by_document ON ink_lan_devices(document_id,revoked_at);
CREATE INDEX ink_pairings_by_document ON ink_pairings(document_id,expires_at);

CREATE TABLE ink_writer_transfers (
    request_id TEXT PRIMARY KEY NOT NULL,
    request_fingerprint TEXT NOT NULL,
    document_id TEXT NOT NULL REFERENCES ink_documents(document_id) ON DELETE RESTRICT,
    from_epoch INTEGER NOT NULL,
    through_seq INTEGER NOT NULL,
    session_id TEXT NOT NULL REFERENCES ink_sessions(session_id) ON DELETE RESTRICT,
    to_epoch INTEGER NOT NULL,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE ink_recovery_imports (
    request_id TEXT PRIMARY KEY NOT NULL,
    request_fingerprint TEXT NOT NULL,
    document_id TEXT NOT NULL UNIQUE REFERENCES ink_documents(document_id) ON DELETE RESTRICT,
    original_document_id TEXT NOT NULL,
    imported_at TEXT NOT NULL
) STRICT;

PRAGMA user_version=16;
COMMIT;
