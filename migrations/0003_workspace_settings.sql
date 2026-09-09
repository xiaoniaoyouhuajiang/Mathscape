BEGIN IMMEDIATE;

INSERT INTO schema_migrations (version, name)
VALUES (3, 'workspace_settings_v1');

CREATE TABLE workspace_settings (
    singleton_id  INTEGER PRIMARY KEY NOT NULL CHECK (singleton_id = 1),
    settings_json TEXT NOT NULL CHECK (json_valid(settings_json)),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

PRAGMA user_version = 3;

COMMIT;
