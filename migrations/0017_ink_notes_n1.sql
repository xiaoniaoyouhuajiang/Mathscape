-- Applied inside one Rust transaction together with the row backfill in
-- `ink_note::migrate_v17`; the file therefore has no BEGIN/COMMIT.
INSERT INTO schema_migrations (version, name) VALUES (17, 'ink_notes_n1');

-- Note metadata lives beside the document row (D1). Every INSERT path writes
-- these columns explicitly; the defaults only describe pre-existing rows until
-- the backfill below replaces them.
ALTER TABLE ink_documents ADD COLUMN title_mode TEXT NOT NULL DEFAULT 'manual'
    CHECK (title_mode IN ('auto', 'manual'));
ALTER TABLE ink_documents ADD COLUMN title_revision INTEGER NOT NULL DEFAULT 0
    CHECK (title_revision >= 0);
ALTER TABLE ink_documents ADD COLUMN lifecycle TEXT NOT NULL DEFAULT 'active'
    CHECK (lifecycle IN ('active', 'trashed'));
ALTER TABLE ink_documents ADD COLUMN lifecycle_revision INTEGER NOT NULL DEFAULT 0
    CHECK (lifecycle_revision >= 0);
-- Unix milliseconds. Present exactly when the document is in the trash.
ALTER TABLE ink_documents ADD COLUMN trashed_at INTEGER
    CHECK ((lifecycle = 'trashed') = (trashed_at IS NOT NULL));
-- Unix milliseconds of the last content edit (D9).
ALTER TABLE ink_documents ADD COLUMN last_used_at INTEGER NOT NULL DEFAULT 0;

CREATE INDEX ink_documents_by_last_used
    ON ink_documents(lifecycle, last_used_at DESC, document_id);
CREATE INDEX ink_documents_by_trashed
    ON ink_documents(lifecycle, trashed_at DESC, document_id);

-- Idempotency for note-level writes. Replays return response_json verbatim
-- without re-checking the current state.
CREATE TABLE ink_note_requests (
    request_id          TEXT PRIMARY KEY NOT NULL,
    kind                TEXT NOT NULL CHECK (kind IN ('create', 'rename', 'trash', 'restore', 'clean_blank')),
    document_id         TEXT,
    request_fingerprint TEXT NOT NULL,
    response_json       TEXT NOT NULL CHECK (json_valid(response_json)),
    created_at          INTEGER NOT NULL
) STRICT;

CREATE INDEX ink_note_requests_by_document ON ink_note_requests(document_id, created_at);

PRAGMA user_version = 17;
