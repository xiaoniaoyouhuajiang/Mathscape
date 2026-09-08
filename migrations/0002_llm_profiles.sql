BEGIN IMMEDIATE;

INSERT INTO schema_migrations (version, name)
VALUES (2, 'llm_profiles_v1');

CREATE TABLE llm_profiles (
    profile_id       TEXT PRIMARY KEY NOT NULL,
    profile_json     TEXT NOT NULL,
    is_default       INTEGER NOT NULL CHECK (is_default IN (0, 1)),
    updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE UNIQUE INDEX llm_profiles_one_default
    ON llm_profiles(is_default)
    WHERE is_default = 1;

PRAGMA user_version = 2;

COMMIT;
