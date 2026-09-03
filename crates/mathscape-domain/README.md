# mathscape-domain

This crate defines Mathscape's authoritative, storage-independent domain boundary.

The first slice contains:

- validated stable, revision, commit, actor, schema-version, and timestamp values;
- the common persistent `EntityHeader` and generic `EntityEnvelope<P>`;
- role-labelled binary relations and hyperedges;
- atomic `MapPatch<P>` operations with optimistic revision preconditions;
- deterministic checks for malformed IDs, endpoints, and conflicting mutations.

The source-layer slice additionally implements the typed chain:

```text
SourceWork -> SourceEdition -> SourceArtifact
                                  |
                             ConversionRun
                                  |
                              SourceSpan
                                  |
                         SourceSemanticUnit
```

`ArtifactLocation` is deliberately stored outside immutable artifact payloads.
`SourceUnitRelation` normalizes its source, target, and evidence spans as
role-labelled relation endpoints. See `docs/source-layer-v1.md` for the
canonical-versus-derived boundary.

`SourcePayload` is the first heterogeneous payload family. A later slice will
wrap it in a complete `DomainPayload` with shared semantics, alignment,
context, and RDTX payloads. SQLite rows and API DTOs must not bypass checked
constructors or `MapPatch::validate`.
