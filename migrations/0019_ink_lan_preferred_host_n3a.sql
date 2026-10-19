-- Applied inside one Rust transaction by `ink_lan::migrate_v19`; the file
-- therefore has no BEGIN/COMMIT.
INSERT INTO schema_migrations(version,name) VALUES(19,'ink_lan_preferred_host_n3a');

-- N3a: remember the address the user last picked for the QR / default URL.
-- NULL means "no preference" (desktop falls back to the first LAN address).
ALTER TABLE ink_lan_settings ADD COLUMN preferred_host TEXT;

-- N3a: why a pairing was issued. `desktop_push` is audited as that reason on
-- claim; everything else stays `tablet_pairing`.
ALTER TABLE ink_pairings ADD COLUMN claim_reason TEXT NOT NULL DEFAULT 'tablet_pairing'
    CHECK(claim_reason IN ('tablet_pairing','desktop_push'));

PRAGMA user_version=19;
