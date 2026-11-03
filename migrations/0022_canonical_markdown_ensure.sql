BEGIN IMMEDIATE;

INSERT INTO schema_migrations (version, name)
VALUES (22, 'canonical_markdown_ensure_v1');

-- Ensure receipts and leases for canonical markdown (material.read on a
-- scanned PDF). A `running` row is the cross-process per-PDF lock and the
-- map's 「解析中」 signal; the count of live `running` rows is the global
-- Marker slot count. Rows never touch artifact bytes. Pages are closed and
-- 1-based. Times are unix milliseconds.
CREATE TABLE canonical_markdown_ensure (
    ensure_id               TEXT PRIMARY KEY NOT NULL,
    idempotency_key         TEXT NOT NULL,
    source_pdf_artifact_id  TEXT NOT NULL,
    canonical_id            TEXT NOT NULL,
    parser_profile          TEXT NOT NULL,
    parser_policy_version   TEXT NOT NULL,
    page_from               INTEGER NOT NULL CHECK (page_from >= 1),
    page_to                 INTEGER NOT NULL CHECK (page_to >= page_from),
    status                  TEXT NOT NULL
                            CHECK (status IN ('running', 'succeeded', 'failed', 'abandoned')),
    owner                   TEXT NOT NULL,
    conversion_id           TEXT,
    markdown_artifact_id    TEXT,
    error                   TEXT,
    started_at_ms           INTEGER NOT NULL,
    lease_expires_at_ms     INTEGER NOT NULL,
    finished_at_ms          INTEGER
) STRICT;

CREATE UNIQUE INDEX canonical_markdown_ensure_one_running_per_pdf
    ON canonical_markdown_ensure(source_pdf_artifact_id)
    WHERE status = 'running';

CREATE INDEX canonical_markdown_ensure_by_key
    ON canonical_markdown_ensure(idempotency_key);

CREATE INDEX canonical_markdown_ensure_by_started
    ON canonical_markdown_ensure(started_at_ms);

PRAGMA user_version = 22;

COMMIT;
