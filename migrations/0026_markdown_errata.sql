BEGIN IMMEDIATE;

INSERT INTO schema_migrations(version, name)
VALUES (26, 'markdown_block_errata_v1');

-- Append-only block/range errata overlay over immutable physical Markdown snapshots.
-- Deliberately isolated from `map_objects` and Artifact bytes so user corrections
-- never cause byte-offset drift, never detach Ink links or SourceSpans, and remain
-- fully auditable and reversible.
CREATE TABLE markdown_block_errata (
    errata_id              TEXT PRIMARY KEY NOT NULL,
    artifact_id            TEXT NOT NULL,
    base_content_hash      TEXT NOT NULL,
    first_block_id         TEXT NOT NULL,
    last_block_id          TEXT NOT NULL,
    covered_block_ids_json TEXT NOT NULL,
    base_byte_start        INTEGER NOT NULL CHECK (base_byte_start >= 0),
    base_byte_end          INTEGER NOT NULL CHECK (base_byte_end > base_byte_start),
    base_text_hash         TEXT NOT NULL,
    original_text          TEXT NOT NULL,
    corrected_text         TEXT NOT NULL,
    status                 TEXT NOT NULL CHECK (status IN ('active', 'reverted', 'superseded')),
    source_kind            TEXT NOT NULL CHECK (source_kind IN ('manual', 'discovery_patch')),
    actor                  TEXT NOT NULL,
    note                   TEXT,
    verified_pdf_page      INTEGER,
    discovery_run_id       TEXT,
    patch_set_id           TEXT,
    superseded_by          TEXT,
    created_at             TEXT NOT NULL,
    updated_at             TEXT NOT NULL
);

CREATE INDEX idx_markdown_errata_artifact_status
    ON markdown_block_errata(artifact_id, status, base_byte_start);

-- Non-blocking review acknowledgments for registered SourceSemanticUnits anchored
-- on blocks affected by active errata. Stored outside `map_objects` so acknowledging
-- does not create map commits.
CREATE TABLE markdown_errata_unit_reviews (
    unit_id     TEXT NOT NULL,
    errata_id   TEXT NOT NULL,
    decision    TEXT NOT NULL CHECK (decision IN ('acknowledged', 'sent_to_sandbox')),
    reviewed_at TEXT NOT NULL,
    PRIMARY KEY (unit_id, errata_id)
);

CREATE INDEX idx_markdown_errata_unit_reviews_errata
    ON markdown_errata_unit_reviews(errata_id);

PRAGMA user_version = 26;

COMMIT;
