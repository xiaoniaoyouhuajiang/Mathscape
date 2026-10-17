-- Applied inside one Rust transaction by `ink_link::migrate_v18`; the file
-- therefore has no BEGIN/COMMIT.
INSERT INTO schema_migrations (version, name) VALUES (18, 'ink_note_links_n2');

-- STRICT CHECK constraints cannot be altered in place, so the request table is
-- rebuilt with the N2 kinds. Existing rows keep their replay semantics.
CREATE TABLE ink_note_requests_v18 (
    request_id          TEXT PRIMARY KEY NOT NULL,
    kind                TEXT NOT NULL CHECK (kind IN (
        'create', 'rename', 'trash', 'restore', 'clean_blank',
        'create_from_context', 'link', 'unlink', 'set_primary')),
    document_id         TEXT,
    request_fingerprint TEXT NOT NULL,
    response_json       TEXT NOT NULL CHECK (json_valid(response_json)),
    created_at          INTEGER NOT NULL
) STRICT;
INSERT INTO ink_note_requests_v18
    (request_id, kind, document_id, request_fingerprint, response_json, created_at)
    SELECT request_id, kind, document_id, request_fingerprint, response_json, created_at
    FROM ink_note_requests;
DROP TABLE ink_note_requests;
ALTER TABLE ink_note_requests_v18 RENAME TO ink_note_requests;
CREATE INDEX ink_note_requests_by_document ON ink_note_requests(document_id, created_at);

-- NoteLink (§4.2, §14.2): a note's relation to a source object, a Markdown
-- block range or another note. Links are note metadata; they never enter the
-- source graph and never create source revisions.
CREATE TABLE ink_note_links (
    link_id         TEXT PRIMARY KEY NOT NULL,
    document_id     TEXT NOT NULL REFERENCES ink_documents(document_id),
    target_kind     TEXT NOT NULL CHECK (target_kind IN ('source_object', 'markdown', 'ink_note')),
    -- Canonical identity: src:<stable_id>, md:<artifact>:<first>:<last>, note:<document_id>.
    target_key      TEXT NOT NULL,
    -- Object stable id, Markdown artifact id or note document id.
    target_id       TEXT NOT NULL,
    target_json     TEXT NOT NULL CHECK (json_valid(target_json)),
    -- Source revision or Markdown content hash at link time.
    target_revision TEXT,
    -- Short name fixed at link time (§5.2).
    target_label    TEXT NOT NULL,
    role            TEXT NOT NULL CHECK (role IN ('attempt', 'alt_proof', 'check', 'study', 'question', 'reference')),
    is_primary      INTEGER NOT NULL CHECK (is_primary IN (0, 1)),
    state           TEXT NOT NULL CHECK (state IN ('active', 'removed')),
    created_at      INTEGER NOT NULL,
    removed_at      INTEGER,
    CHECK ((state = 'removed') = (removed_at IS NOT NULL)),
    CHECK (state = 'active' OR is_primary = 0)
) STRICT;

CREATE UNIQUE INDEX ink_note_links_active_unique
    ON ink_note_links(document_id, target_key, role) WHERE state = 'active';
CREATE UNIQUE INDEX ink_note_links_one_primary
    ON ink_note_links(document_id) WHERE state = 'active' AND is_primary = 1;
CREATE INDEX ink_note_links_by_target ON ink_note_links(target_kind, target_id, state);
CREATE INDEX ink_note_links_by_note ON ink_note_links(document_id, state, created_at);

PRAGMA user_version = 18;
