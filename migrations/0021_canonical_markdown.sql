BEGIN IMMEDIATE;

INSERT INTO schema_migrations (version, name)
VALUES (21, 'canonical_markdown_v1');

-- Logical canonical markdown. One row per (PDF, parser profile, policy).
-- The head pointer names an immutable Markdown artifact; nothing here rewrites
-- artifact bytes. Coverage pages are closed and 1-based (page_index_base one).
CREATE TABLE canonical_markdown (
    canonical_id            TEXT PRIMARY KEY NOT NULL,
    source_pdf_artifact_id  TEXT NOT NULL,
    parser_profile          TEXT NOT NULL,
    parser_policy_version   TEXT NOT NULL,
    head_artifact_id        TEXT,
    head_revision           TEXT,
    total_pages             INTEGER CHECK (total_pages IS NULL OR total_pages >= 0),
    created_at              TEXT NOT NULL,
    updated_at              TEXT NOT NULL,
    UNIQUE (source_pdf_artifact_id, parser_profile, parser_policy_version)
) STRICT;

CREATE INDEX canonical_markdown_by_pdf
    ON canonical_markdown(source_pdf_artifact_id);

CREATE TABLE canonical_markdown_coverage (
    canonical_id           TEXT NOT NULL
                           REFERENCES canonical_markdown(canonical_id) ON DELETE CASCADE,
    conversion_id          TEXT NOT NULL,
    markdown_artifact_id   TEXT NOT NULL,
    page_from              INTEGER NOT NULL CHECK (page_from >= 1),
    page_to                INTEGER NOT NULL CHECK (page_to >= page_from),
    page_index_base        TEXT NOT NULL CHECK (page_index_base = 'one'),
    recorded_at            TEXT NOT NULL,
    PRIMARY KEY (canonical_id, conversion_id, markdown_artifact_id)
) STRICT;

CREATE INDEX canonical_markdown_coverage_by_markdown
    ON canonical_markdown_coverage(markdown_artifact_id);

PRAGMA user_version = 21;

COMMIT;
