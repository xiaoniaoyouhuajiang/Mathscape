# mathscape-domain

This crate defines Mathscape's authoritative, storage-independent domain boundary.

The first slice contains:

- validated stable, revision, commit, actor, schema-version, and timestamp values;
- the common persistent `EntityHeader` and generic `EntityEnvelope<P>`;
- role-labelled binary relations and hyperedges;
- atomic `MapPatch<P>` operations with optimistic revision preconditions;
- deterministic checks for malformed IDs, endpoints, and conflicting mutations.

Payloads remain generic in this slice. A later slice will introduce a tagged
`DomainPayload` assembled from the fixture-backed source, semantic, alignment,
context, and RDTX payload types. SQLite rows and API DTOs must not bypass
`MapPatch::validate`.
