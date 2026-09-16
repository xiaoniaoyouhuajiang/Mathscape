# mathscape-artifact

Portable, content-addressed Managed Artifact Pool for a Mathscape Workspace.

The crate currently provides:

- versioned `workspace.toml` creation and validation;
- relocatable `pool:sha256/<digest>` URI resolution;
- sharded `objects/sha256/<2>/<2>/<digest>` storage;
- owned staging, streaming SHA-256, expected-digest verification and quarantine;
- atomic no-clobber promotion and content deduplication;
- SQLite catalog registration through `mathscape-store-sqlite`;
- append-only verification checks and portability audit;
- safe lookup of Managed files without following symbolic links.

The default import path always creates a Managed copy. Reference-only External
locations, hydration, garbage collection and live Workspace export remain
deferred. See [`../../docs/storage-and-integrations.md`](../../docs/storage-and-integrations.md).
