BEGIN IMMEDIATE;

INSERT INTO schema_migrations(version, name)
VALUES (25, 'study_marks_sidecar_v1');

-- User-local study progress sidecar for source semantic units and shared objects.
-- Deliberately isolated from `map_objects` so marking unread / studying / mastered
-- never creates map commits, revisions, or changes to Trust axes.
CREATE TABLE study_marks (
    object_id   TEXT PRIMARY KEY NOT NULL,
    status      TEXT NOT NULL CHECK (status IN ('unread', 'studying', 'mastered')),
    updated_at  TEXT NOT NULL,
    note        TEXT
);

CREATE INDEX study_marks_by_status
    ON study_marks(status, updated_at);

PRAGMA user_version = 25;

COMMIT;
